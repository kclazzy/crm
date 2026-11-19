//! Уведомления: колокольчик, системные уведомления ОС, тихие часы (ТЗ §4.5).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::{new_id, now_ms, CurrentUser};
use crate::http::{ApiError, AppState};

/// Сохранить уведомление и сразу подсказать онлайн-окнам пользователя.
pub async fn push(
    app: &AppState,
    user: &str,
    task: Option<&str>,
    kind: &str,
    payload: serde_json::Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO notifications(id, user_id, task_id, kind, payload, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(user)
    .bind(task)
    .bind(kind)
    .bind(payload.to_string())
    .bind(now_ms())
    .execute(&app.pool)
    .await?;
    app.rt.hub.send(
        vec![user.to_owned()],
        serde_json::json!({ "type": "notification", "kind": kind }),
    );
    Ok(())
}

/// Активные владельцы — им идут тревоги о сервере и безопасности.
pub async fn owners(app: &AppState) -> Result<Vec<String>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT id FROM users WHERE role = 'host' AND is_active = 1")
            .fetch_all(&app.pool)
            .await?,
    )
}

/// Сколько тревог одного вида владелец получает за час — при массовом переборе
/// с многих адресов колокольчик не тонет; всё остальное есть в журнале.
const ALERTS_PER_HOUR: i64 = 20;

/// Тревога владельцам — если о том же (`kind` и то же значение `about` в payload)
/// не сообщали за последние `within_ms`: повторы одной причины не засыпают колокольчик.
pub async fn alert_owners(
    app: &AppState,
    kind: &str,
    about: &str,
    payload: serde_json::Value,
    within_ms: i64,
) -> Result<(), ApiError> {
    let value = match payload.get(about) {
        Some(serde_json::Value::String(v)) => v.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    };
    for o in owners(app).await? {
        let told: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM notifications
                             WHERE user_id = ?1 AND kind = ?2 AND created_at > ?3
                               AND json_extract(payload, '$.' || ?4) = ?5)
                 OR (SELECT count(*) FROM notifications
                      WHERE user_id = ?1 AND kind = ?2 AND created_at > ?6) >= ?7",
        )
        .bind(&o)
        .bind(kind)
        .bind(now_ms() - within_ms)
        .bind(about)
        .bind(&value)
        .bind(now_ms() - 3_600_000)
        .bind(ALERTS_PER_HOUR)
        .fetch_one(&app.pool)
        .await?;
        if !told {
            push(app, &o, None, kind, payload.clone()).await?;
        }
    }
    Ok(())
}

/// Было ли уже такое уведомление пользователю за последние `within_ms` — чтобы не спамить.
pub async fn recently(
    pool: &SqlitePool,
    user: &str,
    kind: &str,
    within_ms: i64,
) -> Result<bool, ApiError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = ? AND kind = ? AND created_at > ?",
    )
    .bind(user)
    .bind(kind)
    .bind(now_ms() - within_ms)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    task_id: Option<String>,
    kind: String,
    payload: String,
    created_at: i64,
    read_at: Option<i64>,
}

#[derive(Serialize)]
pub struct Notification {
    id: String,
    task_id: Option<String>,
    kind: String,
    payload: serde_json::Value,
    created_at: i64,
    read_at: Option<i64>,
}

#[derive(Serialize)]
pub struct Bell {
    unread: i64,
    items: Vec<Notification>,
    /// Сейчас тихие часы для этого пользователя: без звука и уведомлений ОС.
    quiet: bool,
}

pub async fn list(State(app): State<AppState>, user: CurrentUser) -> Result<Json<Bell>, ApiError> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, task_id, kind, payload, created_at, read_at FROM notifications
         WHERE user_id = ? AND hidden_at IS NULL ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&user.id)
    .fetch_all(&app.pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|r| Notification {
            payload: serde_json::from_str(&r.payload).unwrap_or(serde_json::Value::Null),
            id: r.id,
            task_id: r.task_id,
            kind: r.kind,
            created_at: r.created_at,
            read_at: r.read_at,
        })
        .collect();
    let unread = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = ? AND read_at IS NULL AND hidden_at IS NULL",
    )
    .bind(&user.id)
    .fetch_one(&app.pool)
    .await?;
    Ok(Json(Bell {
        unread,
        items,
        quiet: is_quiet(&app.pool, &user.id).await?,
    }))
}

