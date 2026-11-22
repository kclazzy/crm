//! Резервное копирование (ТЗ §5): горячий снимок БД через VACUUM INTO, проверка
//! целостности, сжатие потоком (по желанию — шифрование паролем), копия в каждую
//! папку отдельно — недоступная папка не мешает остальным; уборка старых копий
//! и вложений, на которые не ссылается ни одна оставшаяся копия.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqlitePool};

use crate::backup_file::BackupKey;
use crate::config::Config;
use crate::http::AppState;

/// Первая проверка — через 2 минуты после старта, дальше раз в минуту:
/// копия делается, когда с последней удачной прошёл выбранный владельцем интервал.
const FIRST_DELAY: Duration = Duration::from_secs(120);
const CHECK_EVERY: Duration = Duration::from_secs(60);
/// После неудачи (никуда не сохранилось) — новая попытка не раньше чем через 15 минут,
/// а не каждую минуту: полный снимок базы зря нагружает диск.
pub const RETRY_AFTER_MS: i64 = 15 * 60 * 1000;
const STATUS_KEY: &str = "backup.last";
/// Список вложений, на которые ссылается копия: `<имя копии>.files`.
const MANIFEST: &str = ".files";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub at: i64,
    /// Копия записана во все папки.
    pub ok: bool,
    pub file: Option<String>,
    pub size: Option<u64>,
    pub error: Option<String>,
    /// Итог по каждой папке.
    #[serde(default)]
    pub dirs: Vec<DirStatus>,
    /// Счётчик изменений данных на момент копии.
    #[serde(default)]
    pub marker: Option<i64>,
    /// Когда последний раз проверили и копия не понадобилась (данные не менялись).
    #[serde(default)]
    pub unchanged_at: Option<i64>,
}

impl Status {
    /// Копия сохранилась хотя бы в одну папку.
    pub fn saved(&self) -> bool {
        self.ok || self.dirs.iter().any(|d| d.ok)
    }

    /// Когда последний раз копия была сделана или оказалась не нужна.
    pub fn checked_at(&self) -> i64 {
        self.unchanged_at.map_or(self.at, |u| u.max(self.at))
    }
}

/// Таблицы, изменения в которых — не повод для новой копии: вход, уведомления,
/// журнал, отметки «прочитано», служебные настройки и сам счётчик.
const NOT_WORTH_A_COPY: &[&str] = &[
    "settings",
    "sessions",
    "notifications",
    "audit_log",
    "reminders_sent",
    "task_alerts",
    "task_views",
    "chat_reads",
    "local_tickets",
    "enroll_codes",
    "disk_usage_snapshots",
    "backup_marker",
    "_sqlx_migrations",
];

/// Поставить триггеры-счётчики на все рабочие таблицы (повторно — ничего не меняет).
pub async fn install_change_marker(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let all: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, coalesce(sql, '') FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await?;
    // Поиск (FTS) — виртуальная таблица и её служебные: их содержимое выводится
    // из рабочих таблиц, а триггеры на них ставить нельзя.
    let virtual_tables: Vec<&str> = all
        .iter()
        .filter(|(_, sql)| sql.to_uppercase().starts_with("CREATE VIRTUAL"))
        .map(|(n, _)| n.as_str())
        .collect();
    let derived = |t: &str| {
        virtual_tables
            .iter()
            .any(|v| t == *v || t.starts_with(&format!("{v}_")))
    };
    for (t, _) in all
        .iter()
        .filter(|(t, _)| !NOT_WORTH_A_COPY.contains(&t.as_str()) && !derived(t))
    {
        if !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue; // имена таблиц наши; на всякий случай — только простые
        }
        for op in ["INSERT", "UPDATE", "DELETE"] {
            let sql = format!(
                "CREATE TRIGGER IF NOT EXISTS crm_mark_{t}_{op} AFTER {op} ON \"{t}\"
                 BEGIN UPDATE backup_marker SET n = n + 1 WHERE id = 1; END"
            );
            sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await?;
        }
    }
    Ok(())
}

async fn change_marker(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT n FROM backup_marker WHERE id = 1")
        .fetch_one(pool)
        .await
}

