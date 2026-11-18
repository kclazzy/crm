//! Разбор командной строки `crm-server`.

use std::path::PathBuf;

pub const USAGE: &str = "\
Использование:
  crm-server [--config ПУТЬ]        запустить сервер в этом окне
  crm-server service install        установить и запустить службу Windows (от администратора)
  crm-server service uninstall      остановить и удалить службу (данные сохраняются)
  crm-server service park           для установщика: удалить службу, запомнив её и автозапуск
  crm-server service unpark         для установщика: вернуть запомненную службу (если была)
                                    и убрать туннель WireGuard прежних версий
  crm-server service start|stop|restart|status
  crm-server service autostart-on   запускать сервер при включении компьютера (по умолчанию)
  crm-server service autostart-off  не запускать сам — только вручную
  crm-server service install-fresh  установить службу с чистой базой: прежние база и вложения
                                    переносятся в папку old-ДАТА рядом с данными
  crm-server reset-owner-password   сбросить забытый пароль владельца (от администратора, на сервере)
  crm-server owner-ticket ХЕШ       для программы на сервере: билет входа владельца без пароля
                                    (от администратора; ХЕШ — SHA-256 билета в hex)
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
    /// Билет входа владельца без пароля (выдаёт администратор Windows).
    OwnerTicket {
        hash: String,
        config: Option<PathBuf>,
    },
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCmd {
    Install,
    /// Установка с чистой базой: прежняя переносится в архивную папку.
    InstallFresh,
    Uninstall,
    Start,
    Stop,
    Restart,
    Status,
    /// Вызывается диспетчером служб Windows, не человеком.
    Run,
    /// Запуск службы вместе с Windows: да или нет.
    Autostart(bool),
    /// Деинсталлятор: удалить службу, запомнив, что она была (и её автозапуск).
    Park,
    /// Установщик: вернуть службу, если её запомнили при удалении.
    Unpark,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("непонятная команда: {0}\n\n{USAGE}")]
pub struct UsageError(String);

/// SHA-256 в hex строчными буквами — так его пишет `secrets::token_hash`.
fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

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
        ["owner-ticket", hash] if is_hash(hash) => Ok(Command::OwnerTicket {
            hash: (*hash).to_owned(),
            config: None,
        }),
        ["owner-ticket", hash, "--config", path] if is_hash(hash) => Ok(Command::OwnerTicket {
            hash: (*hash).to_owned(),
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
                "install-fresh" => ServiceCmd::InstallFresh,
                "uninstall" => ServiceCmd::Uninstall,
                "start" => ServiceCmd::Start,
                "stop" => ServiceCmd::Stop,
                "restart" => ServiceCmd::Restart,
                "status" => ServiceCmd::Status,
                "run" => ServiceCmd::Run,
                "autostart-on" => ServiceCmd::Autostart(true),
                "autostart-off" => ServiceCmd::Autostart(false),
                "park" => ServiceCmd::Park,
                "unpark" => ServiceCmd::Unpark,
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
        assert_eq!(
            p("service autostart-off"),
            Ok(Command::Service(ServiceCmd::Autostart(false)))
        );
        assert!(p("service vpn-apply").is_err(), "VPN убран");
        assert_eq!(p("service park"), Ok(Command::Service(ServiceCmd::Park)));
        assert_eq!(
            p("service unpark"),
            Ok(Command::Service(ServiceCmd::Unpark))
        );
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
    fn owner_ticket_takes_only_a_hash() {
        let h = "ab".repeat(32);
        assert_eq!(
            p(&format!("owner-ticket {h}")),
            Ok(Command::OwnerTicket {
                hash: h.clone(),
                config: None
            })
        );
        assert_eq!(
            p(&format!("owner-ticket {h} --config D:/crm.toml")),
            Ok(Command::OwnerTicket {
                hash: h.clone(),
                config: Some("D:/crm.toml".into())
            })
        );
        // Сам билет в командную строку не попадает — только SHA-256 в hex.
        assert!(p("owner-ticket").is_err());
        assert!(p("owner-ticket билет").is_err());
        assert!(p(&format!("owner-ticket {}", "AB".repeat(32))).is_err());
        assert!(p(&format!("owner-ticket {}", "ab".repeat(31))).is_err());
    }

    #[test]
    fn fresh_install_is_a_service_command() {
        assert_eq!(
            p("service install-fresh"),
            Ok(Command::Service(ServiceCmd::InstallFresh))
        );
    }

    #[test]
    fn version_and_garbage() {
        assert_eq!(p("--version"), Ok(Command::Version));
        let err = p("--frobnicate").unwrap_err().to_string();
        assert!(err.contains("Использование"));
    }
}
