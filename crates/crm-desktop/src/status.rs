//! Состояние сервера для трея и окна — чистая функция от фактов (ТЗ §2.5.1).

use serde::Serialize;

/// Состояние службы Windows на этом компьютере.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    NotInstalled,
    Stopped,
    Running,
    /// Не удалось узнать (нет прав, не Windows).
    Unknown,
}

/// Что ответил сервер на 127.0.0.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Ok {
        needs_owner: bool,
        ca_fingerprint: Option<String>,
    },
    Unreachable,
}

/// Цвет точки на иконке трея.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    Ok,
    Warn,
    Error,
    Offline,
}

/// Что показать пользователю.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub light: Light,
    /// Короткая фраза для подсказки трея и строки меню.
    pub text: String,
    pub service: Service,
    pub reachable: bool,
    pub needs_owner: bool,
    pub ca_fingerprint: Option<String>,
}

pub fn summarize(service: Service, health: &Health) -> Status {
    let (light, text) = match (service, health) {
        (
            _,
            Health::Ok {
                needs_owner: true, ..
            },
        ) => (Light::Warn, "Сервер работает · создайте владельца"),
        (_, Health::Ok { .. }) => (Light::Ok, "Сервер работает"),
        (Service::NotInstalled, Health::Unreachable) => {
            (Light::Offline, "Сервер на этом компьютере не установлен")
        }
        (Service::Stopped, Health::Unreachable) => (Light::Error, "Сервер остановлен"),
        (Service::Running, Health::Unreachable) => (Light::Error, "Сервер не отвечает"),
        (Service::Unknown, Health::Unreachable) => (Light::Offline, "Нет связи с сервером"),
    };
    let (reachable, needs_owner, ca_fingerprint) = match health {
        Health::Ok {
            needs_owner,
            ca_fingerprint,
        } => (true, *needs_owner, ca_fingerprint.clone()),
        Health::Unreachable => (false, false, None),
    };
    Status {
        light,
        text: text.to_owned(),
        service,
        reachable,
        needs_owner,
        ca_fingerprint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(needs_owner: bool) -> Health {
        Health::Ok {
            needs_owner,
            ca_fingerprint: Some("AB:CD".into()),
        }
    }

    #[test]
    fn working_server_is_green_and_asks_for_owner_in_yellow() {
        let s = summarize(Service::Running, &ok(false));
        assert_eq!(
            (s.light, s.reachable, s.ca_fingerprint.as_deref()),
            (Light::Ok, true, Some("AB:CD"))
        );
        let s = summarize(Service::Running, &ok(true));
        assert_eq!(s.light, Light::Warn);
        assert!(s.needs_owner);
    }

    #[test]
    fn stopped_or_silent_server_is_red() {
        assert_eq!(
            summarize(Service::Stopped, &Health::Unreachable).light,
            Light::Error
        );
        let s = summarize(Service::Running, &Health::Unreachable);
        assert_eq!(
            (s.light, s.text.as_str()),
            (Light::Error, "Сервер не отвечает")
        );
    }

    #[test]
    fn computer_without_server_is_grey() {
        assert_eq!(
            summarize(Service::NotInstalled, &Health::Unreachable).light,
            Light::Offline
        );
        assert_eq!(
            summarize(Service::Unknown, &Health::Unreachable).light,
            Light::Offline
        );
    }

    #[test]
    fn answer_from_server_wins_over_service_query() {
        // sc.exe мог не ответить, а сервер (например, запущенный вручную) работает.
        assert_eq!(summarize(Service::Unknown, &ok(false)).light, Light::Ok);
    }
}
