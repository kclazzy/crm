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
const CHECK_TIMEOUT: Duration = Duration::from_secs(6);

/// Узлы живут, пока работает программа: соединение переиспользуется окном и проверками.
fn dialers() -> &'static Mutex<HashMap<String, Arc<Dialer>>> {
    static ALL: OnceLock<Mutex<HashMap<String, Arc<Dialer>>>> = OnceLock::new();
    ALL.get_or_init(Default::default)
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
    Some(match tokio::time::timeout(CHECK_TIMEOUT, d.check()).await {
        Ok(Ok(Path::None)) | Err(_) => Err("узел сервера не отвечает".into()),
        Ok(Ok(p)) => Ok(p),
        Ok(Err(e)) => Err(e.to_string()),
    })
}

/// Путь внутри сети офиса (а не через интернет или ретранслятор).
pub fn in_office(path: &Path) -> bool {
    matches!(path, Path::Direct { addr, .. } if crm_core::remote::is_office_addr(&addr.ip().to_string()))
}
