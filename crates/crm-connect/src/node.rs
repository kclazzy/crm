//! Подключение по ключу узла: запасной путь, когда ни один адрес сервера не отвечает
//! (из дома без открытого порта на роутере, за чужим Wi-Fi, рядом с VPN).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crm_p2p::{Dial, Dialer, Path, Relay};
use tokio::sync::Mutex;

use crate::Server;

/// Сколько ждать узел при проверке: ретранслятор отвечает за секунду-две.
/// Проверка ждёт дольше самого соединения: оборвать долгое первое соединение
/// значит начинать каждый раз с нуля — и сервер «не отвечает» навсегда.
const CHECK_TIMEOUT: Duration = Duration::from_secs(crm_p2p::CONNECT_TIMEOUT.as_secs() + 5);

/// Узлы живут, пока работает программа: соединение переиспользуется окном и проверками.
fn dialers() -> &'static Mutex<HashMap<String, Arc<Dialer>>> {
    static ALL: OnceLock<Mutex<HashMap<String, Arc<Dialer>>>> = OnceLock::new();
    ALL.get_or_init(Default::default)
}

/// Постоянный секрет этого компьютера (лежит рядом со списком серверов).
static DEVICE_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Прочитать или создать секрет устройства в `dir`. Без него сервер видит программу
/// каждый раз как новое устройство, и одобрение владельца не запоминается.
pub fn use_device_key(dir: &std::path::Path) -> std::io::Result<()> {
    if DEVICE_KEY.get().is_some() {
        return Ok(());
    }
    let path = dir.join("device.key");
    let key = match std::fs::read(&path)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
    {
        Some(k) => k,
        None => {
            let k = crm_p2p::new_key();
            std::fs::create_dir_all(dir)?;
            std::fs::write(&path, k)?;
            k
        }
    };
    let _ = DEVICE_KEY.set(key);
    Ok(())
}

/// Имя этого устройства для сервера (`None` — секрет устройства не задан).
pub fn device_id() -> Option<String> {
    DEVICE_KEY.get().map(crm_p2p::public_id)
}

/// `CRM_P2P_RELAY=off` — только прямые соединения (закрытая сеть, тесты).
fn relay() -> Relay {
    match std::env::var("CRM_P2P_RELAY").as_deref() {
        Ok("off") => Relay::Off,
        _ => Relay::Public,
    }
}

/// Прямые адреса узла: офисные адреса сервера с UDP-портом узла.
async fn direct(server: &Server) -> Vec<SocketAddr> {
    let Some(udp) = server.udp else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for a in server.addresses_in_order() {
        if !crm_core::remote::is_office_addr(&a) {
            continue;
        }
        let found = tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::lookup_host((a.as_str(), udp)),
        )
        .await;
        if let Ok(Ok(addrs)) = found {
            out.extend(addrs.filter(SocketAddr::is_ipv4));
        }
    }
    out
}

/// Соединение с узлом сервера (`None` — в ключе нет узла: владелец не включал).
pub async fn dialer(server: &Server) -> Option<Result<Arc<Dialer>, crm_p2p::Error>> {
    let id = server.node.clone()?;
    let mut all = dialers().lock().await;
    if let Some(d) = all.get(&id) {
        return Some(Ok(d.clone()));
    }
    let made = Dialer::new(Dial {
        id: id.clone(),
        direct: direct(server).await,
        relay: relay(),
        key: DEVICE_KEY.get().copied(),
    })
    .await
    .map(Arc::new);
    if let Ok(d) = &made {
        all.insert(id, d.clone());
    }
    Some(made)
}

/// Отвечает ли узел и каким путём идёт связь.
pub async fn check(server: &Server) -> Option<Result<Path, String>> {
    let d = match dialer(server).await? {
        Ok(d) => d,
        Err(e) => return Some(Err(e.to_string())),
    };
    let found = match tokio::time::timeout(CHECK_TIMEOUT, d.check()).await {
        Ok(Ok(Path::None)) | Err(_) => Err("узел сервера не отвечает".into()),
        Ok(Ok(p)) => Ok(p),
        Ok(Err(e)) => Err(e.to_string()),
    };
    if found.is_err() {
        // Сеть могла смениться (сон, другой Wi-Fi): в следующий раз — новое соединение.
        let gone = match &server.node {
            Some(id) => dialers().lock().await.remove(id),
            None => None,
        };
        // Окно с CRM его не держит — закрыть узел, а не бросить открытым.
        if let Some(old) = gone.and_then(|d| Arc::try_unwrap(d).ok()) {
            old.close().await;
        }
    }
    Some(found)
}

/// Путь внутри сети офиса (а не через интернет или ретранслятор).
pub fn in_office(path: &Path) -> bool {
    matches!(path, Path::Direct { addr, .. } if crm_core::remote::is_office_addr(&addr.ip().to_string()))
}
