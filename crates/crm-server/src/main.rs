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
    #[error(transparent)]
    Reset(#[from] crm_server::recovery::ResetError),
    #[error("{0}")]
    Input(String),
    #[error(transparent)]
    Restore(#[from] crm_server::restore::RestoreError),
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
        Command::ResetOwnerPassword { config } => reset_owner(config),
        Command::Restore { source, config } => restore(source, config),
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

fn config_location(config_path: Option<PathBuf>) -> PathBuf {
    config_path
        .or_else(|| std::env::var_os("CRM_CONFIG").map(PathBuf::from))
        .unwrap_or_else(config::default_config_path)
}

/// Сброс забытого пароля владельца на самом сервере. Запускать от администратора:
/// иначе Windows не даст открыть папку с базой.
#[tokio::main]
async fn reset_owner(config_path: Option<PathBuf>) -> Result<(), MainError> {
    let cfg = config::Config::load(&config_location(config_path))?;
    println!(
        "Сброс пароля владельца CRM. Данные: {}",
        cfg.storage.db_dir.display()
    );
    let ask = |prompt: &str| {
        rpassword::prompt_password(prompt)
            .map_err(|e| MainError::Input(format!("не удалось прочитать пароль: {e}")))
    };
    let first = ask("Новый пароль (не короче 10 символов): ")?;
    let second = ask("Повторите пароль: ")?;
    if first != second {
        return Err(MainError::Input(
            "пароли не совпадают — ничего не изменено".into(),
        ));
    }
    let username = crm_server::recovery::reset_owner_password(&cfg.storage, &first).await?;
    println!("Готово. Пароль владельца «{username}» изменён, все его входы завершены.");
    Ok(())
}

/// Восстановление на этом компьютере: копия готовится и встанет на место при запуске.
#[tokio::main]
async fn restore(source: PathBuf, config_path: Option<PathBuf>) -> Result<(), MainError> {
    let cfg = config::Config::load(&config_location(config_path))?;
    let staged = crm_server::restore::stage(&cfg.storage, &source).await?;
    let at = chrono::DateTime::from_timestamp_millis(staged.at)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    println!(
        "Копия {} проверена (данные на {at}), файлов вложений перенесено: {}.\n\
         Перезапустите сервер (меню трея → «Перезапустить сервер» или `crm-server service restart`) —\n\
         копия встанет на место, прежняя база сохранится как before_restore_*.db.",
        staged.name, staged.attachments_copied
    );
    Ok(())
}

#[tokio::main]
async fn run_foreground(config_path: Option<PathBuf>) -> Result<(), MainError> {
    logging::to_console();
    let path = config_location(config_path);
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
