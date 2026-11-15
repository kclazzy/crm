//! Второй фактор входа: код из приложения-аутентификатора (Яндекс Ключ, Google Authenticator…)
//! и резервные коды. Когда код нужен — решает `crm_core::totp::login_rule`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use crm_core::totp::{self, Policy};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::{audit, now_ms, CurrentUser, Owner};
use crate::http::{ApiError, AppState};
use crate::secrets::token_hash;

const POLICY_KEY: &str = "two_factor";
const BACKUP_COUNT: usize = 10;
const ISSUER: &str = "CRM";

/// Политика организации; по умолчанию — код при входе из интернета.
pub async fn policy(pool: &SqlitePool) -> Result<Policy, ApiError> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(POLICY_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(raw
        .as_deref()
        .and_then(Policy::parse)
        .unwrap_or(Policy::Remote))
}

pub async fn enrolled(pool: &SqlitePool, user_id: &str) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT totp_secret IS NOT NULL FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?
            .unwrap_or(false),
    )
}

fn now_secs() -> u64 {
    u64::try_from(now_ms() / 1000).unwrap_or_default()
}

/// Резервный код как его вводят: без пробелов и дефисов, в нижнем регистре.
fn backup_key(input: &str) -> String {
    input
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Проверить код из приложения или резервный. Верный код тратится (повторно не пройдёт).
pub async fn check_code(pool: &SqlitePool, user_id: &str, input: &str) -> Result<bool, ApiError> {
    let row: Option<(Option<String>, Option<i64>)> =
        sqlx::query_as("SELECT totp_secret, totp_last_step FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let Some((Some(secret), last)) = row else {
        return Ok(false);
    };
    let secret = totp::base32_decode(&secret)
        .ok_or_else(|| ApiError::Corrupt("секрет второго фактора".into()))?;
    let last = last.and_then(|l| u64::try_from(l).ok());
    if let Some(step) = totp::verify(&secret, input, now_secs(), last) {
        let step = i64::try_from(step).unwrap_or(i64::MAX);
        // Условие в запросе — на случай двух одновременных входов с одним кодом.
        let n = sqlx::query(
            "UPDATE users SET totp_last_step = ? WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)",
        )
        .bind(step)
        .bind(user_id)
        .bind(step)
        .execute(pool)
        .await?
        .rows_affected();
        return Ok(n == 1);
    }
    let key = backup_key(input);
    if key.len() < 10 {
        return Ok(false);
    }
    let n = sqlx::query(
        "UPDATE totp_backup SET used_at = ? WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
    )
    .bind(now_ms())
    .bind(user_id)
    .bind(token_hash(&key))
    .execute(pool)
    .await?
    .rows_affected();
    Ok(n == 1)
}

// ---------- Свой код ----------

#[derive(Serialize)]
pub struct Status {
    enabled: bool,
    backup_left: i64,
    policy: &'static str,
}

pub async fn status(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Status>, ApiError> {
    let backup_left = sqlx::query_scalar(
        "SELECT count(*) FROM totp_backup WHERE user_id = ? AND used_at IS NULL",
    )
    .bind(&user.id)
    .fetch_one(&app.pool)
    .await?;
    Ok(Json(Status {
        enabled: enrolled(&app.pool, &user.id).await?,
        backup_left,
        policy: policy(&app.pool).await?.as_str(),
    }))
}

#[derive(Serialize)]
pub struct Started {
    /// Для ручного ввода в приложение.
    secret: String,
    url: String,
    qr_svg: String,
}

/// Новый секрет: показать QR-код. Включится после подтверждения кодом.
pub async fn start(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Started>, ApiError> {
    let mut secret = [0u8; totp::SECRET_LEN];
    getrandom::fill(&mut secret)
        .map_err(|e| ApiError::Failed(format!("генератор случайных чисел: {e}")))?;
    let b32 = totp::base32(&secret);
    sqlx::query("UPDATE users SET totp_pending = ? WHERE id = ?")
        .bind(&b32)
        .bind(&user.id)
        .execute(&app.pool)
        .await?;
    let url = totp::otpauth_url(ISSUER, &user.username, &secret);
    Ok(Json(Started {
        qr_svg: crate::connect::qr(&url)?,
        url,
        secret: b32,
    }))
}

#[derive(Deserialize)]
pub struct CodeIn {
    code: String,
}

#[derive(Serialize)]
pub struct Confirmed {
    /// Показываются один раз.
    backup_codes: Vec<String>,
}

fn new_backup_codes() -> Result<Vec<String>, ApiError> {
    const ABC: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut raw = [0u8; BACKUP_COUNT * 10];
    getrandom::fill(&mut raw)
        .map_err(|e| ApiError::Failed(format!("генератор случайных чисел: {e}")))?;
    Ok(raw
        .chunks(10)
        .map(|c| {
            let s: String = c
                .iter()
                .map(|b| char::from(ABC[usize::from(*b) % ABC.len()]))
                .collect();
            format!("{}-{}", &s[..5], &s[5..])
        })
        .collect())
}

/// Подтвердить кодом из приложения: код включается, выдаются резервные коды,
/// снимается ограничение «сначала настройте код» со всех входов пользователя.
pub async fn confirm(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CodeIn>,
) -> Result<Json<Confirmed>, ApiError> {
    let pending: Option<String> = sqlx::query_scalar("SELECT totp_pending FROM users WHERE id = ?")
        .bind(&user.id)
        .fetch_one(&app.pool)
        .await?;
    let pending = pending.ok_or_else(|| ApiError::BadRequest("сначала покажите QR-код".into()))?;
    let secret = totp::base32_decode(&pending)
        .ok_or_else(|| ApiError::Corrupt("секрет второго фактора".into()))?;
    let step = totp::verify(&secret, &req.code, now_secs(), None).ok_or_else(|| {
        ApiError::BadRequest("код не подошёл — проверьте время на телефоне и введите новый".into())
    })?;
    let codes = new_backup_codes()?;
    let mut tx = app.pool.begin().await?;
    sqlx::query(
        "UPDATE users SET totp_secret = totp_pending, totp_pending = NULL, totp_last_step = ? WHERE id = ?",
    )
    .bind(i64::try_from(step).unwrap_or(i64::MAX))
    .bind(&user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM totp_backup WHERE user_id = ?")
        .bind(&user.id)
        .execute(&mut *tx)
        .await?;
    for c in &codes {
        sqlx::query("INSERT INTO totp_backup(user_id, code_hash) VALUES (?, ?)")
            .bind(&user.id)
            .bind(token_hash(&backup_key(c)))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE sessions SET needs_2fa_setup = 0 WHERE user_id = ?")
        .bind(&user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(
        &app.pool,
        Some(&user.id),
        "2fa.enabled",
        Some(&user.username),
        "",
    )
    .await?;
    Ok(Json(Confirmed {
        backup_codes: codes,
    }))
}

async fn clear(pool: &SqlitePool, user_id: &str) -> Result<(), ApiError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE users SET totp_secret = NULL, totp_pending = NULL, totp_last_step = NULL WHERE id = ?",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM totp_backup WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Выключить свой код — только если организация его не требует.
pub async fn disable(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, ApiError> {
    if policy(&app.pool).await? != Policy::Off {
        return Err(ApiError::Conflict(
            "код обязателен по правилам организации — его можно только перенастроить",
        ));
    }
    clear(&app.pool, &user.id).await?;
    audit(
        &app.pool,
        Some(&user.id),
        "2fa.disabled",
        Some(&user.username),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Владелец ----------

/// Сотрудник потерял телефон и резервные коды: код сбрасывается, настроит заново в офисе.
pub async fn reset(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let username: Option<String> = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(&id)
        .fetch_optional(&app.pool)
        .await?;
    let username = username.ok_or(ApiError::NotFound)?;
    clear(&app.pool, &id).await?;
    audit(&app.pool, Some(&owner.id), "2fa.reset", Some(&username), "").await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PolicyIn {
    policy: String,
}

#[derive(Serialize)]
pub struct PolicyOut {
    policy: &'static str,
}

pub async fn get_policy(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<PolicyOut>, ApiError> {
    Ok(Json(PolicyOut {
        policy: policy(&app.pool).await?.as_str(),
    }))
}

pub async fn set_policy(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<PolicyIn>,
) -> Result<Json<PolicyOut>, ApiError> {
    let p = Policy::parse(&req.policy)
        .ok_or_else(|| ApiError::BadRequest("политика: off, remote или all".into()))?;
    if p == Policy::All && !enrolled(&app.pool, &owner.id).await? {
        return Err(ApiError::Conflict(
            "сначала включите код себе — иначе после выхода не войдёте сами",
        ));
    }
    sqlx::query(
        "INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(POLICY_KEY)
    .bind(p.as_str())
    .execute(&app.pool)
    .await?;
    audit(
        &app.pool,
        Some(&owner.id),
        "2fa.policy",
        Some(p.as_str()),
        "",
    )
    .await?;
    Ok(Json(PolicyOut { policy: p.as_str() }))
}
