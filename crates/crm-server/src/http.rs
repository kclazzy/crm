//! HTTP-маршруты, состояние приложения и единый тип ошибки API.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::{auth, secrets};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
}

pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/invites/accept", post(auth::accept_invite))
        .route("/api/me", get(auth::me))
        .route("/api/admin/users", post(auth::create_user))
        .route("/api/admin/users/{id}/reset", post(auth::reset_access))
        .route("/api/admin/sessions", get(auth::list_sessions))
        .route("/api/admin/sessions/{id}", delete(auth::revoke_session))
        .with_state(AppState { pool })
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health(State(app): State<AppState>) -> Result<Json<Health>, ApiError> {
    sqlx::query("SELECT 1").execute(&app.pool).await?;
    Ok(Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
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
    #[error("недостаточно прав")]
    Forbidden,
    #[error("не найдено")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("слишком много неудачных попыток, попробуйте через 15 минут")]
    TooManyAttempts,
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
            Self::Unauthorized | Self::BadCredentials | Self::BadInvite => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
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
