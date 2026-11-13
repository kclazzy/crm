//! Точка входа: `crm-server [--config PATH]`.

use std::path::PathBuf;

use crm_server::{config, db, http};

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Db(#[from] db::DbError),
    #[error("не удалось открыть порт: {0}")]
    Io(#[from] std::io::Error),
    #[error("неизвестный аргумент «{0}». Использование: crm-server [--config PATH]")]
    Args(String),
}

fn config_path() -> Result<PathBuf, MainError> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (None, _) => Ok(std::env::var_os("CRM_CONFIG")
            .map_or_else(|| PathBuf::from("config.toml"), PathBuf::from)),
        (Some("--config"), Some(p)) => Ok(PathBuf::from(p)),
        (Some(other), _) => Err(MainError::Args(other.to_owned())),
    }
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = config::Config::load(&config_path()?)?;
    let pool = db::open(&cfg.storage).await?;
    let listener = tokio::net::TcpListener::bind(cfg.listen).await?;
    tracing::info!(addr = %listener.local_addr()?, db = %cfg.storage.db_dir.display(), "CRM запущена");

    axum::serve(listener, http::router(pool))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("остановка");
        })
        .await?;
    Ok(())
}
