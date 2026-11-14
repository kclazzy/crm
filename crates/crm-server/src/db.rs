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
    #[error("восстановление из копии: {0}")]
    Restore(Box<crate::restore::RestoreError>),
}

/// Открывает (или создаёт) базу и доводит схему до актуальной. Повторный вызов безопасен.
pub async fn open(storage: &Storage) -> Result<SqlitePool, DbError> {
    std::fs::create_dir_all(&storage.db_dir).map_err(|source| DbError::CreateDir {
        path: storage.db_dir.clone(),
        source,
    })?;
    make_private(&storage.db_dir)?;
    crate::restore::apply_staged(storage).map_err(|e| DbError::Restore(Box::new(e)))?;
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

/// Папка с БД — только для учётной записи сервера (в БД хеши паролей).
/// На Windows доступ ограничивает установщик через ACL папки ProgramData\CRM.
pub(crate) fn make_private(dir: &std::path::Path) -> Result<(), DbError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| DbError::CreateDir {
                path: dir.to_owned(),
                source,
            },
        )?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
