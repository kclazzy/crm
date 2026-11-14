//! «Мои серверы»: что пользователь добавил по ключу. Файл в папке приложения.

use std::path::{Path, PathBuf};

use crm_core::connkey::{decode, normalize_fp, KeyError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// Отпечаток CA — один сервер, даже если сменил адреса.
    pub id: String,
    pub name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub fp: String,
    /// Адрес, который ответил в прошлый раз, — пробуем первым.
    #[serde(default)]
    pub last_addr: Option<String>,
    /// Постоянный локальный порт туннеля: вход в CRM сохраняется между запусками.
    pub local_port: u16,
    #[serde(default)]
    pub user: Option<String>,
}

impl Server {
    pub fn addresses_in_order(&self) -> Vec<String> {
        let mut out: Vec<String> = self.last_addr.iter().cloned().collect();
        for a in &self.addrs {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
        out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Key(#[from] KeyError),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("список серверов повреждён: {0}")]
    Corrupt(String),
    #[error("сервер не найден")]
    NotFound,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    servers: Vec<Server>,
}

pub struct Store {
    path: PathBuf,
}

/// Результат добавления: сервер и, если ключ был для нового сотрудника, приглашение.
#[derive(Debug, Clone, Serialize)]
pub struct Added {
    pub server: Server,
    pub invite: Option<String>,
    /// Такой сервер уже был — обновлены адреса.
    pub updated: bool,
}

/// Порт для туннеля — постоянный для сервера (по отпечатку), чтобы адрес окна не менялся.
fn port_for(id: &str, taken: &[u16]) -> u16 {
    let h = id
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
    let mut p = 47_100 + (h % 800) as u16;
    while taken.contains(&p) {
        p = if p >= 47_899 { 47_100 } else { p + 1 };
    }
    p
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn io<'a>(&'a self, p: &'a Path) -> impl Fn(std::io::Error) -> StoreError + 'a {
        move |source| StoreError::Io {
            path: p.to_owned(),
            source,
        }
    }

    pub fn list(&self) -> Result<Vec<Server>, StoreError> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str::<File>(&s)
                .map(|f| f.servers)
                .map_err(|e| StoreError::Corrupt(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(self.io(&self.path)(e)),
        }
    }

    fn save(&self, servers: Vec<Server>) -> Result<(), StoreError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(self.io(dir))?;
        }
        let json = serde_json::to_string_pretty(&File { servers })
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(self.io(&tmp))?;
        std::fs::rename(&tmp, &self.path).map_err(self.io(&self.path))
    }

    /// Добавить сервер по ключу. Повторный ключ того же сервера обновляет адреса.
    pub fn add_key(&self, key: &str, pin: Option<&str>) -> Result<Added, StoreError> {
        let k = decode(key, pin)?;
        self.add(Server {
            id: normalize_fp(&k.fp),
            name: k.name,
            addrs: k.addrs,
            port: k.port,
            fp: k.fp,
            last_addr: None,
            local_port: 0,
            user: k.user,
        })
        .map(|(server, updated)| Added {
            server,
            invite: k.invite,
            updated,
        })
    }

    /// Добавить или обновить сервер (свой компьютер владельца — тоже так).
    pub fn add(&self, mut s: Server) -> Result<(Server, bool), StoreError> {
        let mut all = self.list()?;
        let updated = if let Some(old) = all.iter_mut().find(|x| x.id == s.id) {
            old.addrs = s.addrs;
            old.port = s.port;
            old.name = s.name;
            if s.user.is_some() {
                old.user = s.user;
            }
            s = old.clone();
            true
        } else {
            let taken: Vec<u16> = all.iter().map(|x| x.local_port).collect();
            s.local_port = port_for(&s.id, &taken);
            all.push(s.clone());
            false
        };
        self.save(all)?;
        Ok((s, updated))
    }

    pub fn remember_addr(&self, id: &str, addr: &str) -> Result<(), StoreError> {
        let mut all = self.list()?;
        let s = all
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or(StoreError::NotFound)?;
        if s.last_addr.as_deref() != Some(addr) {
            s.last_addr = Some(addr.to_owned());
            self.save(all)?;
        }
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), StoreError> {
        let mut all = self.list()?;
        let s = all
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or(StoreError::NotFound)?;
        s.name = name.trim().chars().take(60).collect();
        self.save(all)
    }

    pub fn remove(&self, id: &str) -> Result<(), StoreError> {
        let mut all = self.list()?;
        let before = all.len();
        all.retain(|s| s.id != id);
        if all.len() == before {
            return Err(StoreError::NotFound);
        }
        self.save(all)
    }

    pub fn get(&self, id: &str) -> Result<Server, StoreError> {
        self.list()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or(StoreError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crm_core::connkey::{encode, ServerKey};

    fn key(fp: &str, addrs: &[&str], invite: Option<&str>) -> String {
        encode(
            &ServerKey {
                name: "Офис".into(),
                addrs: addrs.iter().map(|a| (*a).to_owned()).collect(),
                port: 8443,
                fp: fp.into(),
                invite: invite.map(Into::into),
                user: Some("ivan".into()),
            },
            Some("1234"),
            [3; 40],
        )
        .unwrap()
    }

    #[test]
    fn same_server_is_updated_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("servers.json"));
        let a = store
            .add_key(
                &key("AA:BB", &["pc", "192.168.1.2"], Some("inv")),
                Some("1234"),
            )
            .unwrap();
        assert!(!a.updated);
        assert_eq!(a.invite.as_deref(), Some("inv"));
        assert!((47_100..47_900).contains(&a.server.local_port));
        store.remember_addr(&a.server.id, "192.168.1.2").unwrap();

        let b = store
            .add_key(&key("aabb", &["pc", "10.0.0.9"], None), Some("1234"))
            .unwrap();
        assert!(b.updated);
        assert_eq!(
            b.server.local_port, a.server.local_port,
            "порт тот же — вход сохранится"
        );
        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(
            all[0].addresses_in_order(),
            ["192.168.1.2", "pc", "10.0.0.9"]
        );

        assert!(matches!(
            store.add_key(&key("cc", &[], None), Some("0000")),
            Err(StoreError::Key(_))
        ));
        store.remove(&a.server.id).unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn ports_do_not_clash() {
        let p = port_for("x", &[]);
        assert_ne!(port_for("x", &[p]), p);
    }
}
