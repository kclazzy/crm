//! Восстановление пароля: одноразовые коды владельца, сброс владельца на самом
//! сервере (только администратор Windows — у него есть доступ к папке базы),
//! запрос сотрудника «Забыли пароль?» владельцу.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::Json;
use chrono::Duration;
use crm_core::auth::{check_password, AccountError, Username};
use serde::{Deserialize, Serialize};

use crate::auth::{audit, ensure_not_locked, new_id, now_ms, Owner};
use crate::config::Storage;
use crate::http::{ApiError, AppState};
use crate::secrets::{hash_password, token_hash, SecretError};

const CODES: usize = 10;
/// Без похожих символов (0/O, 1/I/L) — код переписывают с бумаги.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// Не больше стольких запросов «Забыли пароль?» с одного IP за 15 минут.
const FORGOT_LIMIT: i64 = 10;

fn new_code() -> Result<String, SecretError> {
    let mut raw = [0u8; 12];
    getrandom::fill(&mut raw).map_err(SecretError::Random)?;
    let chars: String = raw
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()]))
        .collect();
    Ok(format!(
        "{}-{}-{}",
        &chars[0..4],
        &chars[4..8],
        &chars[8..12]
    ))
}

/// Регистр и дефисы не важны: «abcd efgh-jkmn» = «ABCD-EFGH-JKMN».
fn normalize(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

#[derive(Serialize)]
pub struct CodesOut {
    codes: Vec<String>,
}

/// Новые 10 кодов; прежние перестают действовать. Показываются один раз.
pub async fn new_codes(
    State(app): State<AppState>,
    Owner(owner): Owner,
) -> Result<Json<CodesOut>, ApiError> {
    let mut codes = Vec::with_capacity(CODES);
    let mut tx = app.pool.begin().await?;
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?")
        .bind(&owner.id)
        .execute(&mut *tx)
        .await?;
    for _ in 0..CODES {
        let code = new_code()?;
        sqlx::query("INSERT INTO recovery_codes(code_hash, user_id, created_at) VALUES (?, ?, ?)")
            .bind(token_hash(&normalize(&code)))
            .bind(&owner.id)
            .bind(now_ms())
            .execute(&mut *tx)
            .await?;
        codes.push(code);
    }
    tx.commit().await?;
    audit(
        &app.pool,
        Some(&owner.id),
        "recovery.codes_issued",
        Some(&owner.username),
        "",
    )
    .await?;
    Ok(Json(CodesOut { codes }))
}

#[derive(Serialize)]
pub struct CodesStatus {
    unused: i64,
}

pub async fn codes_status(
    State(app): State<AppState>,
    Owner(owner): Owner,
) -> Result<Json<CodesStatus>, ApiError> {
    let unused = sqlx::query_scalar(
        "SELECT count(*) FROM recovery_codes WHERE user_id = ? AND used_at IS NULL",
    )
    .bind(&owner.id)
    .fetch_one(&app.pool)
    .await?;
    Ok(Json(CodesStatus { unused }))
}

#[derive(Deserialize)]
pub struct RecoverIn {
    username: String,
    code: String,
    new_password: String,
}

#[derive(Serialize)]
pub struct Done {
    ok: bool,
}

/// Владелец задаёт новый пароль по одноразовому коду. Попытки ограничены, как вход.
pub async fn recover(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<RecoverIn>,
) -> Result<Json<Done>, ApiError> {
    let ip = addr.ip().to_string();
    let key = req.username.trim().to_lowercase();
    ensure_not_locked(&app.pool, "recover.failed", &key, &ip).await?;

    let owner: Option<(String, String)> = sqlx::query_as(
        "SELECT u.id, c.code_hash FROM users u JOIN recovery_codes c ON c.user_id = u.id
         WHERE u.username = ? AND u.role = 'host' AND u.is_active = 1
           AND c.code_hash = ? AND c.used_at IS NULL",
    )
    .bind(&key)
    .bind(token_hash(&normalize(&req.code)))
    .fetch_optional(&app.pool)
    .await?;
    let Some((user_id, code_hash)) = owner else {
        audit(&app.pool, None, "recover.failed", Some(&key), &ip).await?;
        return Err(ApiError::BadCode);
    };
    check_password(&req.new_password, &Username::parse(&key)?)?;
    let hash = hash_password(req.new_password).await?;

    let mut tx = app.pool.begin().await?;
    let claimed = sqlx::query(
        "UPDATE recovery_codes SET used_at = ? WHERE code_hash = ? AND used_at IS NULL",
    )
    .bind(now_ms())
    .bind(&code_hash)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed != 1 {
        return Err(ApiError::BadCode);
    }
    set_password_and_close_sessions(&mut tx, &user_id, &hash).await?;
    tx.commit().await?;
    audit(&app.pool, Some(&user_id), "recover.ok", Some(&key), &ip).await?;
    Ok(Json(Done { ok: true }))
}

async fn set_password_and_close_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ForgotIn {
    username: String,
}

/// «Забыли пароль?» сотрудника: пометка для владельца. Ответ всегда одинаковый,
/// чтобы по нему нельзя было узнать, существует ли логин.
pub async fn forgot(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<ForgotIn>,
) -> Result<Json<Done>, ApiError> {
    let ip = addr.ip().to_string();
    let key = req.username.trim().to_lowercase();
    let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'reset.requested' AND ip LIKE ? AND created_at > ?")
        .bind(crate::p2p::lockout_scope(&ip))
        .bind(now_ms() - Duration::minutes(15).num_milliseconds())
        .fetch_one(&app.pool)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = ? AND role <> 'host' AND is_active = 1)")
        .bind(&key)
        .fetch_one(&app.pool)
        .await?;
    if exists && recent < FORGOT_LIMIT {
        audit(&app.pool, None, "reset.requested", Some(&key), &ip).await?;
    }
    Ok(Json(Done { ok: true }))
}

