//! Выбор папки на сервере (копии, перенос данных): диски, вложенные папки, «Создать папку».
//! Папки — на компьютере с CRM: туда пишет служба сервера, а не браузер владельца.

use std::path::{Path, PathBuf};

use axum::extract::Query;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::Owner;
use crate::http::ApiError;

const MAX_ENTRIES: usize = 500;

#[derive(Deserialize)]
pub struct ListQuery {
    path: Option<String>,
}

#[derive(Serialize)]
pub struct Entry {
    name: String,
    path: PathBuf,
}

#[derive(Serialize)]
pub struct Listing {
    /// Открытая папка; `None` — список дисков.
    path: Option<PathBuf>,
    /// Папка выше; `None` у диска — «Диски».
    parent: Option<PathBuf>,
    entries: Vec<Entry>,
}

/// Диски этого компьютера (Windows) или корень файловой системы.
fn roots() -> Vec<Entry> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|l| format!("{}:\\", char::from(l)))
            .filter(|d| Path::new(d).exists())
            .map(|d| Entry {
                name: d.clone(),
                path: PathBuf::from(d),
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![Entry {
            name: "/".into(),
            path: PathBuf::from("/"),
        }]
    }
}

/// Служебные папки Windows и скрытые — в выборе не нужны.
fn hidden(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('$')
        || [
            "System Volume Information",
            "Recovery",
            "PerfLogs",
            "Config.Msi",
        ]
        .contains(&name)
}

pub async fn list(Owner(_): Owner, Query(q): Query<ListQuery>) -> Result<Json<Listing>, ApiError> {
    let Some(raw) = q.path.filter(|p| !p.trim().is_empty()) else {
        return Ok(Json(Listing {
            path: None,
            parent: None,
            entries: roots(),
        }));
    };
    let dir = PathBuf::from(raw.trim());
    if !dir.is_absolute() {
        return Err(ApiError::BadRequest("нужен полный путь".into()));
    }
    let read = std::fs::read_dir(&dir)
        .map_err(|e| ApiError::BadRequest(format!("не открыть папку {}: {e}", dir.display())))?;
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (!hidden(&name)).then(|| Entry {
                name,
                path: e.path(),
            })
        })
        .collect();
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries.truncate(MAX_ENTRIES);
    Ok(Json(Listing {
        parent: dir.parent().map(Path::to_path_buf),
        path: Some(dir),
        entries,
    }))
}

#[derive(Deserialize)]
pub struct MkdirIn {
    parent: PathBuf,
    name: String,
}

pub async fn mkdir(Owner(_): Owner, Json(req): Json<MkdirIn>) -> Result<Json<Entry>, ApiError> {
    let name = req.name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(ApiError::BadRequest(
            "имя папки без символов / \\ : * ? \" < > |".into(),
        ));
    }
    if !req.parent.is_absolute() {
        return Err(ApiError::BadRequest("нужен полный путь".into()));
    }
    let path = req.parent.join(name);
    std::fs::create_dir_all(&path)
        .map_err(|e| ApiError::BadRequest(format!("не создать папку {}: {e}", path.display())))?;
    Ok(Json(Entry {
        name: name.to_owned(),
        path,
    }))
}
