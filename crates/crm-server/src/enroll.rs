//! Подключение программы в офисе по короткому коду (протокол — `crm_core::enroll`).
//!
//! Владелец выдаёт код сотруднику; программа выполняет обмен SPAKE2 и получает
//! ключ подключения — тот же, что «Ключ подключения», только без длинной строки.

use axum::extract::{Path, State};
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use crm_core::enroll::{server_answer, Code};
use serde::{Deserialize, Serialize};

use crate::auth::{now_ms, Owner, INVITE_TTL};
use crate::http::{ApiError, AppState};

/// Сколько попыток даёт один код: верный ввод — одна-две, перебор — не пройдёт.
const MAX_ATTEMPTS: i64 = 10;

#[derive(Serialize)]
pub struct CodeOut {
    code: String,
    expires_at: i64,
}

/// Новый код для сотрудника (прежний его код перестаёт действовать).
pub async fn new_code(
    State(app): State<AppState>,
    Owner(_): Owner,
    Path(user_id): Path<String>,
) -> Result<Json<CodeOut>, ApiError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = ? AND is_active = 1)")
            .bind(&user_id)
            .fetch_one(&app.pool)
            .await?;
    if !exists {
        return Err(ApiError::NotFound);
    }
    let expires_at = now_ms() + INVITE_TTL.num_milliseconds();
    let mut tx = app.pool.begin().await?;
    sqlx::query("DELETE FROM enroll_codes WHERE user_id = ? OR expires_at < ?")
        .bind(&user_id)
        .bind(now_ms())
        .execute(&mut *tx)
        .await?;
    // Номер кода — 4 символа: при редком совпадении берём другой.
    let code = loop {
        let mut random = [0u8; 5];
        getrandom::fill(&mut random)
            .map_err(|e| ApiError::Failed(format!("генератор случайных чисел: {e}")))?;
        let code = Code::from_random(random);
        let shown = code.to_string();
        let added = sqlx::query(
            "INSERT OR IGNORE INTO enroll_codes(id, secret, user_id, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(code.id())
        .bind(&shown[5..])
        .bind(&user_id)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if added == 1 {
            break shown;
        }
    };
    tx.commit().await?;
    Ok(Json(CodeOut { code, expires_at }))
}

#[derive(Deserialize)]
pub struct EnrollIn {
    /// Открытая часть кода.
    id: String,
    /// Первое сообщение SPAKE2 программы (base64).
    msg: String,
}

#[derive(Serialize)]
pub struct EnrollOut {
    msg: String,
    nonce: String,
    sealed: String,
}

/// Ответ программе. Верен ли её код, сервер не знает — каждая попытка считается.
pub async fn enroll(
    State(app): State<AppState>,
    Json(req): Json<EnrollIn>,
) -> Result<Json<EnrollOut>, ApiError> {
    let fp = app.ca_fingerprint.clone().ok_or_else(|| {
        ApiError::BadRequest("подключение по коду работает только с HTTPS".into())
    })?;
    let id = req.id.trim().to_ascii_uppercase();
    // Попытка засчитывается до ответа: так перебор упирается в предел.
    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE enroll_codes SET attempts = attempts + 1
         WHERE id = ? AND expires_at > ? AND attempts < ?
           AND user_id IN (SELECT id FROM users WHERE is_active = 1)
         RETURNING secret, user_id",
    )
    .bind(&id)
    .bind(now_ms())
    .bind(MAX_ATTEMPTS)
    .fetch_optional(&app.pool)
    .await?;
    let (secret, user_id) = row.ok_or(ApiError::NotFound)?;
    let code = Code::parse(&format!("{id}{secret}")).map_err(|_| ApiError::NotFound)?;
    let msg = B64
        .decode(req.msg.trim())
        .map_err(|_| ApiError::BadRequest("сообщение программы повреждено".into()))?;

    let built = crate::connect::build_key(&app, Some(&user_id)).await?;
    let key = crate::connect::key_string(&built.key, None)?;
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce)
        .map_err(|e| ApiError::Failed(format!("генератор случайных чисел: {e}")))?;
    let answer = server_answer(&code, &fp, &msg, &key, nonce)
        .map_err(|_| ApiError::BadRequest("сообщение программы повреждено".into()))?;
    Ok(Json(EnrollOut {
        msg: B64.encode(answer.msg),
        nonce: B64.encode(answer.nonce),
        sealed: B64.encode(answer.sealed),
    }))
}
