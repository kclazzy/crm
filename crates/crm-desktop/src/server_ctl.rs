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
struct ErrorBody {
    error: String,
}

/// Создать владельца. Сервер разрешает это только с этого же компьютера.
pub async fn create_owner(
    username: String,
    display_name: String,
    password: String,
) -> Result<(), String> {
    let res = loopback_client()?
        .post(format!("{}/api/setup", base()))
        .json(&serde_json::json!({
            "username": username, "display_name": display_name, "password": password
        }))
        .send()
        .await
        .map_err(|_| "сервер не отвечает".to_owned())?;
    if res.status().is_success() {
        return Ok(());
    }
    let msg = res
        .json::<ErrorBody>()
        .await
        .map_or_else(|_| "не удалось создать владельца".to_owned(), |b| b.error);
    Err(msg)
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

    /// `crm-server service <action>` с повышением прав (окно UAC). Ждём завершения.
    pub fn elevated(action: &str) -> Result<(), String> {
        let exe = server_exe()?;
        let script = format!(
            "try {{ $p = Start-Process -FilePath '{}' -ArgumentList 'service','{}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode }} catch {{ exit 1223 }}",
            exe.display().to_string().replace('\'', "''"),
            action
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

    pub fn elevated(action: &str) -> Result<(), String> {
        let _ = super::server_exe;
        Err(format!(
            "«{action}»: управление службой есть только в Windows"
        ))
    }
}

pub use platform::{elevated, service_state};

/// Действия, которые окно и трей могут попросить у службы.
pub fn is_allowed_action(action: &str) -> bool {
    matches!(action, "install" | "start" | "stop" | "restart")
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
