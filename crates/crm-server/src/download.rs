//! Программа CRM с самого сервера: установщик при установке кладёт свою копию
//! `CRM-Setup.exe` рядом с `crm-server.exe`, и любой компьютер офиса скачивает её
//! в браузере — без флешки и интернета. Скачать можно без входа: программа нужна
//! как раз тому, у кого ещё нет пароля.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{header, HeaderValue, Response};
use axum::Json;
use serde::Serialize;

use crate::http::ApiError;

pub const FILE: &str = "CRM-Setup.exe";

/// Где лежит копия установщика (`CRM_INSTALLER` — для проверки).
fn installer() -> Option<PathBuf> {
    let path = match std::env::var_os("CRM_INSTALLER") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe().ok()?.parent()?.join(FILE),
    };
    path.is_file().then_some(path)
}

#[derive(Serialize)]
pub struct Info {
    available: bool,
    size: Option<u64>,
}

pub async fn info() -> Json<Info> {
    let size = installer()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len());
    Json(Info {
        available: size.is_some(),
        size,
    })
}

pub async fn file() -> Result<Response<Body>, ApiError> {
    let path = installer().ok_or(ApiError::NotFound)?;
    let f = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
    let mut res = Response::new(Body::from_stream(tokio_util::io::ReaderStream::new(f)));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.microsoft.portable-executable"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"CRM-Setup.exe\""),
    );
    if let Ok(v) = HeaderValue::from_str(&len.to_string()) {
        h.insert(header::CONTENT_LENGTH, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}
