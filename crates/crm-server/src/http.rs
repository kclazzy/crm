//! HTTP-маршруты и единый тип ошибки API.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .with_state(pool)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health(State(pool): State<SqlitePool>) -> Result<Json<Health>, ApiError> {
    sqlx::query("SELECT 1").execute(&pool).await?;
    Ok(Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("ошибка базы данных")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Db(e) => {
                tracing::error!(error = %e, "ошибка БД");
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
