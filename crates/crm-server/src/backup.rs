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
}

impl Status {
    /// Копия сохранилась хотя бы в одну папку.
    pub fn saved(&self) -> bool {
        self.ok || self.dirs.iter().any(|d| d.ok)
    }
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
        Some(s) if s.saved() => now - s.at >= i64::from(policy.every_hours()) * 3600 * 1000,
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
    let result = run_locked(&app.pool, &cfg, tz).await;
    let status = match &result {
        Ok((f, dirs)) => Status {
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
        },
    };
    if let Ok(json) = serde_json::to_string(&status) {
        let _ = sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(STATUS_KEY)
            .bind(json)
            .execute(&app.pool)
            .await;
    }
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
    let snapshot = db_dir.join("backup-snapshot.tmp.db");
    let packed = db_dir.join("backup-packed.tmp");
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
            dirs.iter()
                .map(|dir| {
                    let r = write_one(&packed, &name, &manifest, dir, &db_dir, &files, now, tz, &policy);
                    if let Err(e) = &r {
                        tracing::error!(dir = %dir.display(), error = %e, "копия в папку не записана");
                    }
                    DirStatus {
                        path: dir.clone(),
                        ok: r.is_ok(),
                        error: r.err().map(|e| format!("{}: {e}", dir.display())),
                    }
                })
                .collect::<Vec<_>>()
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
) -> Result<(), BackupError> {
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    crate::db::make_private(dir).map_err(|e| BackupError::Corrupt(e.to_string()))?;
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::copy(packed, &tmp).map_err(io(&tmp))?;
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

/// Фоновый воркер: копия по расписанию владельца, если папка выбрана.
pub async fn hourly(app: AppState) {
    tokio::time::sleep(FIRST_DELAY).await;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    loop {
        tick.tick().await;
        let cfg = app.rt.config();
        let last = last_status(&app.pool).await;
        let now = Utc::now().timestamp_millis();
        if cfg.storage.backup_dirs.is_empty() || !due(&cfg.backup, last.as_ref(), now) {
            continue;
        }
        if let Err(e) = run(&app).await {
            tracing::error!(error = %e, "резервное копирование не удалось");
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
        }
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
