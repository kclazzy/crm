//! Резервное копирование (ТЗ §5): горячий снимок БД через VACUUM INTO, проверка
//! целостности, сжатие zstd, копия в каждую папку бэкапов, ротация 48 ч / 30 дней.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqlitePool};

use crate::config::Config;
use crate::http::AppState;

/// Первый бэкап — через 2 минуты после старта, дальше раз в час.
const FIRST_DELAY: Duration = Duration::from_secs(120);
const EVERY: Duration = Duration::from_secs(3600);
const STATUS_KEY: &str = "backup.last";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub at: i64,
    pub ok: bool,
    pub file: Option<String>,
    pub size: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupFile {
    pub name: String,
    pub at: i64,
    pub size: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("не выбрана папка для резервных копий")]
    NoDirs,
    #[error("ошибка базы данных: {0}")]
    Db(#[from] sqlx::Error),
    #[error("копия базы повреждена: {0}")]
    Corrupt(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("фоновая задача прервана: {0}")]
    Join(#[from] tokio::task::JoinError),
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> BackupError + '_ {
    move |source| BackupError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Сделать бэкап сейчас. Возвращает имя файла и размер сжатой копии.
pub async fn run(app: &AppState) -> Result<BackupFile, BackupError> {
    let _one_at_a_time = app.rt.backup_lock.lock().await;
    let cfg = app.rt.config();
    let result = run_locked(&app.pool, &cfg).await;
    let status = match &result {
        Ok(f) => Status {
            at: Utc::now().timestamp_millis(),
            ok: true,
            file: Some(f.name.clone()),
            size: Some(f.size),
            error: None,
        },
        Err(e) => Status {
            at: Utc::now().timestamp_millis(),
            ok: false,
            file: None,
            size: None,
            error: Some(e.to_string()),
        },
    };
    if let Ok(json) = serde_json::to_string(&status) {
        let _ = sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(STATUS_KEY)
            .bind(json)
            .execute(&app.pool)
            .await;
    }
    result
}

async fn run_locked(pool: &SqlitePool, cfg: &Config) -> Result<BackupFile, BackupError> {
    let dirs = cfg.storage.backup_dirs.clone();
    if dirs.is_empty() {
        return Err(BackupError::NoDirs);
    }
    let snapshot = cfg.storage.db_dir.join("backup-snapshot.tmp.db");
    if snapshot.exists() {
        std::fs::remove_file(&snapshot).map_err(io(&snapshot))?;
    }
    // Горячий согласованный снимок: работа сотрудников не останавливается.
    sqlx::query("VACUUM INTO ?")
        .bind(snapshot.to_string_lossy().into_owned())
        .execute(pool)
        .await?;
    check_integrity(&snapshot).await?;

    let now = Utc::now();
    let name = crm_core::backup::file_name(now);
    let tls = cfg.storage.db_dir.join("tls");
    let size = tokio::task::spawn_blocking({
        let (snapshot, name) = (snapshot.clone(), name.clone());
        move || write_copies(&snapshot, &name, &dirs, &tls, now)
    })
    .await??;
    let _ = std::fs::remove_file(&snapshot);
    tracing::info!(%name, size, "резервная копия создана");
    Ok(BackupFile {
        name,
        at: now.timestamp_millis(),
        size,
    })
}

/// Подключение к копии только на чтение: `PRAGMA integrity_check` должен ответить «ok».
pub async fn check_integrity(db: &Path) -> Result<(), BackupError> {
    let mut conn = SqliteConnectOptions::new()
        .filename(db)
        .read_only(true)
        .connect()
        .await?;
    let verdict: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut conn)
        .await?;
    drop(conn);
    if verdict == "ok" {
        Ok(())
    } else {
        Err(BackupError::Corrupt(verdict))
    }
}

fn write_copies(
    snapshot: &Path,
    name: &str,
    dirs: &[PathBuf],
    tls: &Path,
    now: DateTime<Utc>,
) -> Result<u64, BackupError> {
    let raw = std::fs::read(snapshot).map_err(io(snapshot))?;
    let packed = zstd::encode_all(raw.as_slice(), 10).map_err(io(snapshot))?;
    for dir in dirs {
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        crate::db::make_private(dir).map_err(|e| BackupError::Corrupt(e.to_string()))?;
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, &packed).map_err(io(&tmp))?;
        std::fs::rename(&tmp, dir.join(name)).map_err(io(dir))?;
        copy_tls(tls, &dir.join("tls"))?;
        rotate(dir, now)?;
    }
    Ok(packed.len() as u64)
}

/// Ключи и сертификат сервера — чтобы восстановленный сервер остался «тем же» для сотрудников.
fn copy_tls(from: &Path, to: &Path) -> Result<(), BackupError> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(()); // HTTPS выключен — копировать нечего
    };
    std::fs::create_dir_all(to).map_err(io(to))?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "crt" || x == "key") {
            std::fs::copy(&p, to.join(e.file_name())).map_err(io(&p))?;
        }
    }
    Ok(())
}

/// Ротация: всё за 48 ч, дальше по одной в сутки за 30 дней. Чужие файлы не трогаем.
fn rotate(dir: &Path, now: DateTime<Utc>) -> Result<(), BackupError> {
    let files = list_dir(dir)?;
    let times: Vec<DateTime<Utc>> = files
        .iter()
        .filter_map(|f| DateTime::from_timestamp_millis(f.at))
        .collect();
    for t in crm_core::backup::to_delete(&times, now, chrono_tz::UTC) {
        let p = dir.join(crm_core::backup::file_name(t));
        std::fs::remove_file(&p).map_err(io(&p))?;
    }
    Ok(())
}

/// Копии в папке, новые сверху.
pub fn list_dir(dir: &Path) -> Result<Vec<BackupFile>, BackupError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(dir)(e)),
    };
    let mut out: Vec<BackupFile> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let at = crm_core::backup::parse_file_name(&name)?;
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            Some(BackupFile {
                name,
                at: at.timestamp_millis(),
                size,
            })
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.at));
    Ok(out)
}

pub async fn last_status(pool: &SqlitePool) -> Option<Status> {
    let json: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(STATUS_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    json.and_then(|j| serde_json::from_str(&j).ok())
}

/// Фоновый воркер: раз в час, если папка бэкапов выбрана.
pub async fn hourly(app: AppState) {
    tokio::time::sleep(FIRST_DELAY).await;
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if app.rt.config().storage.backup_dirs.is_empty() {
            continue;
        }
        if let Err(e) = run(&app).await {
            tracing::error!(error = %e, "резервное копирование не удалось");
        }
    }
}
