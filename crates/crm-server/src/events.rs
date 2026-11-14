//! События в реальном времени (WebSocket): каждому пользователю — только его.
//!
//! Событие — подсказка «обновите это»; данные клиент берёт через API, где проверяются права.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::auth::CurrentUser;
use crate::http::{ApiError, AppState};

#[derive(Debug)]
struct Outgoing {
    to: Vec<String>,
    text: String,
}

pub struct Hub {
    tx: broadcast::Sender<Arc<Outgoing>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(256).0,
        }
    }
}

impl Hub {
    /// Отправить событие перечисленным пользователям (на все их устройства).
    pub fn send(&self, to: Vec<String>, event: Value) {
        if to.is_empty() {
            return;
        }
        // Нет подписчиков — не ошибка: никто не онлайн.
        let _ = self.tx.send(Arc::new(Outgoing {
            to,
            text: event.to_string(),
        }));
    }
}

/// Что-то изменилось в задаче: сообщаем всем, кто её видит.
pub async fn task_changed(app: &AppState, task_id: &str, what: &str) -> Result<(), ApiError> {
    let to = crate::access::audience(&app.pool, task_id).await?;
    app.rt.hub.send(
        to,
        serde_json::json!({ "type": "task", "task_id": task_id, "what": what }),
    );
    Ok(())
}

/// Изменились счётчики пользователя (прочитал на другом устройстве).
pub fn unread_changed(app: &AppState, user_id: &str) {
    app.rt.hub.send(
        vec![user_id.to_owned()],
        serde_json::json!({ "type": "unread" }),
    );
}

/// Как часто сверять, что сессия ещё действует.
const RECHECK: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn ws(ws: WebSocketUpgrade, State(app): State<AppState>, user: CurrentUser) -> Response {
    ws.on_upgrade(move |socket| serve(socket, app, user))
}

async fn serve(mut socket: WebSocket, app: AppState, user: CurrentUser) {
    let mut rx = app.rt.hub.tx.subscribe();
    let mut recheck = tokio::time::interval(RECHECK);
    recheck.tick().await;
    if socket
        .send(Message::Text(r#"{"type":"hello"}"#.into()))
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    if ev.to.contains(&user.id)
                        && socket.send(Message::Text(ev.text.clone().into())).await.is_err()
                    {
                        return;
                    }
                }
                // Отстали: просим клиента перечитать всё.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if socket.send(Message::Text(r#"{"type":"resync"}"#.into())).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
            _ = recheck.tick() => {
                if !session_alive(&app, &user.session_id).await {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
        }
    }
}

async fn session_alive(app: &AppState, session_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.id = ? AND s.revoked_at IS NULL AND s.expires_at > ? AND u.is_active = 1",
    )
    .bind(session_id)
    .bind(crate::auth::now_ms())
    .fetch_one(&app.pool)
    .await
    .is_ok_and(|n| n > 0)
}