/// Итог копии по расписанию.
#[derive(Debug)]
pub enum Scheduled {
    Saved(BackupFile),
    /// С прошлой копии рабочие данные не менялись — копия не нужна.
    Unchanged,
    Failed(String),
}

/// Копия по расписанию: пропускается, если данные не менялись с прошлой удачной.
pub async fn scheduled(app: &AppState) -> Scheduled {
    let last = last_status(&app.pool).await;
    if let (Some(s), Ok(now)) = (&last, change_marker(&app.pool).await) {
        if s.ok && s.marker == Some(now) {
            let mut s = s.clone();
            s.unchanged_at = Some(Utc::now().timestamp_millis());
            save_status(&app.pool, &s).await;
            return Scheduled::Unchanged;
        }
    }
    match run(app).await {
        Ok(f) => Scheduled::Saved(f),
        Err(e) => Scheduled::Failed(e.to_string()),
    }
}

async fn save_status(pool: &SqlitePool, s: &Status) {
    if let Ok(json) = serde_json::to_string(s) {
        let _ = sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(STATUS_KEY)
            .bind(json)
            .execute(pool)
            .await;
    }
}

/// Где делать временный снимок и сжатие: на диске копий (первая доступная папка),
/// чтобы не изнашивать диск с базой; если ни одна не доступна — рядом с базой.
pub fn workspace(dirs: &[PathBuf], db_dir: &Path) -> PathBuf {
    for d in dirs {
        let probe = d.join(".crm-probe");
        // Только существующая папка: отключённую флешку или диск не создаём.
        if d.is_dir() && std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return d.clone();
        }
    }
    db_dir.to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirStatus {
    pub path: PathBuf,
    pub ok: bool,
    pub error: Option<String>,
}

