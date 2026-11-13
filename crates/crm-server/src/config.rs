//! Конфигурация сервера (`config.toml`). Разбирается один раз при старте;
//! дальше код работает с уже проверенными типами.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default)]
    pub storage: Storage,
}

/// Где хранятся данные (ТЗ §2.4). БД — только на локальном диске.
#[derive(Debug, Clone, Deserialize)]
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

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            storage: Storage::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("ошибка в config.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("не удалось прочитать {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(s)?)
    }

    /// Читает файл; если его нет — настройки по умолчанию (первый запуск).
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 8443))
}

fn default_data_root() -> PathBuf {
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
