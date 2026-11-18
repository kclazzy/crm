//! Связь с сервером на этом компьютере: состояние службы, проверка здоровья,
//! действия с правами администратора (через запрос UAC).

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::status::{Health, Service};

pub const PORT: u16 = 8443;

fn base() -> String {
    format!("https://127.0.0.1:{PORT}")
}

/// Клиент только для 127.0.0.1: сертификат не проверяется, потому что соединение
/// не выходит за пределы компьютера (подменить его по сети нельзя), а папка с CA
/// закрыта для обычных пользователей.
fn loopback_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct HealthBody {
    status: String,
    ca_fingerprint: Option<String>,
}

#[derive(Deserialize)]
struct SetupBody {
    needs_owner: bool,
}

pub async fn health() -> Health {
    let Ok(client) = loopback_client() else {
        return Health::Unreachable;
    };
    let h: Option<HealthBody> = async {
        client
            .get(format!("{}/api/health", base()))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()
    }
    .await;
    let Some(h) = h.filter(|h| h.status == "ok") else {
        return Health::Unreachable;
    };
    let needs_owner = async {
        client
            .get(format!("{}/api/setup/status", base()))
            .send()
            .await
            .ok()?
            .json::<SetupBody>()
            .await
            .ok()
    }
    .await
    .is_some_and(|s| s.needs_owner);
    Health::Ok {
        needs_owner,
        ca_fingerprint: h.ca_fingerprint,
    }
}

#[derive(Deserialize)]
struct SetupOut {
    ticket: String,
}

/// Одноразовый билет входа: сам билет остаётся в программе, серверу передаётся
/// только его SHA-256 (как `crm_server::secrets::token_hash`).
pub fn new_ticket() -> Result<(String, String), String> {
    use sha2::Digest;
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| format!("генератор случайных чисел: {e}"))?;
    let ticket: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let hash = format!("{:x}", sha2::Sha256::digest(ticket.as_bytes()));
    Ok((ticket, hash))
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// Создать владельца. Сервер разрешает это только с этого же компьютера.
/// Возвращает билет, чтобы открыть CRM сразу со входом.
pub async fn create_owner(
    username: String,
    display_name: String,
    password: String,
) -> Result<String, String> {
    let res = loopback_client()?
        .post(format!("{}/api/setup", base()))
        .json(&serde_json::json!({
            "username": username, "display_name": display_name, "password": password
        }))
        .send()
        .await
        .map_err(|_| "сервер не отвечает".to_owned())?;
    if res.status().is_success() {
        return res
            .json::<SetupOut>()
            .await
            .map(|o| o.ticket)
            .map_err(|_| "владелец создан, но сервер не прислал билет входа".to_owned());
    }
    let msg = res
        .json::<ErrorBody>()
        .await
        .map_or_else(|_| "не удалось создать владельца".to_owned(), |b| b.error);
    Err(msg)
}

/// Папка данных прежнего сервера, если она осталась (C:\\ProgramData\\CRM). Внутрь
/// обычный пользователь заглянуть не может — права только у администраторов, —
/// но саму папку видно. Её создаёт только установка сервера.
pub fn old_data_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let root = PathBuf::from(std::env::var_os("ProgramData")?).join("CRM");
        root.exists().then_some(root)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// crm-server.exe лежит рядом с приложением (так его кладёт установщик).
fn server_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("нет папки приложения")?;
    Ok(dir.join(if cfg!(windows) {
        "crm-server.exe"
    } else {
        "crm-server"
    }))
}

#[cfg(windows)]
mod platform {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use super::{server_exe, Service};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

