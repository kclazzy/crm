//! Разбор командной строки `crm-server`.

use std::path::PathBuf;

pub const USAGE: &str = "\
Использование:
  crm-server [--config ПУТЬ]        запустить сервер в этом окне
  crm-server service install        установить и запустить службу Windows (от администратора)
  crm-server service uninstall      остановить и удалить службу (данные сохраняются)
  crm-server service start|stop|restart|status
  crm-server reset-owner-password   сбросить забытый пароль владельца (от администратора, на сервере)
  crm-server restore ИСТОЧНИК       восстановить данные: файл копии .db.zst, папка копий
                                    или пакет переноса .crmpack (затем перезапустить службу)
  crm-server --version";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run {
        config: Option<PathBuf>,
    },
    Service(ServiceCmd),
    /// Сброс пароля владельца прямо в базе (нужны права на папку данных).
    ResetOwnerPassword {
        config: Option<PathBuf>,
    },
    Restore {
        source: PathBuf,
        config: Option<PathBuf>,
    },
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCmd {
    Install,
    Uninstall,
    Start,
    Stop,
    Restart,
    Status,
    /// Вызывается диспетчером служб Windows, не человеком.
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("непонятная команда: {0}\n\n{USAGE}")]
pub struct UsageError(String);

pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let bad = || UsageError(words.join(" "));
    match words.as_slice() {
        [] => Ok(Command::Run { config: None }),
        ["--config", path] => Ok(Command::Run {
            config: Some(PathBuf::from(path)),
        }),
        ["--version" | "-V"] => Ok(Command::Version),
        ["reset-owner-password"] => Ok(Command::ResetOwnerPassword { config: None }),
        ["reset-owner-password", "--config", path] => Ok(Command::ResetOwnerPassword {
            config: Some(PathBuf::from(path)),
        }),
        ["restore", source] => Ok(Command::Restore {
            source: PathBuf::from(source),
            config: None,
        }),
        ["restore", source, "--config", path] => Ok(Command::Restore {
            source: PathBuf::from(source),
            config: Some(PathBuf::from(path)),
        }),
        ["service", sub] => {
            let cmd = match *sub {
                "install" => ServiceCmd::Install,
                "uninstall" => ServiceCmd::Uninstall,
                "start" => ServiceCmd::Start,
                "stop" => ServiceCmd::Stop,
                "restart" => ServiceCmd::Restart,
                "status" => ServiceCmd::Status,
                "run" => ServiceCmd::Run,
                _ => return Err(bad()),
            };
            Ok(Command::Service(cmd))
        }
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Command, UsageError> {
        let args: Vec<String> = s.split_whitespace().map(String::from).collect();
        parse(&args)
    }

    #[test]
    fn runs_in_foreground_by_default() {
        assert_eq!(p(""), Ok(Command::Run { config: None }));
        assert_eq!(
            p("--config C:/crm/c.toml"),
            Ok(Command::Run {
                config: Some("C:/crm/c.toml".into())
            })
        );
        assert!(p("--config").is_err());
    }

    #[test]
    fn service_subcommands() {
        assert_eq!(
            p("service install"),
            Ok(Command::Service(ServiceCmd::Install))
        );
        assert_eq!(
            p("service uninstall"),
            Ok(Command::Service(ServiceCmd::Uninstall))
        );
        assert_eq!(
            p("service restart"),
            Ok(Command::Service(ServiceCmd::Restart))
        );
        assert_eq!(
            p("service status"),
            Ok(Command::Service(ServiceCmd::Status))
        );
        assert_eq!(p("service run"), Ok(Command::Service(ServiceCmd::Run)));
        assert!(p("service").is_err());
        assert!(p("service install extra").is_err());
        assert!(p("service fly").is_err());
    }

    #[test]
    fn restore_command() {
        assert_eq!(
            p("restore E:/crm-backup"),
            Ok(Command::Restore {
                source: "E:/crm-backup".into(),
                config: None
            })
        );
        assert!(p("restore").is_err());
    }

    #[test]
    fn owner_password_reset_command() {
        assert_eq!(
            p("reset-owner-password"),
            Ok(Command::ResetOwnerPassword { config: None })
        );
        assert_eq!(
            p("reset-owner-password --config D:/crm.toml"),
            Ok(Command::ResetOwnerPassword {
                config: Some("D:/crm.toml".into())
            })
        );
        assert!(p("reset-owner-password now").is_err());
    }

    #[test]
    fn version_and_garbage() {
        assert_eq!(p("--version"), Ok(Command::Version));
        let err = p("--frobnicate").unwrap_err().to_string();
        assert!(err.contains("Использование"));
    }
}
