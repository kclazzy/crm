//! Восстановление из копии и переезд на другой компьютер (ТЗ §2.8, §5).
//!
//! Копия готовится рядом с рабочей базой (`crm.db.restore`) и встаёт на место
//! при следующем запуске сервера — пока база открыта, подменять её нельзя.

use std::path::{Path, PathBuf};

use crate::backup::{self, BackupError};
use crate::config::Storage;

pub const STAGED: &str = "crm.db.restore";
const DB: &str = "crm.db";

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("в {0} нет резервных копий")]
    NoBackups(PathBuf),
    #[error("{0} — это не копия CRM (ожидается .db.zst, .crmpack или папка копий)")]
    Unknown(PathBuf),
    #[error("ни одна копия не прошла проверку целостности; последняя ошибка: {0}")]
    AllCorrupt(String),
    #[error(transparent)]
    Backup(#[from] BackupError),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("пакет переноса повреждён: {0}")]
    Zip(String),
    #[error("{0}")]
    Locked(crate::backup_file::UnpackError),
}

impl RestoreError {
    /// Копия зашифрована, а пароля нет или он неверный — спросить у владельца.
    pub fn needs_password(&self) -> bool {
        matches!(self, RestoreError::Locked(_))
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> RestoreError + '_ {
    move |source| RestoreError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Что подготовлено к восстановлению.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Staged {
    /// Имя копии и её время (мс) — «данные на 13.11.2026 14:00».
    pub name: String,
    pub at: i64,
    pub attachments_copied: usize,
}

/// Подготовить восстановление: `source` — файл копии, папка копий или пакет переноса.
/// `password` — пароль копий, если они зашифрованы на другом компьютере.
pub async fn stage(
    storage: &Storage,
    source: &Path,
    password: Option<&str>,
) -> Result<Staged, RestoreError> {
    std::fs::create_dir_all(&storage.db_dir).map_err(io(&storage.db_dir))?;
    let is_pack = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("crmpack"));
    if is_pack {
        let unpacked = storage.db_dir.join("restore-pack");
        let _ = std::fs::remove_dir_all(&unpacked);
        unzip(source, &unpacked)?;
        let r = stage_dir(storage, &unpacked, password).await;
        let _ = std::fs::remove_dir_all(&unpacked);
        return r;
    }
    if source.is_dir() {
        return stage_dir(storage, source, password).await;
    }
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let at = crm_core::backup::parse_file_name(&name)
        .ok_or_else(|| RestoreError::Unknown(source.to_owned()))?;
    stage_file(storage, source, password).await?;
    let dir = source.parent().unwrap_or(Path::new("."));
    let attachments_copied = bring_along(storage, dir)?;
    Ok(Staged {
        name,
        at: at.timestamp_millis(),
        attachments_copied,
    })
}

/// Из папки копий — самая свежая целая.
async fn stage_dir(
    storage: &Storage,
    dir: &Path,
    password: Option<&str>,
) -> Result<Staged, RestoreError> {
    let mut files = backup::list_dir(dir)?;
    if files.is_empty() {
        return Err(RestoreError::NoBackups(dir.to_owned()));
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.at));
    let mut last_err = String::new();
    for f in files {
        match stage_file(storage, &dir.join(&f.name), password).await {
            Ok(()) => {
                let attachments_copied = bring_along(storage, dir)?;
                return Ok(Staged {
                    name: f.name,
                    at: f.at,
                    attachments_copied,
                });
            }
            // Пароль не подошёл — более старые копии зашифрованы так же.
            Err(e) if e.needs_password() => return Err(e),
            Err(e) => {
                tracing::warn!(file = %f.name, error = %e, "копия не подходит, беру предыдущую");
                last_err = e.to_string();
            }
        }
    }
    Err(RestoreError::AllCorrupt(last_err))
}