    /// Запускается ли служба вместе с Windows (`None` — службы нет).
    pub fn service_autostart() -> Option<bool> {
        let out = Command::new("sc.exe")
            .args(["qc", "CRMServer"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // AUTO_START / DEMAND_START sc.exe тоже не переводит.
        Some(String::from_utf8_lossy(&out.stdout).contains("AUTO_START"))
    }

    pub fn service_state() -> Service {
        let Ok(out) = Command::new("sc.exe")
            .args(["query", "CRMServer"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        else {
            return Service::Unknown;
        };
        if out.status.code() == Some(ERROR_SERVICE_DOES_NOT_EXIST) {
            return Service::NotInstalled;
        }
        // Слова RUNNING/STOPPED sc.exe не переводит ни в одной локализации.
        let text = String::from_utf8_lossy(&out.stdout);
        if text.contains("RUNNING") {
            Service::Running
        } else if text.contains("STOPPED") {
            Service::Stopped
        } else {
            Service::Unknown
        }
    }

    /// Сброс пароля владельца: окно PowerShell от имени администратора с командой
    /// `crm-server reset-owner-password` — пароль вводится в нём, окно остаётся открытым,
    /// чтобы был виден результат. Права проверяет сама Windows (окно UAC).
    pub fn reset_owner_password_console() -> Result<(), String> {
        let exe = server_exe()?;
        let inner = format!(
            "& '{}' reset-owner-password",
            exe.display().to_string().replace('\'', "''")
        );
        let script = format!(
            "try {{ Start-Process -FilePath powershell.exe -Verb RunAs -ArgumentList '-NoExit','-NoProfile','-Command',\"{}\" }} catch {{ exit 1223 }}",
            inner.replace('"', "`\"")
        );
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
        match status.code() {
            Some(0) => Ok(()),
            _ => Err("действие отменено в окне контроля учётных записей".into()),
        }
    }

    /// `crm-server service <action>` с повышением прав (окно UAC). Ждём завершения.
    pub fn elevated(action: &str) -> Result<(), String> {
        run_elevated(&["service", action])
    }

    /// Билет входа владельцу: `crm-server owner-ticket <хеш>` от администратора.
    pub fn issue_owner_ticket(hash: &str) -> Result<(), String> {
        run_elevated(&["owner-ticket", hash])
    }

    /// `crm-server <args>` с повышением прав (окно UAC). Аргументы — только наши
    /// константы и hex, кавычки в них не попадают.
    fn run_elevated(args: &[&str]) -> Result<(), String> {
        let exe = server_exe()?;
        let list = args
            .iter()
            .map(|a| format!("'{a}'"))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "try {{ $p = Start-Process -FilePath '{}' -ArgumentList {list} -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode }} catch {{ exit 1223 }}",
            exe.display().to_string().replace('\'', "''"),
        );
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
        match status.code() {
            Some(0) => Ok(()),
            Some(1223) => Err("действие отменено в окне контроля учётных записей".into()),
            _ => Err(
                "не удалось выполнить действие, подробности — в журнале C:\\ProgramData\\CRM\\logs"
                    .into(),
            ),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Service;

    pub fn service_state() -> Service {
        Service::Unknown
    }

    pub fn service_autostart() -> Option<bool> {
        None
    }

    pub fn reset_owner_password_console() -> Result<(), String> {
        Err("сброс пароля владельца из программы есть только в Windows; используйте команду crm-server reset-owner-password".into())
    }

    pub fn elevated(action: &str) -> Result<(), String> {
        let _ = super::server_exe;
        Err(format!(
            "«{action}»: управление службой есть только в Windows"
        ))
    }

    pub fn issue_owner_ticket(_hash: &str) -> Result<(), String> {
        Err("вход без пароля из программы есть только в Windows".into())
    }
}

pub use platform::{
    elevated, issue_owner_ticket, reset_owner_password_console, service_autostart, service_state,
};

/// Действия, которые окно и трей могут попросить у службы.
pub fn is_allowed_action(action: &str) -> bool {
    matches!(
        action,
        "install" | "install-fresh" | "uninstall" | "start" | "stop" | "restart"
    )
}

/// Открыть адрес в браузере по умолчанию (для телефона — тот же адрес в сети).
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Адреса этого компьютера в локальной сети — их сотрудники вводят для подключения.
pub fn lan_addresses() -> Vec<String> {
    let mut out: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(v4) if !v4.is_link_local() => Some(format!("https://{v4}:{PORT}")),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod ticket_tests {
    use super::*;

    #[test]
    fn ticket_hash_is_what_the_server_expects() {
        use sha2::Digest;
        let (ticket, hash) = new_ticket().unwrap();
        assert_eq!(ticket.len(), 64);
        assert_eq!(
            hash,
            format!("{:x}", sha2::Sha256::digest(ticket.as_bytes()))
        );
        assert!(hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_ne!(new_ticket().unwrap().0, ticket, "каждый раз новый");
    }

    #[test]
    fn fresh_install_is_allowed_from_the_window() {
        assert!(is_allowed_action("install-fresh"));
        assert!(
            is_allowed_action("uninstall"),
            "«этот компьютер — не сервер»"
        );
        assert!(!is_allowed_action("owner-ticket"));
    }
}