pub async fn is_quiet(pool: &SqlitePool, user: &str) -> Result<bool, ApiError> {
    let always: bool = sqlx::query_scalar("SELECT notify_always FROM users WHERE id = ?")
        .bind(user)
        .fetch_one(pool)
        .await?;
    if always {
        return Ok(false);
    }
    let org = crate::org::load(pool).await?;
    Ok(!org.schedule().is_working(chrono::Utc::now()))
}

pub async fn read_all(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, ApiError> {
    sqlx::query("UPDATE notifications SET read_at = ? WHERE user_id = ? AND read_at IS NULL")
        .bind(now_ms())
        .bind(&user.id)
        .execute(&app.pool)
        .await?;
    app.rt
        .hub
        .send(vec![user.id], serde_json::json!({ "type": "notification" }));
    Ok(StatusCode::NO_CONTENT)
}

/// Удалить своё уведомление. Запись скрывается (и считается прочитанной), а не
/// стирается: по ней та же тревога не приходит снова сразу же.
pub async fn hide(
    State(app): State<AppState>,
    user: CurrentUser,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<StatusCode, ApiError> {
    let now = now_ms();
    let done = sqlx::query(
        "UPDATE notifications SET hidden_at = ?, read_at = coalesce(read_at, ?)
         WHERE id = ? AND user_id = ? AND hidden_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(&id)
    .bind(&user.id)
    .execute(&app.pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    app.rt
        .hub
        .send(vec![user.id], serde_json::json!({ "type": "notification" }));
    Ok(StatusCode::NO_CONTENT)
}

/// Удалить все свои уведомления.
pub async fn hide_all(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, ApiError> {
    let now = now_ms();
    sqlx::query(
        "UPDATE notifications SET hidden_at = ?, read_at = coalesce(read_at, ?)
         WHERE user_id = ? AND hidden_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(&user.id)
    .execute(&app.pool)
    .await?;
    app.rt
        .hub
        .send(vec![user.id], serde_json::json!({ "type": "notification" }));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ProblemIn {
    text: String,
    #[serde(default)]
    page: String,
}

/// «Сообщить о проблеме»: пара слов от сотрудника — владельцу в колокольчик, с именем,
/// страницей и версией. Повтор того же текста за 10 минут не дублируется; общий
/// предел тревог одного вида в час — как у остальных.
pub async fn problem(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(req): Json<ProblemIn>,
) -> Result<StatusCode, ApiError> {
    let text: String = req.text.trim().chars().take(1000).collect();
    if text.is_empty() {
        return Err(ApiError::BadRequest(
            "опишите проблему хотя бы парой слов".into(),
        ));
    }
    let page: String = req.page.chars().take(200).collect();
    alert_owners(
        &app,
        "problem_report",
        "text",
        serde_json::json!({
            "from": user.display_name,
            "username": user.username,
            "text": text,
            "page": page,
            "version": env!("CARGO_PKG_VERSION"),
        }),
        10 * 60 * 1000,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, Deserialize)]
pub struct Prefs {
    /// «Получать уведомления всегда» — игнорировать тихие часы.
    notify_always: bool,
}

pub async fn get_prefs(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Prefs>, ApiError> {
    let notify_always = sqlx::query_scalar("SELECT notify_always FROM users WHERE id = ?")
        .bind(&user.id)
        .fetch_one(&app.pool)
        .await?;
    Ok(Json(Prefs { notify_always }))
}

pub async fn put_prefs(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(p): Json<Prefs>,
) -> Result<Json<Prefs>, ApiError> {
    sqlx::query("UPDATE users SET notify_always = ? WHERE id = ?")
        .bind(p.notify_always)
        .bind(&user.id)
        .execute(&app.pool)
        .await?;
    Ok(Json(p))
}
