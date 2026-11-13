//! Секреты: случайные токены, их хеши, хеширование паролей argon2id.

use std::sync::OnceLock;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("нет источника случайных чисел: {0}")]
    Random(getrandom::Error),
    #[error("ошибка хеширования пароля: {0}")]
    Hash(argon2::password_hash::Error),
    #[error("фоновая задача прервана: {0}")]
    Join(#[from] tokio::task::JoinError),
}

fn random_bytes<const N: usize>() -> Result<[u8; N], SecretError> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(SecretError::Random)?;
    Ok(buf)
}

/// Новый токен (256 бит) для сессии или приглашения. Клиенту отдаётся он сам,
/// в БД пишется только [`token_hash`].
pub fn new_token() -> Result<String, SecretError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

pub fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn hash_blocking(password: &str) -> Result<String, SecretError> {
    let salt = SaltString::encode_b64(&random_bytes::<16>()?).map_err(SecretError::Hash)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(SecretError::Hash)
}

/// argon2id — медленный по замыслу, поэтому вне потока обработки запросов.
pub async fn hash_password(password: String) -> Result<String, SecretError> {
    tokio::task::spawn_blocking(move || hash_blocking(&password)).await?
}

/// Хеш-заглушка: если логина нет, проверяем пароль против неё, чтобы ответ
/// занимал столько же времени, сколько при неверном пароле.
fn dummy_hash() -> Result<&'static str, SecretError> {
    static DUMMY: OnceLock<String> = OnceLock::new();
    if let Some(h) = DUMMY.get() {
        return Ok(h);
    }
    let h = hash_blocking(&new_token()?)?;
    Ok(DUMMY.get_or_init(|| h))
}

/// `stored = None` — пользователя нет или пароль не задан: ответ `false`,
/// но время проверки то же, что и для настоящего пароля.
pub async fn verify_password(
    password: String,
    stored: Option<String>,
) -> Result<bool, SecretError> {
    tokio::task::spawn_blocking(move || {
        // Пароль заглушки — случайный и нигде не сохранён, поэтому с ней проверка всегда ложна.
        let hash = match &stored {
            Some(h) => h.as_str(),
            None => dummy_hash()?,
        };
        let parsed = PasswordHash::new(hash).map_err(SecretError::Hash)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await?
}
