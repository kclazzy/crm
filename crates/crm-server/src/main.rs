//! Точка входа `crm-server`: запуск сервера или управление службой Windows.

use std::path::PathBuf;

use crm_server::cli::{self, Command, ServiceCmd};
use crm_server::{config, logging, server};

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)]
    Usage(#[from] cli::UsageError),
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Run(#[from] server::RunError),
    #[cfg(windows)]
    #[error(transparent)]
    Service(#[from] crm_server::winsvc::ServiceError),
    #[cfg(not(windows))]
    #[error("службы Windows доступны только в Windows; в Linux используйте packaging/linux/crm-server.service")]
    NotWindows,
}

fn main() -> Result<(), MainError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args)? {
        Command::Version => {
            println!("crm-server {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Run { config } => run_foreground(config),
        Command::Service(cmd) => service(cmd),
    }
}

#[cfg(windows)]
fn service(cmd: ServiceCmd) -> Result<(), MainError> {
    let message = crm_server::winsvc::handle(cmd)?;
    if !message.is_empty() {
        println!("{message}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn service(_cmd: ServiceCmd) -> Result<(), MainError> {
    Err(MainError::NotWindows)
}

#[tokio::main]
async fn run_foreground(config_path: Option<PathBuf>) -> Result<(), MainError> {
    logging::to_console();
    let path = config_path
        .or_else(|| std::env::var_os("CRM_CONFIG").map(PathBuf::from))
        .unwrap_or_else(config::default_config_path);
    let cfg = config::Config::load(&path)?;
    let running = server::start(&cfg).await?;
    let scheme = if cfg.tls.enabled { "https" } else { "http" };
    tracing::info!(
        addr = %format!("{scheme}://{}", running.addr),
        ca = running.ca_fingerprint.as_deref().unwrap_or("—"),
        config = %path.display(),
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