#[derive(Debug, thiserror::Error)]
pub enum ResetError {
    #[error("{0}")]
    Weak(#[from] AccountError),
    #[error("владелец ещё не создан — откройте программу CRM и создайте его")]
    NoOwner,
    #[error(transparent)]
    Db(#[from] crate::db::DbError),
    #[error("ошибка базы данных: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
}

/// Сброс пароля (и кода из приложения) владельца прямо в базе — команда `crm-server reset-owner-password`
/// на самом сервере. Открыть базу может только администратор Windows (права на
/// папку данных), поэтому отдельная проверка не нужна. Возвращает логин владельца.
pub async fn reset_owner_password(
    storage: &Storage,
    new_password: &str,
) -> Result<String, ResetError> {
    let pool = crate::db::open(storage).await?;
    let owner: Option<(String, String)> =
        sqlx::query_as("SELECT id, username FROM users WHERE role = 'host'")
            .fetch_optional(&pool)
            .await?;
    let (id, username) = owner.ok_or(ResetError::NoOwner)?;
    check_password(new_password, &Username::parse(&username)?)?;
    let hash = hash_password(new_password.to_owned()).await?;
    let mut tx = pool.begin().await?;
    set_password_and_close_sessions(&mut tx, &id, &hash).await?;
    // Телефон с кодом тоже мог потеряться: код настраивается заново после входа.
    sqlx::query(
        "UPDATE users SET totp_secret = NULL, totp_pending = NULL, totp_last_step = NULL WHERE id = ?",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM totp_backup WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    sqlx::query("INSERT INTO audit_log(id, actor_id, action, entity, entity_id, ip, created_at) VALUES (?, ?, 'owner.reset_on_server', 'user', ?, 'local', ?)")
        .bind(new_id())
        .bind(&id)
        .bind(&username)
        .bind(now_ms())
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(username)
}
