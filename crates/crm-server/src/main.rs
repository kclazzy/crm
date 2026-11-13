//! Точка входа: `crm-server [--config PATH]`.

use std::path::PathBuf;

use crm_server::{config, server};

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Run(#[from] server::RunError),
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
    let running = server::start(&cfg).await?;
    let scheme = if cfg.tls.enabled { "https" } else { "http" };
    tracing::info!(
        addr = %format!("{scheme}://{}", running.addr),
        ca = running.ca_fingerprint.as_deref().unwrap_or("—"),
        db = %cfg.storage.db_dir.display(),
        "CRM запущена"
    );
    running
        .run_until(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("остановка");
        })
        .await?;
    Ok(())
}
