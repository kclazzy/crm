//! HTTP-маршруты, состояние приложения и единый тип ошибки API.

use axum::extract::State;
use std::sync::Arc;

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::{auth, secrets};

/// Изменяемые на лету настройки и режимы сервера.
pub struct Runtime {
    /// Куда сохранять изменения настроек (config.toml).
    pub config_path: Option<std::path::PathBuf>,
    pub config: std::sync::RwLock<crate::config::Config>,
    /// Данные перенесены в новое место — до перезапуска только чтение.
    pub frozen: std::sync::atomic::AtomicBool,
    /// Бэкапы идут строго по одному.
    pub backup_lock: tokio::sync::Mutex<()>,
}

impl Runtime {
    pub fn config(&self) -> crate::config::Config {
        match self.config.read() {
            Ok(c) => c.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub rt: Arc<Runtime>,
    /// Сертификат CA сервера (PEM) для установки в браузер; `None` — HTTPS выключен.
    pub ca_pem: Option<Arc<str>>,
    /// Отпечаток CA для сверки сотрудником/владельцем.
    pub ca_fingerprint: Option<Arc<str>>,
}

impl AppState {
    pub fn new(
        pool: SqlitePool,
        config: crate::config::Config,
        config_path: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            pool,
            rt: Arc::new(Runtime {
                config_path,
                config: std::sync::RwLock::new(config),
                frozen: std::sync::atomic::AtomicBool::new(false),
                backup_lock: tokio::sync::Mutex::new(()),
            }),
            ca_pem: None,
            ca_fingerprint: None,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/tls/ca", get(ca_certificate))
        .route("/api/setup", post(auth::setup))
        .route("/api/setup/status", get(auth::setup_status))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/recover", post(crate::recovery::recover))
        .route("/api/auth/forgot", post(crate::recovery::forgot))
        .route(
            "/api/admin/recovery-codes",
            post(crate::recovery::new_codes).get(crate::recovery::codes_status),
        )
        .route("/api/invites/accept", post(auth::accept_invite))
        .route("/api/me", get(auth::me))
        .route(
            "/api/admin/users",
            post(auth::create_user).get(auth::list_users),
        )
        .route("/api/admin/users/{id}/reset", post(auth::reset_access))
        .route("/api/admin/sessions", get(auth::list_sessions))
        .route("/api/admin/sessions/{id}", delete(auth::revoke_session))
        .route("/api/admin/server", get(crate::admin::server_info))
        .route("/api/admin/storage", get(crate::admin::storage))
        .route("/api/admin/storage/move", post(crate::admin::move_data))
        .route(
            "/api/admin/backup",
            axum::routing::put(crate::admin::set_backup_dirs),
        )
        .route("/api/admin/backup/run", post(crate::admin::backup_now))
        .route("/api/admin/backups", get(crate::admin::backups))
        .fallback(crate::web::static_or_index)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            frozen_guard,
        ))
        .layer(axum::middleware::map_response(crate::web::security_headers))
        .with_state(state)
}

/// После переноса данных до перезапуска разрешено только чтение — иначе новые записи
/// попали бы в старую базу и потерялись.
async fn frozen_guard(
    State(app): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let read = matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    );
    if !read && app.rt.frozen.load(std::sync::atomic::Ordering::SeqCst) {
        return ApiError::Frozen.into_response();
    }
    next.run(req).await
}

/// CA сервера — открытая информация (без ключа); нужна браузеру, чтобы доверять серверу.
async fn ca_certificate(State(app): State<AppState>) -> Result<Response, ApiError> {
    let pem = app.ca_pem.ok_or(ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"crm-ca.crt\"",
            ),
        ],
        pem.to_string(),
    )
        .into_response())
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    ca_fingerprint: Option<Arc<str>>,
}

async fn health(State(app): State<AppState>) -> Result<Json<Health>, ApiError> {
    sqlx::query("SELECT 1").execute(&app.pool).await?;
    Ok(Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        ca_fingerprint: app.ca_fingerprint,
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("требуется вход")]
    Unauthorized,
    /// Одинаковый ответ для «нет такого логина» и «неверный пароль».
    #[error("неверный логин или пароль")]
    BadCredentials,
    #[error("приглашение недействительно или истекло")]
    BadInvite,
    #[error("неверный логин или код восстановления")]
    BadCode,
    #[error("недостаточно прав")]
    Forbidden,
    #[error("не найдено")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("слишком много неудачных попыток, попробуйте через 15 минут")]
    TooManyAttempts,
    #[error("данные перенесены в новое место — перезапустите сервер (меню трея → «Перезапустить сервер»)")]
    Frozen,
    /// Понятная владельцу причина сбоя операции (бэкап, перенос).
    #[error("{0}")]
    Failed(String),
    #[error("внутренняя ошибка сервера")]
    Db(#[from] sqlx::Error),
    #[error("внутренняя ошибка сервера")]
    Secret(#[from] secrets::SecretError),
    #[error("внутренняя ошибка сервера")]
    Corrupt(String),
}

impl From<crm_core::auth::AccountError> for ApiError {
    fn from(e: crm_core::auth::AccountError) -> Self {
        Self::BadRequest(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::BadCredentials | Self::BadInvite | Self::BadCode => {
                StatusCode::UNAUTHORIZED
            }
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
            Self::Frozen => StatusCode::SERVICE_UNAVAILABLE,
            Self::Failed(why) => {
                tracing::error!(%why, "операция не выполнена");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Db(e) => {
                tracing::error!(error = %e, "ошибка БД");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Secret(e) => {
                tracing::error!(error = %e, "ошибка криптографии");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Corrupt(what) => {
                tracing::error!(%what, "некорректные данные в БД");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (
            status,
            Json(serde_json::json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}
