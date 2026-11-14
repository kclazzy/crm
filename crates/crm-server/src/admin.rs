//! Админ-панель владельца: адреса сервера, хранилище, резервное копирование (ТЗ §2.4, §2.7, §5).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::Json;
use crm_core::disk::{assess, DiskLevel, Thresholds};
use serde::{Deserialize, Serialize};

use crate::auth::Owner;
use crate::backup::{self, BackupError, BackupFile, Status};
use crate::http::{ApiError, AppState};

// ---------- Адреса ----------

#[derive(Serialize)]
pub struct ServerInfo {
    hostname: String,
    version: &'static str,
    /// Адреса для сотрудников: сначала по имени компьютера (не меняется), потом IP.
    addresses: Vec<String>,
    ca_fingerprint: Option<std::sync::Arc<str>>,
}

pub async fn server_info(State(app): State<AppState>, Owner(_): Owner) -> Json<ServerInfo> {
    let cfg = app.rt.config();
    let port = cfg.listen.port();
    let scheme = if cfg.tls.enabled { "https" } else { "http" };
    let sans = crate::tls::local_sans();
    let names = sans.names.iter().filter(|n| *n != "localhost");
    let ips = sans
        .ips
        .iter()
        .filter(|ip| ip.is_ipv4() && !ip.is_loopback());
    let addresses = names
        .map(|n| format!("{scheme}://{n}:{port}"))
        .chain(ips.map(|ip| format!("{scheme}://{ip}:{port}")))
        .collect();
    Json(ServerInfo {
        hostname: gethostname::gethostname().to_string_lossy().into_owned(),
        version: env!("CARGO_PKG_VERSION"),
        addresses,
        ca_fingerprint: app.ca_fingerprint.clone(),
    })
}

// ---------- Хранилище ----------

#[derive(Serialize)]
pub struct Volume {
    /// Корень тома (диск C:\ или точка монтирования) — для подписи.
    path: String,
    /// Что лежит на томе: «Данные», «Резервные копии».
    roles: Vec<&'static str>,
    total_bytes: u64,
    free_bytes: u64,
    crm_bytes: u64,
    level: DiskLevel,
}

#[derive(Serialize)]
pub struct StorageInfo {
    data_dir: PathBuf,
    backup_dirs: Vec<PathBuf>,
    volumes: Vec<Volume>,
    /// Данные перенесены, ждём перезапуска.
    restart_required: bool,
    policy: crm_core::backup::Policy,
    /// Сколько копий будет лежать в папке при этих настройках.
    expected_copies: u32,
}

/// Ближайшая существующая папка (у ещё не созданной папки бэкапов — её родитель).
fn existing(p: &Path) -> PathBuf {
    p.ancestors().find(|a| a.exists()).unwrap_or(p).to_owned()
}