/// Распаковать копию рядом с базой и проверить её. Файл копии — недоверенный:
/// чужие `-wal`/`-shm` рядом с распакованной базой удаляются до открытия.
async fn stage_file(
    storage: &Storage,
    file: &Path,
    password: Option<&str>,
) -> Result<(), RestoreError> {
    let staged = storage.db_dir.join(STAGED);
    let tmp = storage.db_dir.join(format!("{STAGED}.tmp"));
    for p in [&tmp, &sidecar(&tmp, "-wal"), &sidecar(&tmp, "-shm")] {
        let _ = std::fs::remove_file(p);
    }
    unpack(storage, file, &tmp, password)?;
    if let Err(e) = backup::check_integrity(&tmp).await {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    for p in [sidecar(&tmp, "-wal"), sidecar(&tmp, "-shm")] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::rename(&tmp, &staged).map_err(io(&staged))?;
    Ok(())
}

/// Распаковать копию (потоком) — ключом этого сервера или паролем.
pub(crate) fn unpack(
    storage: &Storage,
    file: &Path,
    to: &Path,
    password: Option<&str>,
) -> Result<(), RestoreError> {
    use crate::backup_file::{BackupKey, UnpackError};
    let local = BackupKey::load(&storage.db_dir).map_err(io(&storage.db_dir))?;
    match crate::backup_file::unpack(file, to, local.as_ref(), password) {
        Ok(()) => Ok(()),
        Err(UnpackError::Io(e)) => Err(io(file)(e)),
        Err(e @ (UnpackError::NeedPassword | UnpackError::WrongPassword)) => {
            let _ = std::fs::remove_file(to);
            Err(RestoreError::Locked(e))
        }
        Err(UnpackError::NotBackup) => {
            let _ = std::fs::remove_file(to);
            Err(RestoreError::Unknown(file.to_owned()))
        }
    }
}

/// Ключи сервера и вложения из папки копии — чтобы сотрудники не переподключались.
fn bring_along(storage: &Storage, dir: &Path) -> Result<usize, RestoreError> {
    backup::copy_tls(&dir.join("tls"), &storage.db_dir.join("tls"))?;
    let p2p = dir.join(backup::P2P_KEY);
    if p2p.exists() {
        std::fs::copy(&p2p, storage.db_dir.join(backup::P2P_KEY)).map_err(io(&p2p))?;
    }
    Ok(backup::copy_new_files(
        &dir.join("attachments"),
        &storage.attachments_dir,
    )?)
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// При запуске: подготовленная копия встаёт на место рабочей базы,
/// прежняя сохраняется как `before_restore_*.db`.
pub fn apply_staged(storage: &Storage) -> Result<Option<PathBuf>, RestoreError> {
    let staged = storage.db_dir.join(STAGED);
    if !staged.exists() {
        return Ok(None);
    }
    let db = storage.db_dir.join(DB);
    let keep = storage.db_dir.join(format!(
        "before_restore_{}.db",
        chrono::Utc::now().format("%Y_%m_%d_%H_%M_%S")
    ));
    if db.exists() {
        std::fs::rename(&db, &keep).map_err(io(&db))?;
        for suffix in ["-wal", "-shm"] {
            let from = sidecar(&db, suffix);
            if from.exists() {
                std::fs::rename(&from, sidecar(&keep, suffix)).map_err(io(&from))?;
            }
        }
    }
    std::fs::rename(&staged, &db).map_err(io(&staged))?;
    tracing::info!(previous = %keep.display(), "база восстановлена из копии");
    Ok(Some(keep))
}

/// Пакет переноса: последняя копия, ключи сервера и вложения — один файл для флешки.
pub fn pack(backup_dir: &Path, out: &Path) -> Result<PathBuf, RestoreError> {
    use std::io::Write;
    let mut files = backup::list_dir(backup_dir)?;
    files.sort_by_key(|f| std::cmp::Reverse(f.at));
    let newest = files
        .first()
        .ok_or_else(|| RestoreError::NoBackups(backup_dir.to_owned()))?;
    std::fs::create_dir_all(out).map_err(io(out))?;
    let path = out.join(format!(
        "crm_move_{}.crmpack",
        chrono::Utc::now().format("%Y_%m_%d_%H_%M")
    ));
    let zerr = |e: zip::result::ZipError| RestoreError::Zip(e.to_string());
    let f = std::fs::File::create(&path).map_err(io(&path))?;
    let mut z = zip::ZipWriter::new(f);
    // Копия уже сжата zstd — второй раз не жмём.
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut add = |name: String, src: &Path| -> Result<(), RestoreError> {
        z.start_file(name, stored).map_err(zerr)?;
        let mut r = std::fs::File::open(src).map_err(io(src))?;
        std::io::copy(&mut r, &mut z).map_err(io(src))?;
        Ok(())
    };
    add(newest.name.clone(), &backup_dir.join(&newest.name))?;
    let p2p = backup_dir.join(backup::P2P_KEY);
    if p2p.exists() {
        add(backup::P2P_KEY.to_owned(), &p2p)?;
    }
    for sub in ["tls", "attachments"] {
        for (rel, abs) in walk(&backup_dir.join(sub)) {
            add(format!("{sub}/{rel}"), &abs)?;
        }
    }
    z.finish().map_err(zerr)?.flush().map_err(io(&path))?;
    Ok(path)
}

/// Файлы папки (рекурсивно) с путями относительно неё через «/».
fn walk(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            out.extend(
                walk(&p)
                    .into_iter()
                    .map(|(r, a)| (format!("{name}/{r}"), a)),
            );
        } else {
            out.push((name, p));
        }
    }
    out
}

/// Распаковка пакета. Имена внутри — недоверенные: только безопасные относительные пути.
pub(crate) fn unzip(pack: &Path, to: &Path) -> Result<(), RestoreError> {
    let f = std::fs::File::open(pack).map_err(io(pack))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| RestoreError::Zip(e.to_string()))?;
    for i in 0..z.len() {
        let mut entry = z
            .by_index(i)
            .map_err(|e| RestoreError::Zip(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(RestoreError::Zip(
                "недопустимое имя файла внутри пакета".to_owned(),
            ));
        };
        let dest = to.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(io(&dest))?;
            continue;
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p).map_err(io(p))?;
        }
        let mut out = std::fs::File::create(&dest).map_err(io(&dest))?;
        std::io::copy(&mut entry, &mut out).map_err(io(&dest))?;
    }
    Ok(())
}
