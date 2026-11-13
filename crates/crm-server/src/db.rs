//! Подключение к SQLite (WAL) и применение миграций.

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::config::Storage;

pub const DB_FILE: &str = "crm.db";

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("не удалось создать папку {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("ошибка базы данных: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("ошибка миграции: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Открывает (или создаёт) базу и доводит схему до актуальной. Повторный вызов безопасен.
pub async fn open(storage: &Storage) -> Result<SqlitePool, DbError> {
    std::fs::create_dir_all(&storage.db_dir).map_err(|source| DbError::CreateDir {
        path: storage.db_dir.clone(),
        source,
    })?;
    let path = storage.db_dir.join(DB_FILE);

    let options = SqliteConnectOptions::from_str("sqlite://")?
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}
