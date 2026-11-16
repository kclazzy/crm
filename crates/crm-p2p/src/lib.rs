//! Подключение к серверу CRM «по ключу узла», а не по адресу (прототип).
//!
//! Сервер и программа сотрудника поднимают по узлу iroh (QUIC поверх обычного UDP,
//! без сетевых адаптеров и прав администратора). Программа открывает
//! `127.0.0.1:ПОРТ`; каждое соединение окна идёт отдельным потоком QUIC к серверу,
//! а тот передаёт его в CRM. HTTP, файлы и WebSocket идут без изменений.
//!
//! Один и тот же способ работает везде:
//! - в офисе — напрямую по адресам из ключа, интернет не нужен;
//! - удалённо — пробитие NAT, а если не вышло — через ретранслятор (трафик зашифрован).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{presets, Connection, RelayMode};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Имя протокола: по одному потоку на каждое соединение с CRM.
pub const ALPN: &[u8] = b"crm/http/1";

/// Сколько ждать соединение с узлом (прямое или через ретранслятор).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("не удалось запустить узел: {0}")]
    Bind(String),
    #[error("ключ узла неверный")]
    BadId,
    #[error("узел сервера не отвечает: {0}")]
    Connect(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Пользоваться ли ретрансляторами (нужен интернет) или только прямыми адресами.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relay {
    Off,
    Public,
}

async fn endpoint(key: Option<[u8; 32]>, udp_port: u16, relay: Relay) -> Result<Endpoint, Error> {
    let bind = |e: &dyn std::fmt::Display| Error::Bind(e.to_string());
    let mut b = match relay {
        Relay::Off => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
        Relay::Public => Endpoint::builder(presets::N0),
    };
    if let Some(k) = key {
        b = b
            .secret_key(SecretKey::from_bytes(&k))
            .alpns(vec![ALPN.to_vec()]);
    }
    b.bind_addr(SocketAddr::from(([0, 0, 0, 0], udp_port)))
        .map_err(|e| bind(&e))?
        .bind()
        .await
        .map_err(|e| bind(&e))
}

/// Узел сервера: принимает соединения и передаёт потоки в CRM.
pub struct Host {
    endpoint: Endpoint,
    task: JoinHandle<()>,
}

impl Host {
    /// `key` — постоянный секрет узла (хранится на сервере); `udp_port` — 0 или
    /// постоянный порт, чтобы в офисе подключаться напрямую; `crm` — куда передавать.
    pub async fn start(
        key: [u8; 32],
        udp_port: u16,
        crm: SocketAddr,
        relay: Relay,
    ) -> Result<Self, Error> {
        let endpoint = endpoint(Some(key), udp_port, relay).await?;
        let ep = endpoint.clone();
        let task = tokio::spawn(async move {
            while let Some(incoming) = ep.accept().await {
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        serve(conn, crm).await;
                    }
                });
            }
        });
        Ok(Self { endpoint, task })
    }

    /// Публичный ключ узла — его и кладём в ключ подключения.
    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }

    pub fn udp_port(&self) -> u16 {
        self.endpoint
            .bound_sockets()
            .iter()
            .find(|a| a.is_ipv4())
            .map_or(0, SocketAddr::port)
    }

    pub async fn stop(self) {
        self.endpoint.close().await;
        self.task.abort();
    }
}

async fn serve(conn: Connection, crm: SocketAddr) {
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        tokio::spawn(async move {
            let Ok(mut tcp) = TcpStream::connect(crm).await else {
                return;
            };
            let (mut r, mut w) = tcp.split();
            let up = async {
                let _ = tokio::io::copy(&mut recv, &mut w).await;
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut w).await;
            };
            let down = async {
                let _ = tokio::io::copy(&mut r, &mut send).await;
                let _ = send.finish();
            };
            tokio::join!(up, down);
        });
    }
}

/// Как найти узел сервера.
#[derive(Debug, Clone)]
pub struct Dial {
    /// Публичный ключ узла сервера.
    pub id: String,
    /// Прямые адреса (офис): работают без интернета.
    pub direct: Vec<SocketAddr>,
    pub relay: Relay,
}

