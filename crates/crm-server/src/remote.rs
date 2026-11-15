//! Доступ через интернет (ТЗ §9, вместо VPN): внешний адрес офиса — белый IP или имя DDNS.
//!
//! Адрес попадает в ключ подключения, в сертификат сервера (чтобы браузер телефона
//! ему доверял) и в ответ `/api/health` — программы сотрудников запоминают его сами.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::Owner;
use crate::http::{ApiError, AppState};

const KEY: &str = "remote_address";

/// Внешний адрес, если доступ через интернет включён.
pub async fn address(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(KEY)
        .fetch_optional(pool)
        .await
}

#[derive(Serialize)]
pub struct Info {
    address: Option<String>,
    port: u16,
    /// Без HTTPS снаружи открывать нельзя.
    https: bool,
    /// IP этого компьютера в сети офиса — на него роутер перенаправляет порт.
    office_ip: Option<String>,
}

async fn info_of(app: &AppState) -> Result<Info, ApiError> {
    let office_ip = crate::connect::server_addrs(app)
        .await?
        .into_iter()
        .find(|a| a.parse::<std::net::Ipv4Addr>().is_ok() && crm_core::remote::is_office_addr(a));
    Ok(Info {
        office_ip,
        address: address(&app.pool).await?,
        port: app.rt.config().listen.port(),
        https: app.ca_fingerprint.is_some(),
    })
}

pub async fn info(State(app): State<AppState>, Owner(_): Owner) -> Result<Json<Info>, ApiError> {
    Ok(Json(info_of(&app).await?))
}

#[derive(Deserialize)]
pub struct AddressIn {
    address: String,
}

pub async fn set(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<AddressIn>,
) -> Result<Json<Info>, ApiError> {
    let port = app.rt.config().listen.port();
    match crm_core::remote::parse_address(&req.address, port)
        .map_err(|e| ApiError::BadRequest(e.into()))?
    {
        Some(a) => {
            sqlx::query(
                "INSERT INTO settings(key, value) VALUES (?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(KEY)
            .bind(&a)
            .execute(&app.pool)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM settings WHERE key = ?")
                .bind(KEY)
                .execute(&app.pool)
                .await?;
        }
    }
    // Сертификат с новым адресом — сразу, без перезапуска.
    app.rt.tls_refresh.notify_one();
    Ok(Json(info_of(&app).await?))
}
