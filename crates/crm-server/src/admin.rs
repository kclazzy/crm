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
    pub(crate) path: String,
    /// Что лежит на томе: «Данные», «Резервные копии».
    roles: Vec<&'static str>,
    total_bytes: u64,
    free_bytes: u64,
    crm_bytes: u64,
    pub(crate) level: DiskLevel,
    /// Через сколько дней том заполнится при нынешнем темпе.
    pub(crate) days_left: Option<i64>,
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

/// История свободного места тома за окно прогноза.
async fn history(pool: &sqlx::SqlitePool, volume: &str) -> Vec<crm_core::disk::Sample> {
    let since = (chrono::Utc::now() - crm_core::disk::TREND_WINDOW).timestamp_millis();
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT taken_at, free_bytes FROM disk_usage_snapshots WHERE volume_id = ? AND taken_at >= ? ORDER BY taken_at",
    )
    .bind(volume)
    .bind(since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter_map(|(at, free)| {
            Some(crm_core::disk::Sample {
                at: chrono::DateTime::from_timestamp_millis(at)?,
                free_bytes: u64::try_from(free).ok()?,
            })
        })
        .collect()
}

/// Тома, где лежат данные CRM: заполненность, прогноз, что на них хранится.
pub(crate) async fn volumes(app: &AppState) -> Vec<(String, Volume)> {
    let cfg = app.rt.config();
    let mut places: Vec<(&'static str, PathBuf, u64)> = vec![
        (
            "Данные",
            cfg.storage.db_dir.clone(),
            dir_size(&cfg.storage.db_dir),
        ),
        (
            "Вложения",
            cfg.storage.attachments_dir.clone(),
            tree_size(&cfg.storage.attachments_dir),
        ),
    ];
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
        let status = assess(
            total,
            free,
            &history(&app.pool, &id).await,
            chrono::Utc::now(),
            Thresholds::default(),
        );
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
                level: status.level,
                days_left: status.days_left,
            },
        ));
    }
    volumes
}

/// Снимок заполненности томов — раз в час, для прогноза.
pub(crate) async fn snapshot_disks(app: &AppState) -> Result<Vec<Volume>, ApiError> {
    let vols = volumes(app).await;
    let now = crate::auth::now_ms();
    for (id, v) in &vols {
        sqlx::query(
            "INSERT OR REPLACE INTO disk_usage_snapshots(taken_at, volume_id, total_bytes, free_bytes, crm_bytes)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(now)
        .bind(id)
        .bind(i64::try_from(v.total_bytes).unwrap_or(i64::MAX))
        .bind(i64::try_from(v.free_bytes).unwrap_or(i64::MAX))
        .bind(serde_json::json!({ "crm": v.crm_bytes }).to_string())
        .execute(&app.pool)
        .await?;
    }
    // Старше года история не нужна.
    sqlx::query("DELETE FROM disk_usage_snapshots WHERE taken_at < ?")
        .bind(now - 366 * 24 * 3600 * 1000)
        .execute(&app.pool)
        .await?;
    Ok(vols.into_iter().map(|(_, v)| v).collect())
}

/// Размер папки со всеми вложенными (вложения разложены по подпапкам).
fn tree_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    Some(if m.is_dir() {
                        tree_size(&e.path())
                    } else {
                        m.len()
                    })
                })
                .sum()
        })
        .unwrap_or(0)
}

pub async fn storage(State(app): State<AppState>, Owner(_): Owner) -> Json<StorageInfo> {
    let cfg = app.rt.config();
    let volumes = volumes(&app).await;
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
    /// Копии шифруются паролем владельца.
    encrypted: bool,
    /// Последнее пробное восстановление.
    verified: Option<backup::Verified>,
}

