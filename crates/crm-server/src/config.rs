//! Конфигурация сервера (`config.toml`). Разбирается один раз при старте;
//! дальше код работает с уже проверенными типами.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default)]
    pub tls: Tls,
    #[serde(default)]
    pub storage: Storage,
    /// Расписание и сроки хранения резервных копий (выбирает владелец).
    #[serde(default)]
    pub backup: crm_core::backup::Policy,
    /// Подключение по ключу узла: пользоваться ли ретрансляторами в интернете.
    /// `false` — только прямые соединения (закрытая сеть, тесты).
    #[serde(default = "yes")]
    pub p2p_relay: bool,
    /// Поиск сервера программой в сети офиса (UDP).
    #[serde(default)]
    pub discovery: Discovery,
    /// Откуда прочитан конфиг — туда же сохраняются изменения из админ-панели.
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    /// Выключать можно только для сервера, слушающего лишь 127.0.0.1 (разработка).
    pub enabled: bool,
}

impl Default for Tls {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Где хранятся данные (ТЗ §2.4). БД — только на локальном диске.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    pub db_dir: PathBuf,
    pub attachments_dir: PathBuf,
    #[serde(default)]
    pub backup_dirs: Vec<PathBuf>,
}

impl Default for Storage {
    fn default() -> Self {
        let root = default_data_root();
        Self {
            db_dir: root.join("data"),
            attachments_dir: root.join("attachments"),
            backup_dirs: Vec::new(),
        }
    }
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            tls: Tls::default(),
            storage: Storage::default(),
            backup: crm_core::backup::Policy::default(),
            p2p_relay: true,
            discovery: Discovery::default(),
            source: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("ошибка в config.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("без HTTPS сервер может слушать только 127.0.0.1, а указан {0}: включите [tls] enabled = true")]
    InsecureListen(SocketAddr),
    #[error("не удалось записать {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("не удалось сохранить настройки: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("не удалось прочитать {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let cfg: Self = toml::from_str(s)?;
        if !cfg.tls.enabled && !cfg.listen.ip().is_loopback() {
            return Err(ConfigError::InsecureListen(cfg.listen));
        }
        Ok(cfg)
    }

    /// Читает файл; если его нет — настройки по умолчанию (первый запуск).
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s).map(|c| Self {
                source: Some(path.to_owned()),
                ..c
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                source: Some(path.to_owned()),
                ..Self::default()
            }),
            Err(source) => Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }
}

/// По умолчанию — все сетевые интерфейсы; это допустимо, потому что TLS включён
/// по умолчанию, а без TLS конфиг с сетевым адресом не принимается.
impl Config {
    /// Сохранить настройки (через временный файл — сбой не оставит битый config.toml).
    pub fn save(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        let text = format!(
            "# Настройки сервера CRM. Изменяются из админ-панели; править вручную — при остановленном сервере.\n{}",
            toml::to_string_pretty(self)?
        );
        let tmp = path.with_extension("toml.tmp");
        let io = |source| ConfigError::Write {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Discovery {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// UDP-порт; 0 — любой свободный (тесты).
    #[serde(default = "discovery_port")]
    pub port: u16,
}

impl Default for Discovery {
    fn default() -> Self {
        Self {
            enabled: true,
            port: discovery_port(),
        }
    }
}

fn discovery_port() -> u16 {
    crm_core::discovery::PORT
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 8443))
}

/// Где лежит config.toml службы: в корне папки данных.
pub fn default_config_path() -> PathBuf {
    default_data_root().join("config.toml")
}

/// Корень данных по умолчанию: `C:\ProgramData\CRM`, `/var/lib/crm`, `/Library/Application Support/CRM`.
pub fn default_data_root() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
            .join("CRM")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/CRM")
    } else {
        PathBuf::from("/var/lib/crm")
    }
}