/// Пора ли делать копию: по расписанию после удачной, через 15 минут после неудачной.
pub fn due(policy: &crm_core::backup::Policy, last: Option<&Status>, now: i64) -> bool {
    if !policy.enabled() {
        return false;
    }
    match last {
        None => true,
        Some(s) if s.saved() => {
            now - s.checked_at() >= i64::from(policy.every_hours()) * 3600 * 1000
        }
        Some(s) => now - s.at >= RETRY_AFTER_MS,
    }
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
    #[error("копия не записалась ни в одну папку: {0}")]
    NowhereSaved(String),
    #[error("копий ещё нет — проверять нечего")]
    NothingToVerify,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> BackupError + '_ {
    move |source| BackupError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Сделать бэкап сейчас. Возвращает имя файла и размер копии; если хоть одна папка
/// недоступна — копия в остальных всё равно делается, а статус это показывает.
pub async fn run(app: &AppState) -> Result<BackupFile, BackupError> {
    let _one_at_a_time = app.rt.backup_lock.lock().await;
    let cfg = app.rt.config();
    let tz = crate::org::load(&app.pool)
        .await
        .map(|o| o.tz())
        .unwrap_or(chrono_tz::Europe::Moscow);
    let marker = change_marker(&app.pool).await.ok();
    let result = run_locked(&app.pool, &cfg, tz).await;
    let status = match &result {
        Ok((f, dirs)) => Status {
            marker,
            unchanged_at: None,
            at: f.at,
            ok: dirs.iter().all(|d| d.ok),
            file: Some(f.name.clone()),
            size: Some(f.size),
            error: dirs
                .iter()
                .filter_map(|d| d.error.clone())
                .reduce(|a, b| format!("{a}; {b}")),
            dirs: dirs.clone(),
        },
        Err(e) => Status {
            at: Utc::now().timestamp_millis(),
            ok: false,
            file: None,
            size: None,
            error: Some(e.to_string()),
            dirs: Vec::new(),
            marker: None,
            unchanged_at: None,
        },
    };
    save_status(&app.pool, &status).await;
    result.map(|(f, _)| f)
}

async fn run_locked(
    pool: &SqlitePool,
    cfg: &Config,
    tz: chrono_tz::Tz,
) -> Result<(BackupFile, Vec<DirStatus>), BackupError> {
    let dirs = cfg.storage.backup_dirs.clone();
    if dirs.is_empty() {
        return Err(BackupError::NoDirs);
    }
    let db_dir = cfg.storage.db_dir.clone();
    // Снимок и сжатие — на диске копий: диск с базой (SSD) почти не пишется.
    let work = workspace(&dirs, &db_dir);
    let snapshot = work.join(".crm-snapshot.tmp.db");
    let packed = work.join(".crm-packed.tmp");
    for p in [&snapshot, &packed] {
        if p.exists() {
            std::fs::remove_file(p).map_err(io(p))?;
        }
    }
    // Горячий согласованный снимок: работа сотрудников не останавливается.
    sqlx::query("VACUUM INTO ?")
        .bind(snapshot.to_string_lossy().into_owned())
        .execute(pool)
        .await?;
    check_integrity(&snapshot).await?;
    let manifest = attachment_list(&snapshot).await?;

    let now = Utc::now();
    let name = crm_core::backup::file_name(now);
    let key = BackupKey::load(&db_dir).map_err(io(&db_dir))?;
    let size = tokio::task::spawn_blocking({
        let (snapshot, packed) = (snapshot.clone(), packed.clone());
        move || crate::backup_file::pack(&snapshot, &packed, key.as_ref()).map_err(io(&packed))
    })
    .await??;
    let _ = std::fs::remove_file(&snapshot);

    let results = tokio::task::spawn_blocking({
        let (name, packed) = (name.clone(), packed.clone());
        let files = cfg.storage.attachments_dir.clone();
        let policy = cfg.backup;
        move || {
            // Папка, где лежит сжатая копия, — последней: туда файл переносится, а не копируется.
            let mut order: Vec<&PathBuf> = dirs.iter().filter(|d| **d != work).collect();
            order.extend(dirs.iter().filter(|d| **d == work));
            let mut out: Vec<DirStatus> = order
                .into_iter()
                .map(|dir| {
                    let r = write_one(&packed, &name, &manifest, dir, &db_dir, &files, now, tz, &policy, *dir == work);
                    if let Err(e) = &r {
                        tracing::error!(dir = %dir.display(), error = %e, "копия в папку не записана");
                    }
                    DirStatus {
                        path: dir.clone(),
                        ok: r.is_ok(),
                        error: r.err().map(|e| format!("{}: {e}", dir.display())),
                    }
                })
                .collect();
            // В статусе — в том порядке, в каком папки заданы.
            out.sort_by_key(|d| dirs.iter().position(|x| *x == d.path));
            out
        }
    })
    .await?;
    let _ = std::fs::remove_file(&packed);
    if !results.iter().any(|d| d.ok) {
        let why = results
            .iter()
            .filter_map(|d| d.error.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(BackupError::NowhereSaved(why));
    }
    tracing::info!(%name, size, "резервная копия создана");
    Ok((
        BackupFile {
            name,
            at: now.timestamp_millis(),
            size,
        },
        results,
    ))
}

/// Какие вложения есть в снимке — чтобы потом убирать из папки копий ненужные.
async fn attachment_list(db: &Path) -> Result<Vec<String>, BackupError> {
    let mut conn = SqliteConnectOptions::new()
        .filename(db)
        .read_only(true)
        .connect()
        .await?;
    let list: Vec<String> = sqlx::query_scalar("SELECT sha256 FROM attachments")
        .fetch_all(&mut conn)
        .await?;
    sqlx::Connection::close(conn).await?;
    Ok(list)
}

#[allow(clippy::too_many_arguments)]
fn write_one(
    packed: &Path,
    name: &str,
    manifest: &[String],
    dir: &Path,
    db_dir: &Path,
    attachments: &Path,
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
    policy: &crm_core::backup::Policy,
    move_packed: bool,
) -> Result<(), BackupError> {
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    crate::db::make_private(dir).map_err(|e| BackupError::Corrupt(e.to_string()))?;
    let tmp = dir.join(format!("{name}.tmp"));
    if move_packed {
        std::fs::rename(packed, &tmp).map_err(io(&tmp))?;
    } else {
        std::fs::copy(packed, &tmp).map_err(io(&tmp))?;
    }
    let list = dir.join(format!("{name}{MANIFEST}"));
    std::fs::write(&list, manifest.join("\n")).map_err(io(&list))?;
    std::fs::rename(&tmp, dir.join(name)).map_err(io(dir))?;
    copy_keys(db_dir, dir)?;
    copy_new_files(attachments, &dir.join("attachments"))?;
    rotate(dir, now, tz, policy)?;
    prune_attachments(dir)?;
    Ok(())
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
    // Закрыть явно и дождаться: на Windows открытый файл нельзя переименовать,
    // а простой drop закрывает соединение в фоне, позже.
    sqlx::Connection::close(conn).await?;
    if verdict == "ok" {
        Ok(())
    } else {
        Err(BackupError::Corrupt(verdict))
    }
}

/// Вложения — инкрементально: файл с тем же именем (sha256) уже есть — не копируем.
pub(crate) fn copy_new_files(from: &Path, to: &Path) -> Result<usize, BackupError> {
    let Ok(prefixes) = std::fs::read_dir(from) else {
        return Ok(0);
    };
    let mut copied = 0;
    for p in prefixes.flatten() {
        let name = p.file_name();
        // Только папки вида «ab» — временные загрузки (tmp) не копируем.
        if name.len() != 2 || !p.path().is_dir() {
            continue;
        }
        let dest_dir = to.join(&name);
        for f in std::fs::read_dir(p.path())
            .map_err(io(&p.path()))?
            .flatten()
        {
            let dest = dest_dir.join(f.file_name());
            if dest.exists() {
                continue;
            }
            std::fs::create_dir_all(&dest_dir).map_err(io(&dest_dir))?;
            let tmp = dest.with_extension("part");
            std::fs::copy(f.path(), &tmp).map_err(io(&tmp))?;
            std::fs::rename(&tmp, &dest).map_err(io(&dest))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Ключи сервера — сертификат и ключ удалённого доступа: восстановленный сервер
/// остаётся «тем же» для сотрудников (в офисе и из дома). Ключ шифрования копий
/// (`backup.key`) сюда не попадает — иначе шифровать было бы незачем.
pub(crate) fn copy_keys(db_dir: &Path, to: &Path) -> Result<(), BackupError> {
    copy_tls(&db_dir.join("tls"), &to.join("tls"))?;
    let p2p = db_dir.join(P2P_KEY);
    if p2p.exists() {
        std::fs::create_dir_all(to).map_err(io(to))?;
        std::fs::copy(&p2p, to.join(P2P_KEY)).map_err(io(&p2p))?;
    }
    Ok(())
}

pub(crate) const P2P_KEY: &str = "p2p.key";

/// Сертификат и ключ HTTPS.
pub(crate) fn copy_tls(from: &Path, to: &Path) -> Result<(), BackupError> {
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

/// Уборка старых копий по правилу владельца (сутки — по часовому поясу организации).
/// Вместе с копией уходит её список вложений. Чужие файлы не трогаем.
fn rotate(
    dir: &Path,
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
    policy: &crm_core::backup::Policy,
) -> Result<(), BackupError> {
    let files = list_dir(dir)?;
    let times: Vec<(DateTime<Utc>, u64)> = files
        .iter()
        .filter_map(|f| DateTime::from_timestamp_millis(f.at).map(|t| (t, f.size)))
        .collect();
    for t in crm_core::backup::to_delete(&times, now, tz, policy) {
        let name = crm_core::backup::file_name(t);
        let p = dir.join(&name);
        std::fs::remove_file(&p).map_err(io(&p))?;
        let _ = std::fs::remove_file(dir.join(format!("{name}{MANIFEST}")));
    }
    Ok(())
}

/// Убрать из папки копий вложения, на которые не ссылается ни одна оставшаяся копия.
/// Если хоть у одной копии нет списка (сделана прежней версией) — ничего не трогаем.
pub(crate) fn prune_attachments(dir: &Path) -> Result<usize, BackupError> {
    let mut needed: HashSet<String> = HashSet::new();
    for f in list_dir(dir)? {
        let list = dir.join(format!("{}{MANIFEST}", f.name));
        match std::fs::read_to_string(&list) {
            Ok(s) => needed.extend(s.lines().map(str::to_owned)),
            Err(_) => return Ok(0),
        }
    }
    let root = dir.join("attachments");
    let Ok(prefixes) = std::fs::read_dir(&root) else {
        return Ok(0);
    };
    let mut removed = 0;
    for p in prefixes.flatten() {
        if !p.path().is_dir() || p.file_name().len() != 2 {
            continue;
        }
        for f in std::fs::read_dir(p.path())
            .map_err(io(&p.path()))?
            .flatten()
        {
            let name = f.file_name().to_string_lossy().into_owned();
            if !needed.contains(&name) {
                std::fs::remove_file(f.path()).map_err(io(&f.path()))?;
                removed += 1;
            }
        }
    }
    Ok(removed)
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

const VERIFY_KEY: &str = "backup.verified";
/// Пробное восстановление — раз в неделю.
pub const VERIFY_EVERY_MS: i64 = 7 * 24 * 3600 * 1000;

/// Итог пробного восстановления.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verified {
    pub at: i64,
    pub ok: bool,
    pub file: Option<String>,
    /// Сколько в копии сотрудников, клиентов и задач — «копия не пустая».
    pub users: i64,
    pub clients: i64,
    pub tasks: i64,
    pub error: Option<String>,
}

/// Пробное восстановление: самая новая копия каждой папки распаковывается во
/// временную папку, открывается только на чтение и проверяется. Рабочая база
/// не затрагивается. При сбое владелец получает уведомление.
pub async fn verify(app: &AppState) -> Result<Verified, BackupError> {
    let cfg = app.rt.config();
    let mut newest: Vec<(PathBuf, BackupFile)> = Vec::new();
    for d in &cfg.storage.backup_dirs {
        if let Some(f) = list_dir(d).ok().and_then(|l| l.into_iter().next()) {
            newest.push((d.clone(), f));
        }
    }
    if newest.is_empty() {
        return Err(BackupError::NothingToVerify);
    }
    // Распаковка для проверки — тоже на диске копий, а не на диске с базой.
    let work = workspace(&cfg.storage.backup_dirs, &cfg.storage.db_dir).join(".crm-verify");
    let mut result = Verified {
        at: Utc::now().timestamp_millis(),
        ok: true,
        file: None,
        users: 0,
        clients: 0,
        tasks: 0,
        error: None,
    };
    for (dir, f) in &newest {
        let _ = std::fs::remove_dir_all(&work);
        let checked = async {
            std::fs::create_dir_all(&work).map_err(io(&work))?;
            let db = work.join("check.db");
            crate::restore::unpack(&cfg.storage, &dir.join(&f.name), &db, None)
                .map_err(|e| BackupError::Corrupt(e.to_string()))?;
            check_integrity(&db).await?;
            counts(&db).await
        }
        .await;
        let _ = std::fs::remove_dir_all(&work);
        result.file = Some(f.name.clone());
        match checked {
            Ok((u, c, t)) if u > 0 => (result.users, result.clients, result.tasks) = (u, c, t),
            Ok(_) => {
                result.ok = false;
                result.error = Some(format!(
                    "{}: в копии нет ни одного сотрудника",
                    dir.display()
                ));
            }
            Err(e) => {
                result.ok = false;
                result.error = Some(format!("{}/{}: {e}", dir.display(), f.name));
            }
        }
        if !result.ok {
            break;
        }
    }
    if let Ok(json) = serde_json::to_string(&result) {
        let _ = sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(VERIFY_KEY)
            .bind(json)
            .execute(&app.pool)
            .await;
    }
    if !result.ok {
        tracing::error!(error = ?result.error, "пробное восстановление не прошло");
        if let Ok(owners) = crate::notify::owners(app).await {
            for o in owners {
                let _ = crate::notify::push(
                    app,
                    &o,
                    None,
                    "backup_problem",
                    serde_json::json!({ "error": result.error, "verify": true }),
                )
                .await;
            }
        }
    }
    Ok(result)
}

async fn counts(db: &Path) -> Result<(i64, i64, i64), BackupError> {
    let mut conn = SqliteConnectOptions::new()
        .filename(db)
        .read_only(true)
        .connect()
        .await?;
    let n: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM users), (SELECT count(*) FROM clients),
                (SELECT count(*) FROM tasks)",
    )
    .fetch_one(&mut conn)
    .await?;
    sqlx::Connection::close(conn).await?;
    Ok(n)
}

pub async fn last_verified(pool: &SqlitePool) -> Option<Verified> {
    let json: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(VERIFY_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    json.and_then(|j| serde_json::from_str(&j).ok())
}

/// Пора ли пробное восстановление: копия есть, а проверки не было неделю.
pub fn verify_due(last_copy: Option<&Status>, last_check: Option<&Verified>, now: i64) -> bool {
    last_copy.is_some_and(Status::saved) && last_check.is_none_or(|v| now - v.at >= VERIFY_EVERY_MS)
}

/// Фоновый воркер: копия по расписанию владельца, если папка выбрана.
pub async fn hourly(app: AppState) {
    tokio::time::sleep(FIRST_DELAY).await;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    loop {
        tick.tick().await;
        let cfg = app.rt.config();
        let last = last_status(&app.pool).await;
        let now = Utc::now().timestamp_millis();
        if cfg.storage.backup_dirs.is_empty() {
            continue;
        }
        if due(&cfg.backup, last.as_ref(), now) {
            if let Scheduled::Failed(e) = scheduled(&app).await {
                tracing::error!(error = %e, "резервное копирование не удалось");
            }
        } else if verify_due(last.as_ref(), last_verified(&app.pool).await.as_ref(), now) {
            let _ = verify(&app).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(at: i64, ok: bool, dirs: &[bool]) -> Status {
        Status {
            at,
            ok,
            file: None,
            size: None,
            error: None,
            dirs: dirs
                .iter()
                .map(|&ok| DirStatus {
                    path: PathBuf::from("x"),
                    ok,
                    error: None,
                })
                .collect(),
            marker: None,
            unchanged_at: None,
        }
    }

    #[test]
    fn unchanged_check_counts_as_a_check_for_the_schedule() {
        let p = crm_core::backup::Policy::default(); // каждый час
        let h = 3600 * 1000;
        let mut s = st(0, true, &[]);
        s.unchanged_at = Some(5 * h);
        assert!(
            !due(&p, Some(&s), 5 * h + h - 1),
            "проверили час назад — ещё рано"
        );
        assert!(due(&p, Some(&s), 6 * h));
    }

    #[test]
    fn trial_restore_runs_weekly_once_a_copy_exists() {
        let saved = st(0, true, &[]);
        let failed = st(0, false, &[false]);
        assert!(!verify_due(None, None, 0), "копий нет");
        assert!(!verify_due(Some(&failed), None, 0));
        assert!(verify_due(Some(&saved), None, 0));
        let v = |at| Verified {
            at,
            ok: true,
            file: None,
            users: 1,
            clients: 0,
            tasks: 0,
            error: None,
        };
        assert!(!verify_due(Some(&saved), Some(&v(0)), VERIFY_EVERY_MS - 1));
        assert!(verify_due(Some(&saved), Some(&v(0)), VERIFY_EVERY_MS));
    }

    #[test]
    fn retry_waits_15_minutes_but_partial_success_keeps_schedule() {
        let p = crm_core::backup::Policy::default(); // каждый час
        let h = 3600 * 1000;
        assert!(due(&p, None, 0));
        let failed = st(0, false, &[false, false]);
        assert!(
            !due(&p, Some(&failed), RETRY_AFTER_MS - 1),
            "не каждую минуту"
        );
        assert!(due(&p, Some(&failed), RETRY_AFTER_MS));
        let partly = st(0, false, &[false, true]);
        assert!(
            !due(&p, Some(&partly), RETRY_AFTER_MS),
            "куда-то сохранилось — по расписанию"
        );
        assert!(due(&p, Some(&partly), h));
        let old_format = st(0, true, &[]);
        assert!(!due(&p, Some(&old_format), h - 1));
        assert!(due(&p, Some(&old_format), h));
    }
}