/// Пробное восстановление сейчас (кнопка «Проверить копию»).
pub async fn verify_backup(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<backup::Verified>, ApiError> {
    backup::verify(&app).await.map(Json).map_err(|e| match e {
        BackupError::NothingToVerify => ApiError::BadRequest(e.to_string()),
        other => ApiError::Failed(other.to_string()),
    })
}

#[derive(Deserialize)]
pub struct BackupPasswordIn {
    /// Новый пароль копий; `null` — больше не шифровать.
    password: Option<String>,
}

/// Пароль копий: новые копии шифруются им. Прежние остаются со старым паролем.
pub async fn set_backup_password(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<BackupPasswordIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db_dir = app.rt.config().storage.db_dir;
    let _one_at_a_time = app.rt.backup_lock.lock().await;
    let action = match req.password {
        Some(p) => {
            if p.chars().count() < 10 {
                return Err(ApiError::BadRequest(
                    "пароль копий — не короче 10 символов".into(),
                ));
            }
            let key = tokio::task::spawn_blocking(move || {
                crate::backup_file::BackupKey::from_password(&p)
            })
            .await
            .map_err(|e| ApiError::Failed(e.to_string()))?
            .map_err(|e| ApiError::Failed(e.to_string()))?;
            key.save(&db_dir)
                .map_err(|e| ApiError::Failed(e.to_string()))?;
            "backup.encrypted"
        }
        None => {
            crate::backup_file::BackupKey::remove(&db_dir)
                .map_err(|e| ApiError::Failed(e.to_string()))?;
            "backup.unencrypted"
        }
    };
    crate::auth::audit(&app.pool, Some(&owner.id), action, None, "local").await?;
    Ok(Json(
        serde_json::json!({ "encrypted": action == "backup.encrypted" }),
    ))
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
    let encrypted = crate::backup_file::BackupKey::load(&app.rt.config().storage.db_dir)
        .ok()
        .flatten()
        .is_some();
    Json(BackupsOut {
        last: backup::last_status(&app.pool).await,
        files: files.into_iter().take(100).collect(),
        encrypted,
        verified: backup::last_verified(&app.pool).await,
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

// ---------- Безопасность ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct FailedLogins {
    ip: String,
    count: i64,
    last_at: i64,
}

#[derive(Serialize)]
pub struct SecurityOut {
    /// Адреса, которым сервер отказывал из-за слишком частых запросов (сутки).
    blocked: Vec<crate::limits::Blocked>,
    /// Неудачные входы и попытки кодов восстановления по адресам (сутки).
    failed_logins: Vec<FailedLogins>,
}

pub async fn security(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<SecurityOut>, ApiError> {
    let since = chrono::Utc::now().timestamp_millis() - 86_400_000;
    let failed_logins = sqlx::query_as(
        "SELECT ip, count(*) AS count, max(created_at) AS last_at FROM audit_log
         WHERE action IN ('login.failed', 'recover.failed') AND created_at > ? AND ip IS NOT NULL
         GROUP BY ip ORDER BY count DESC LIMIT 50",
    )
    .bind(since)
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(SecurityOut {
        blocked: app.rt.limiter.blocked(),
        failed_logins,
    }))
}

// ---------- Журнал безопасности ----------

/// События безопасности и их названия для владельца. Остальные записи журнала
/// (работа с задачами) сюда не попадают.
const JOURNAL: &[(&str, &str, &str)] = &[
    ("login.ok", "login", "Вход"),
    (
        "login.ticket",
        "login",
        "Вход владельца на сервере без пароля",
    ),
    ("login.failed", "login", "Неверный пароль"),
    ("recover.ok", "login", "Пароль восстановлен кодом"),
    ("recover.failed", "login", "Неверный код восстановления"),
    ("reset.requested", "login", "Просит сбросить пароль"),
    ("user.disabled", "access", "Сотрудник отключён"),
    ("user.enabled", "access", "Сотрудник включён"),
    ("user.deleted", "access", "Сотрудник удалён"),
    ("2fa.enabled", "access", "Код из приложения настроен"),
    ("2fa.disabled", "access", "Код из приложения выключен"),
    (
        "2fa.reset",
        "access",
        "Код из приложения сброшен владельцем",
    ),
    (
        "2fa.policy",
        "access",
        "Изменено правило кода из приложения",
    ),
    (
        "recovery.codes_issued",
        "access",
        "Выпущены коды восстановления владельца",
    ),
    (
        "owner.reset_on_server",
        "access",
        "Пароль владельца сброшен на сервере",
    ),
    ("p2p.key", "access", "Новый ключ удалённого доступа"),
    ("backup.encrypted", "data", "Задан пароль резервных копий"),
    ("backup.unencrypted", "data", "Шифрование копий выключено"),
    ("server.crt", "data", "Новый сертификат сервера"),
    ("server.key", "data", "Новый ключ сертификата"),
    (
        "restore.staged",
        "data",
        "Подготовлено восстановление из копии",
    ),
    ("import.merge", "data", "Импорт базы: объединение"),
    ("import.replace", "data", "Импорт базы: замена"),
    ("move.packed", "data", "Собран пакет переезда"),
];

#[derive(Deserialize)]
pub struct JournalIn {
    /// Сотрудник: его действия и события о нём.
    user: Option<String>,
    /// login | access | data
    kind: Option<String>,
    /// Записи раньше этого времени (мс) — следующая страница.
    before: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize)]
pub struct JournalItem {
    id: String,
    at: i64,
    action: String,
    title: &'static str,
    kind: &'static str,
    actor_name: Option<String>,
    /// Логин, о котором событие (в том числе несуществующий — при подборе).
    about: Option<String>,
    about_name: Option<String>,
    ip: Option<String>,
    /// server | office | device | internet
    place: &'static str,
}

/// События журнала нужного вида — JSON-массив для `json_each`.
fn journal_actions(kind: Option<&str>) -> String {
    let actions: Vec<&str> = JOURNAL
        .iter()
        .filter(|(_, k, _)| kind.is_none_or(|want| want == *k))
        .map(|(a, _, _)| *a)
        .collect();
    serde_json::to_string(&actions).unwrap_or_else(|_| "[]".into())
}

/// Удалить запись журнала. Она скрывается, а не стирается: блокировка за подбор
/// пароля по-прежнему её учитывает.
pub async fn journal_hide(
    State(app): State<AppState>,
    Owner(_): Owner,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    let done = sqlx::query("UPDATE audit_log SET hidden_at = ? WHERE id = ? AND hidden_at IS NULL")
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(&id)
        .execute(&app.pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Удалить все записи журнала по тем же фильтрам, что и список (сотрудник, вид).
pub async fn journal_hide_all(
    State(app): State<AppState>,
    Owner(_): Owner,
    axum::extract::Query(q): axum::extract::Query<JournalIn>,
) -> Result<axum::http::StatusCode, ApiError> {
    sqlx::query(
        "UPDATE audit_log SET hidden_at = ?4
         WHERE hidden_at IS NULL AND action IN (SELECT value FROM json_each(?1))
           AND (?2 IS NULL OR actor_id = ?2
                OR entity_id = (SELECT username FROM users WHERE id = ?2))
           AND (?3 IS NULL OR created_at < ?3)",
    )
    .bind(journal_actions(q.kind.as_deref()))
    .bind(&q.user)
    .bind(q.before)
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(&app.pool)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// id, время, действие, кто, о ком (логин), имя того, о ком, адрес.
type JournalRow = (
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Откуда событие — словами для журнала.
fn journal_place(ip: Option<&str>) -> &'static str {
    match ip {
        None | Some("" | "local") => "server",
        Some(ip)
            if ip
                .parse::<std::net::IpAddr>()
                .is_ok_and(|a| a.is_loopback()) =>
        {
            "server"
        }
        Some(ip) if crate::p2p::is_device_addr(ip) => "device",
        Some(ip) if crm_core::remote::is_office_addr(ip) => "office",
        Some(_) => "internet",
    }
}

pub async fn journal(
    State(app): State<AppState>,
    Owner(_): Owner,
    axum::extract::Query(q): axum::extract::Query<JournalIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actions = journal_actions(q.kind.as_deref());
    let rows: Vec<JournalRow> =
        sqlx::query_as(
            "SELECT a.id, a.created_at, a.action, actor.display_name, a.entity_id, about.display_name, a.ip
             FROM audit_log a
             LEFT JOIN users actor ON actor.id = a.actor_id
             LEFT JOIN users about ON about.username = a.entity_id
             WHERE a.hidden_at IS NULL AND a.action IN (SELECT value FROM json_each(?1))
               AND (?2 IS NULL OR a.actor_id = ?2
                    OR a.entity_id = (SELECT username FROM users WHERE id = ?2))
               AND (?3 IS NULL OR a.created_at < ?3)
             ORDER BY a.created_at DESC, a.id DESC
             LIMIT ?4",
        )
        .bind(&actions)
        .bind(&q.user)
        .bind(q.before)
        .bind(q.limit.unwrap_or(100).clamp(1, 500))
        .fetch_all(&app.pool)
        .await?;
    let items: Vec<JournalItem> = rows
        .into_iter()
        .filter_map(|(id, at, action, actor_name, about, about_name, ip)| {
            let (_, kind, title) = JOURNAL.iter().find(|(a, _, _)| *a == action)?;
            Some(JournalItem {
                place: journal_place(ip.as_deref()),
                id,
                at,
                action,
                title,
                kind,
                actor_name,
                about,
                about_name,
                ip,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

// ---------- Восстановление и переезд ----------

#[derive(Deserialize)]
pub struct RestoreIn {
    /// Папка копий (по умолчанию — первая из настроенных) и имя копии или пакета.
    dir: Option<String>,
    name: Option<String>,
    /// Пароль копий — если они зашифрованы на другом компьютере.
    #[serde(default)]
    password: Option<String>,
}

/// Подготовить восстановление из копии; встанет на место после перезапуска.
pub async fn restore(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<RestoreIn>,
) -> Result<Json<crate::restore::Staged>, ApiError> {
    let cfg = app.rt.config();
    let dir = match req.dir.as_deref() {
        Some(d) => parse_dir(d)?,
        None => cfg
            .storage
            .backup_dirs
            .first()
            .cloned()
            .ok_or_else(|| ApiError::BadRequest("не выбрана папка резервных копий".into()))?,
    };
    let source = match req.name.as_deref() {
        Some(n) if n.contains(['/', '\\']) || n.starts_with('.') => {
            return Err(ApiError::BadRequest("имя копии без пути".into()))
        }
        Some(n) => dir.join(n),
        None => dir,
    };
    // Отдельная копия «до» не делается: при запуске прежняя база сохраняется
    // как before_restore_*.db (новая копия той же минуты затёрла бы выбранную).
    let staged = crate::restore::stage(&cfg.storage, &source, req.password.as_deref()).await?;
    app.rt.frozen.store(true, Ordering::SeqCst);
    crate::auth::audit(
        &app.pool,
        Some(&owner.id),
        "restore.staged",
        Some(&staged.name),
        "local",
    )
    .await?;
    Ok(Json(staged))
}

#[derive(Deserialize)]
pub struct MoveOutIn {
    /// Куда положить пакет переноса (флешка, сетевая папка).
    dir: String,
}

#[derive(Serialize)]
pub struct Packed {
    file: String,
}

/// Переезд: свежая копия → пакет переноса; этот сервер больше не принимает изменения.
pub async fn move_out(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<MoveOutIn>,
) -> Result<Json<Packed>, ApiError> {
    let out = parse_dir(&req.dir)?;
    probe_writable(&out)?;
    let cfg = app.rt.config();
    let backup_dir = cfg
        .storage
        .backup_dirs
        .first()
        .cloned()
        .ok_or_else(|| ApiError::BadRequest("сначала выберите папку резервных копий".into()))?;
    // Сначала запрещаем запись, потом копируем — изменения после копии не потеряются.
    app.rt.frozen.store(true, Ordering::SeqCst);
    let result = async {
        crate::backup::run(&app)
            .await
            .map_err(|e| ApiError::Failed(e.to_string()))?;
        tokio::task::spawn_blocking(move || crate::restore::pack(&backup_dir, &out))
            .await
            .map_err(|e| ApiError::Failed(e.to_string()))?
            .map_err(|e| ApiError::Failed(e.to_string()))
    }
    .await;
    let file = match result {
        Ok(f) => f,
        Err(e) => {
            app.rt.frozen.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    crate::auth::audit(&app.pool, Some(&owner.id), "move.packed", None, "local").await?;
    Ok(Json(Packed {
        file: file.display().to_string(),
    }))
}
