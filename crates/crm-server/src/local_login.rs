//! Вход владельца на самом сервере без пароля — одноразовый билет на 2 минуты.
//!
//! Выдают билет только те, кто и так может всё: создание владельца (пароль только
//! что введён) и администратор Windows (`crm-server owner-ticket`, у него есть
//! доступ к папке базы — он может и сбросить пароль). Обменять билет на сессию
//! можно только с этого же компьютера (127.0.0.1), поэтому перехваченный билет
//! по сети бесполезен.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::auth::now_ms;
use crate::config::Storage;
use crate::http::{ApiError, AppState};
use crate::recovery::ResetError;
use crate::secrets::{new_token, token_hash};

/// Сколько живёт билет: от нажатия кнопки до открытия окна.
const TTL_MS: i64 = 2 * 60 * 1000;

async fn store(pool: &SqlitePool, hash: &str, user_id: &str) -> Result<(), sqlx::Error> {
    let now = now_ms();
    sqlx::query("DELETE FROM local_tickets WHERE expires_at <= ?")
        .bind(now)
        .execute(pool)
        .await?;
    sqlx::query("INSERT OR REPLACE INTO local_tickets(hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(hash)
        .bind(user_id)
        .bind(now + TTL_MS)
        .execute(pool)
        .await?;
    Ok(())
}

/// Билет для пользователя — сразу после создания владельца.
pub(crate) async fn issue(pool: &SqlitePool, user_id: &str) -> Result<String, ApiError> {
    let ticket = new_token()?;
    store(pool, &token_hash(&ticket), user_id).await?;
    Ok(ticket)
}

/// `crm-server owner-ticket <хеш>`: билет владельцу прямо в базе. Открыть базу
/// может только администратор Windows. Возвращает логин владельца.
pub async fn issue_offline(storage: &Storage, hash: &str) -> Result<String, ResetError> {
    let pool = crate::db::open(storage).await?;
    let owner: Option<(String, String)> = sqlx::query_as(
        "SELECT id, username FROM users WHERE role = 'host' AND is_active = 1
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    let (id, username) = owner.ok_or(ResetError::NoOwner)?;
    store(&pool, hash, &id).await?;
    pool.close().await;
    Ok(username)
}

#[derive(Deserialize)]
pub struct TicketIn {
    t: String,
}

/// Обмен билета на сессию и переход в CRM. Неверный, просроченный или чужой
/// (не с этого компьютера) билет ведёт на ту же страницу — к обычному входу.
pub async fn enter(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<TicketIn>,
) -> Result<impl IntoResponse, ApiError> {
    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if !addr.ip().is_loopback() {
        return Ok((StatusCode::SEE_OTHER, headers));
    }
    let user: Option<String> = sqlx::query_scalar(
        "DELETE FROM local_tickets WHERE hash = ? AND expires_at > ? RETURNING user_id",
    )
    .bind(token_hash(&q.t))
    .bind(now_ms())
    .fetch_optional(&app.pool)
    .await?;
    if let Some(user_id) = user {
        let cookie = crate::auth::session_without_password(&app.pool, &user_id, &addr).await?;
        headers.insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).map_err(|e| ApiError::Failed(e.to_string()))?,
        );
    }
    Ok((StatusCode::SEE_OTHER, headers))
}