/// Каким путём сейчас идёт трафик.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Path {
    /// Напрямую: в сети офиса или через пробитый NAT.
    Direct {
        addr: SocketAddr,
        rtt_ms: u64,
    },
    /// Через ретранслятор — медленнее, но работает за любым роутером.
    Relayed {
        rtt_ms: u64,
    },
    None,
}

struct Link {
    endpoint: Endpoint,
    addr: EndpointAddr,
    conn: Mutex<Option<Connection>>,
}

impl Link {
    /// Живое соединение; оборвалось (сервер перезапустили) — подключаемся заново.
    async fn conn(&self) -> Result<Connection, Error> {
        let mut slot = self.conn.lock().await;
        if let Some(c) = slot.as_ref().filter(|c| c.close_reason().is_none()) {
            return Ok(c.clone());
        }
        let c = tokio::time::timeout(
            CONNECT_TIMEOUT,
            self.endpoint.connect(self.addr.clone(), ALPN),
        )
        .await
        .map_err(|_| Error::Connect("нет ответа".into()))?
        .map_err(|e| Error::Connect(e.to_string()))?;
        *slot = Some(c.clone());
        Ok(c)
    }

    async fn carry(&self, mut tcp: TcpStream) -> Result<(), Error> {
        let mut conn = self.conn().await?;
        let (mut send, mut recv) = match conn.open_bi().await {
            Ok(s) => s,
            // Соединение умерло между проверкой и открытием потока — одна новая попытка.
            Err(_) => {
                self.conn.lock().await.take();
                conn = self.conn().await?;
                conn.open_bi()
                    .await
                    .map_err(|e| Error::Connect(e.to_string()))?
            }
        };
        let (mut r, mut w) = tcp.split();
        let up = async {
            let _ = tokio::io::copy(&mut r, &mut send).await;
            let _ = send.finish();
        };
        let down = async {
            let _ = tokio::io::copy(&mut recv, &mut w).await;
            let _ = tokio::io::AsyncWriteExt::shutdown(&mut w).await;
        };
        tokio::join!(up, down);
        Ok(())
    }
}

/// Сторона программы сотрудника: локальный порт → узел сервера.
pub struct Client {
    link: Arc<Link>,
    port: u16,
    task: JoinHandle<()>,
}

impl Client {
    /// Открыть локальный порт (`0` — любой). Соединение с узлом устанавливается
    /// при первом запросе, так что открыть можно и без сети.
    pub async fn open(local_port: u16, dial: Dial) -> Result<Self, Error> {
        let id: EndpointId = dial.id.parse().map_err(|_| Error::BadId)?;
        let addr = dial
            .direct
            .iter()
            .fold(EndpointAddr::new(id), |a, d| a.with_ip_addr(*d));
        let link = Arc::new(Link {
            endpoint: endpoint(None, 0, dial.relay).await?,
            addr,
            conn: Mutex::new(None),
        });
        let listener = TcpListener::bind(("127.0.0.1", local_port)).await?;
        let port = listener.local_addr()?.port();
        let l = link.clone();
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let l = l.clone();
                tokio::spawn(async move {
                    let _ = l.carry(tcp).await;
                });
            }
        });
        Ok(Self { link, port, task })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Подключиться сейчас и сказать, каким путём пошло (для «Проверить соединение»).
    pub async fn check(&self) -> Result<Path, Error> {
        self.link.conn().await?;
        Ok(self.path().await)
    }

    pub async fn path(&self) -> Path {
        let slot = self.link.conn.lock().await;
        let Some(conn) = slot.as_ref().filter(|c| c.close_reason().is_none()) else {
            return Path::None;
        };
        let paths = conn.paths();
        let Some(p) = paths.into_iter().find(|p| p.is_selected()) else {
            return Path::None;
        };
        let rtt_ms = p.rtt().as_millis() as u64;
        match p.remote_addr() {
            iroh::TransportAddr::Ip(addr) => Path::Direct {
                addr: *addr,
                rtt_ms,
            },
            _ => Path::Relayed { rtt_ms },
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}