/// Идентификатор тома: на Unix — номер устройства, на Windows — буква диска.
fn volume_id(p: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(existing(p))
            .map(|m| m.dev().to_string())
            .unwrap_or_default()
    }
    #[cfg(not(unix))]
    {
        p.components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|it| {
            it.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

pub async fn storage(State(app): State<AppState>, Owner(_): Owner) -> Json<StorageInfo> {
    let cfg = app.rt.config();
    let mut places: Vec<(&'static str, PathBuf, u64)> = vec![(
        "Данные",
        cfg.storage.db_dir.clone(),
        dir_size(&cfg.storage.db_dir),
    )];
    for d in &cfg.storage.backup_dirs {
        let size = backup::list_dir(d)
            .map(|v| v.iter().map(|f| f.size).sum())
            .unwrap_or(0);
        places.push(("Резервные копии", d.clone(), size));
    }
    let mut volumes: Vec<(String, Volume)> = Vec::new();
    for (role, path, size) in places {
        let id = volume_id(&path);
        if let Some((_, v)) = volumes.iter_mut().find(|(vid, _)| *vid == id) {
            if !v.roles.contains(&role) {
                v.roles.push(role);
            }
            v.crm_bytes += size;
            continue;
        }
        let at = existing(&path);
        let (total, free) = fs4::statvfs(&at)
            .map(|s| (s.total_space(), s.available_space()))
            .unwrap_or((0, 0));
        let level = assess(total, free, &[], chrono::Utc::now(), Thresholds::default()).level;
        let root = at
            .ancestors()
            .last()
            .map(|r| r.display().to_string())
            .unwrap_or_default();
        volumes.push((
            id,
            Volume {
                path: root,
                roles: vec![role],
                total_bytes: total,
                free_bytes: free,
                crm_bytes: size,
                level,
            },
        ));
    }
    Json(StorageInfo {
        data_dir: cfg.storage.db_dir,
        backup_dirs: cfg.storage.backup_dirs,
        volumes: volumes.into_iter().map(|(_, v)| v).collect(),
        restart_required: app.rt.frozen.load(Ordering::SeqCst),
        policy: cfg.backup,
        expected_copies: cfg.backup.expected_copies(),
    })
}

/// Путь от владельца: абсолютный и не сетевая папка Windows (\\server\share).
fn parse_dir(raw: &str) -> Result<PathBuf, ApiError> {
    let raw = raw.trim();
    let p = PathBuf::from(raw);
    if raw.is_empty() || !p.is_absolute() {
        return Err(ApiError::BadRequest(format!(
            "нужен полный путь к папке, например D:\\CRM-backup (получено «{raw}»)"
        )));
    }
    Ok(p)
}

fn probe_writable(dir: &Path) -> Result<(), ApiError> {
    let fail = |e: std::io::Error| {
        ApiError::BadRequest(format!("нет доступа к папке {}: {e}", dir.display()))
    };
    std::fs::create_dir_all(dir).map_err(fail)?;
    let probe = dir.join(".crm-write-test");
    std::fs::write(&probe, b"ok").map_err(fail)?;
    std::fs::remove_file(&probe).map_err(fail)
}

fn save(app: &AppState, change: impl FnOnce(&mut crate::config::Config)) -> Result<(), ApiError> {
    let mut cfg = app
        .rt
        .config
        .write()
        .map_err(|_| ApiError::Failed("настройки недоступны".into()))?;
    change(&mut cfg);
    if let Some(path) = &app.rt.config_path {
        cfg.save(path)
            .map_err(|e| ApiError::Failed(e.to_string()))?;
    }
    Ok(())
}

// ---------- Резервное копирование ----------

#[derive(Deserialize)]
pub struct BackupDirsIn {
    dirs: Vec<String>,
    /// Расписание и сроки; не передано — остаются прежние.
    policy: Option<crm_core::backup::RawPolicy>,
}

#[derive(Serialize)]
pub struct BackupDirsOut {
    dirs: Vec<PathBuf>,
    /// Предупреждения: например, копии на том же диске, что и данные.
    warnings: Vec<String>,
}

pub async fn set_backup_dirs(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<BackupDirsIn>,
) -> Result<Json<BackupDirsOut>, ApiError> {
    let policy = match req.policy {
        Some(raw) => crm_core::backup::Policy::try_from(raw)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        None => app.rt.config().backup,
    };
    let data = app.rt.config().storage.db_dir;
    let mut dirs = Vec::new();
    let mut warnings = Vec::new();
    for raw in req.dirs.iter().take(3) {
        let dir = parse_dir(raw)?;
        if dir.starts_with(&data) {
            return Err(ApiError::BadRequest(
                "папка копий не может быть внутри папки данных".into(),
            ));
        }
        probe_writable(&dir)?;
        if volume_id(&dir) == volume_id(&data) {
            warnings.push(format!("{} — на том же диске, что и данные: поломка диска уничтожит и данные, и копии. Добавьте второй диск, флешку или NAS.", dir.display()));
        }
        dirs.push(dir);
    }
    save(&app, |c| {
        c.storage.backup_dirs = dirs.clone();
        c.backup = policy;
    })?;
    Ok(Json(BackupDirsOut { dirs, warnings }))
}

pub async fn backup_now(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<BackupFile>, ApiError> {
    backup::run(&app).await.map(Json).map_err(|e| match e {
        BackupError::NoDirs => ApiError::BadRequest(e.to_string()),
        other => ApiError::Failed(format!("резервная копия не создана: {other}")),
    })
}

#[derive(Serialize)]
pub struct BackupsOut {
    last: Option<Status>,
    files: Vec<BackupFile>,
}

pub async fn backups(State(app): State<AppState>, Owner(_): Owner) -> Json<BackupsOut> {
    let files = app
        .rt
        .config()
        .storage
        .backup_dirs
        .first()
        .and_then(|d| backup::list_dir(d).ok())
        .unwrap_or_default();
    Json(BackupsOut {
        last: backup::last_status(&app.pool).await,
        files: files.into_iter().take(100).collect(),
    })
}

// ---------- Перенос данных ----------

#[derive(Deserialize)]
pub struct MoveIn {
    data_dir: String,
}

#[derive(Serialize)]
pub struct MoveOut {
    data_dir: PathBuf,
    restart_required: bool,
}

/// Перенос базы в другую папку: согласованная копия + ключи сервера, новый путь в
/// config.toml; до перезапуска — только чтение, чтобы ничего не записалось в старую базу.
pub async fn move_data(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<MoveIn>,
) -> Result<Json<MoveOut>, ApiError> {
    let target = parse_dir(&req.data_dir)?;
    if target.to_string_lossy().starts_with(r"\\") {
        return Err(ApiError::BadRequest(
            "базу нельзя хранить в сетевой папке — только на диске этого компьютера".into(),
        ));
    }
    let _exclusive = app.rt.backup_lock.lock().await;
    let current = app.rt.config().storage.db_dir;
    if target == current {
        return Err(ApiError::BadRequest(
            "это и есть текущая папка данных".into(),
        ));
    }
    let db = target.join(crate::db::DB_FILE);
    if db.exists() {
        return Err(ApiError::BadRequest(format!(
            "в {} уже есть база CRM — выберите пустую папку",
            target.display()
        )));
    }
    probe_writable(&target)?;
    crate::db::make_private(&target).map_err(|e| ApiError::Failed(e.to_string()))?;
    sqlx::query("VACUUM INTO ?")
        .bind(db.to_string_lossy().into_owned())
        .execute(&app.pool)
        .await?;
    backup::check_integrity(&db)
        .await
        .map_err(|e| ApiError::Failed(e.to_string()))?;
    copy_keys(&current.join("tls"), &target.join("tls"))
        .map_err(|e| ApiError::Failed(format!("ключи сервера не скопированы: {e}")))?;

    save(&app, |c| c.storage.db_dir = target.clone())?;
    app.rt.frozen.store(true, Ordering::SeqCst);
    tracing::warn!(to = %target.display(), "данные перенесены — нужен перезапуск");
    Ok(Json(MoveOut {
        data_dir: target,
        restart_required: true,
    }))
}

fn copy_keys(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(());
    };
    std::fs::create_dir_all(to)?;
    for e in entries.flatten() {
        if e.path()
            .extension()
            .is_some_and(|x| x == "crt" || x == "key")
        {
            std::fs::copy(e.path(), to.join(e.file_name()))?;
        }
    }
    Ok(())
}
