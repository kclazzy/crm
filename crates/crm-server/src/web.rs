//! Раздача веб-интерфейса (встроен в сервер) и заголовки безопасности для всех ответов.

use axum::body::Body;
use axum::http::{header, HeaderValue, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../web/dist"]
struct Assets;

/// Политика: только свои скрипты и стили, без встраивания в чужие сайты (ТЗ §3.2.1).
const CSP: &str = "default-src 'self'; img-src 'self' data:; connect-src 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'";

/// Страница или файл интерфейса; неизвестный путь вне /api — index.html (маршруты SPA).
pub async fn static_or_index(uri: Uri) -> axum::response::Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "не найдено" })),
        )
            .into_response();
    }
    match Assets::get(path).filter(|_| !path.is_empty()) {
        Some(file) => {
            let immutable = path.starts_with("assets/");
            respond(file, immutable)
        }
        None => match Assets::get("index.html") {
            Some(index) => respond(index, false),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    }
}

fn respond(file: rust_embed::EmbeddedFile, immutable: bool) -> axum::response::Response {
    let mime = file.metadata.mimetype().to_owned();
    let cache = if immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut res = Response::new(Body::from(file.data.into_owned()));
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

/// Заголовки безопасности на каждый ответ — и страницы, и API.
pub async fn security_headers(mut res: axum::response::Response) -> axum::response::Response {
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    res
}
