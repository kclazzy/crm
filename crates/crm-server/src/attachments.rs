//! Вложения: файл хранится один раз по sha256 (ТЗ §4.7).

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::access::Scope;
use crate::auth::{now_ms, CurrentUser};
use crate::http::{ApiError, AppState};

/// Самый большой файл (ТЗ: по умолчанию 100 МБ).
pub const MAX_FILE: usize = 100 * 1024 * 1024;

/// Имя файла без путей и управляющих символов — только для показа и скачивания.
pub fn clean_name(s: &str) -> String {
    let base = s.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '<' | '>' | ':' | '|' | '?' | '*'))
        .take(200)
        .collect();
    let t = cleaned.trim().trim_matches('.').to_owned();
    if t.is_empty() {
        "файл".into()
    } else {
        t
    }
}

fn is_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn path_for(dir: &std::path::Path, sha: &str) -> std::path::PathBuf {
    dir.join(&sha[..2]).join(sha)
}

/// Типы, которые браузер может показать сам; остальное — только скачивание.
fn safe_mime(m: &str) -> Option<&'static str> {
    Some(match m {
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "application/pdf" => "application/pdf",
        _ => return None,
    })
}

fn sniff_mime(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "doc" | "docx" | "xls" | "xlsx" | "zip" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

#[derive(Serialize)]
pub struct Uploaded {
    sha256: String,
    size_bytes: i64,
    mime: String,
    file_name: String,
}

/// Загрузка файла потоком: `POST /api/attachments`, имя — в заголовке `X-File-Name` (URL-кодированное).
pub async fn upload(
    State(app): State<AppState>,
    _: CurrentUser,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Uploaded>), ApiError> {
    let raw_name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("файл");
    let decoded = percent_decode(raw_name);
    let file_name = clean_name(&decoded);
    let dir = app.rt.config().storage.attachments_dir;
    let tmp_dir = dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| ApiError::Failed(format!("папка вложений: {e}")))?;
    let tmp = tmp_dir.join(crate::auth::new_id());
    let result = receive(body, &tmp).await;
    let (sha, size) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };
    let dest = path_for(&dir, &sha);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Failed(format!("папка вложений: {e}")))?;
    }
    if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&tmp).await;
    } else {
        tokio::fs::rename(&tmp, &dest)
            .await
            .map_err(|e| ApiError::Failed(format!("сохранение файла: {e}")))?;
    }
    let mime = sniff_mime(&file_name).to_owned();
    let size_bytes = i64::try_from(size).unwrap_or(i64::MAX);
    sqlx::query(
        "INSERT INTO attachments(sha256, size_bytes, mime, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(sha256) DO UPDATE SET created_at = excluded.created_at",
    )
    .bind(&sha)
    .bind(size_bytes)
    .bind(&mime)
    .bind(now_ms())
    .execute(&app.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(Uploaded {
            sha256: sha,
            size_bytes,
            mime,
            file_name,
        }),
    ))
}

async fn receive(body: Body, tmp: &std::path::Path) -> Result<(String, usize), ApiError> {
    let io = |e: std::io::Error| ApiError::Failed(format!("запись файла: {e}"));
    let mut file = tokio::fs::File::create(tmp).await.map_err(io)?;
    let mut hasher = Sha256::new();
    let mut size = 0usize;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ApiError::BadRequest("загрузка прервана".into()))?;
        size += chunk.len();
        if size > MAX_FILE {
            return Err(ApiError::BadRequest("файл больше 100 МБ".into()));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(io)?;
    }
    if size == 0 {
        return Err(ApiError::BadRequest("пустой файл".into()));
    }
    file.flush().await.map_err(io)?;
    file.sync_all().await.map_err(io)?;
    let sha: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((sha, size))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        if b == b'%' {
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    #[serde(default)]
    download: bool,
}

/// Скачать файл можно, только если он прикреплён к сообщению в доступной задаче.
pub async fn download(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(sha): Path<String>,
    Query(q): Query<DownloadQuery>,
) -> Result<Response, ApiError> {
    if !is_sha(&sha) {
        return Err(ApiError::NotFound);
    }
    let scope = Scope::load(&app.pool, &user).await?;
    let uses: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT m.task_id, ma.file_name, a.mime FROM message_attachments ma
         JOIN messages m ON m.id = ma.message_id JOIN attachments a ON a.sha256 = ma.sha256
         WHERE ma.sha256 = ?",
    )
    .bind(&sha)
    .fetch_all(&app.pool)
    .await?;
    let (_, name, mime) = uses
        .into_iter()
        .find(|(t, _, _)| scope.task(t).can_read())
        .ok_or(ApiError::NotFound)?;
    let path = path_for(&app.rt.config().storage.attachments_dir, &sha);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let mut res = Response::new(Body::from_stream(tokio_util::io::ReaderStream::new(file)));
    let h = res.headers_mut();
    let shown = safe_mime(&mime).filter(|_| !q.download);
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(shown.unwrap_or("application/octet-stream")),
    );
    if let Ok(v) = HeaderValue::from_str(&len.to_string()) {
        h.insert(header::CONTENT_LENGTH, v);
    }
    let disposition = if shown.is_some() {
        "inline"
    } else {
        "attachment"
    };
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    if let Ok(v) = HeaderValue::from_str(&format!("{disposition}; filename*=UTF-8''{encoded}")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    // Файл не может выполнять скрипты, даже если это HTML под видом картинки.
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox",
        ),
    );
    Ok(res)
}

/// Удалить с диска файлы, на которые не ссылается ни одно сообщение
/// (загруженные, но не отправленные — через сутки).
pub async fn collect_garbage(app: &AppState) -> Result<usize, ApiError> {
    let day_ago = now_ms() - 24 * 3600 * 1000;
    let orphans: Vec<String> = sqlx::query_scalar(
        "SELECT sha256 FROM attachments a
         WHERE NOT EXISTS (SELECT 1 FROM message_attachments ma WHERE ma.sha256 = a.sha256)
           AND (a.created_at < ? OR NOT EXISTS (SELECT 1 FROM message_attachments))",
    )
    .bind(day_ago)
    .fetch_all(&app.pool)
    .await?;
    let dir = app.rt.config().storage.attachments_dir;
    let mut removed = 0;
    for sha in orphans {
        if !is_sha(&sha) {
            continue;
        }
        sqlx::query("DELETE FROM attachments WHERE sha256 = ? AND NOT EXISTS (SELECT 1 FROM message_attachments WHERE sha256 = ?)")
            .bind(&sha)
            .bind(&sha)
            .execute(&app.pool)
            .await?;
        match tokio::fs::remove_file(path_for(&dir, &sha)).await {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(%sha, error = %e, "не удалось удалить файл"),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_paths_and_dangerous_chars() {
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("C:\\Users\\a\\счёт.pdf"), "счёт.pdf");
        assert_eq!(clean_name("a\"b<c>.txt"), "abc.txt");
        assert_eq!(clean_name("..."), "файл");
        assert_eq!(clean_name(""), "файл");
    }

    #[test]
    fn percent_decoding_handles_cyrillic_and_garbage() {
        assert_eq!(percent_decode("%D1%81%D1%87%D1%91%D1%82.pdf"), "счёт.pdf");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn only_lowercase_hex_sha_accepted() {
        assert!(is_sha(&"a".repeat(64)));
        assert!(!is_sha(&"A".repeat(64)));
        assert!(!is_sha("../x"));
    }
}
