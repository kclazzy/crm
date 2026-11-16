//! Подключение к серверу CRM «по ключу узла», а не по адресу.
//!
//! Сервер и программа сотрудника поднимают по узлу iroh (QUIC поверх обычного UDP,
//! без сетевых адаптеров и прав администратора). Каждое соединение окна с CRM идёт
//! отдельным потоком QUIC; HTTP, файлы и WebSocket идут без изменений.
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

/// Новый случайный секрет узла.
pub fn new_key() -> [u8; 32] {
    SecretKey::generate().to_bytes()
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

/// Один поток от программы сотрудника: читать и писать как в обычный сокет.
pub type Stream = tokio::io::Join<iroh::endpoint::RecvStream, iroh::endpoint::SendStream>;

/// Кто на той стороне потока.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// Публичный ключ устройства сотрудника.
    pub id: String,
    /// Адрес, с которого сейчас идут пакеты; `None` — через ретранслятор.
    pub direct: Option<SocketAddr>,
}

type BoxFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
type Handler = Arc<dyn Fn(Peer, Stream) -> BoxFuture + Send + Sync>;

/// Узел сервера: принимает соединения и отдаёт потоки обработчику.
pub struct Host {
    endpoint: Endpoint,
    task: JoinHandle<()>,
}

impl Host {
    /// `key` — постоянный секрет узла (хранится на сервере); `udp_port` — 0 или
    /// постоянный порт; `handler` получает каждый поток (одно соединение с CRM).
    pub async fn serve<F, Fut>(
        key: [u8; 32],
        udp_port: u16,
        relay: Relay,
        handler: F,
    ) -> Result<Self, Error>
    where
        F: Fn(Peer, Stream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |p, s| Box::pin(handler(p, s)));
        let endpoint = endpoint(Some(key), udp_port, relay).await?;
        let ep = endpoint.clone();
        let task = tokio::spawn(async move {
            while let Some(incoming) = ep.accept().await {
                let handler = handler.clone();
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        accept_streams(conn, handler).await;
                    }
                });
            }
        });
        Ok(Self { endpoint, task })
    }

    /// Узел, который передаёт каждый поток на TCP-адрес (проверочная программа, тесты).
    pub async fn start(
        key: [u8; 32],
        udp_port: u16,
        crm: SocketAddr,
        relay: Relay,
    ) -> Result<Self, Error> {
        Self::serve(key, udp_port, relay, move |_, mut stream| async move {
            if let Ok(mut tcp) = TcpStream::connect(crm).await {
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
            }
        })
        .await
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

/// Выбранный сейчас путь: прямой адрес и задержка либо только задержка (ретранслятор).
fn selected(conn: &Connection) -> Option<(Option<SocketAddr>, u64)> {
    let paths = conn.paths();
    let p = paths.into_iter().find(|p| p.is_selected())?;
    let rtt = p.rtt().as_millis() as u64;
    Some(match p.remote_addr() {
        iroh::TransportAddr::Ip(addr) => (Some(*addr), rtt),
        _ => (None, rtt),
    })
}

async fn accept_streams(conn: Connection, handler: Handler) {
    let id = conn.remote_id().to_string();
    while let Ok((send, recv)) = conn.accept_bi().await {
        let peer = Peer {
            id: id.clone(),
            direct: selected(&conn).and_then(|(a, _)| a),
        };
        tokio::spawn(handler(peer, tokio::io::join(recv, send)));
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

/// Соединение с узлом сервера: открывает потоки, само переподключается.
pub struct Dialer {
    endpoint: Endpoint,
    addr: EndpointAddr,
    conn: Mutex<Option<Connection>>,
}

impl Dialer {
    /// Сети может ещё не быть: соединение устанавливается при первом потоке.
    pub async fn new(dial: Dial) -> Result<Self, Error> {
        let id: EndpointId = dial.id.parse().map_err(|_| Error::BadId)?;
        let addr = dial
            .direct
            .iter()
            .fold(EndpointAddr::new(id), |a, d| a.with_ip_addr(*d));
        Ok(Self {
            endpoint: endpoint(None, 0, dial.relay).await?,
            addr,
            conn: Mutex::new(None),
        })
    }

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

    /// Новый поток к серверу (одно соединение окна с CRM).
    pub async fn stream(&self) -> Result<Stream, Error> {
        let conn = self.conn().await?;
        let (send, recv) = match conn.open_bi().await {
            Ok(s) => s,
            // Соединение умерло между проверкой и открытием потока — одна новая попытка.
            Err(_) => {
                self.conn.lock().await.take();
                self.conn()
                    .await?
                    .open_bi()
                    .await
                    .map_err(|e| Error::Connect(e.to_string()))?
            }
        };
        Ok(tokio::io::join(recv, send))
    }

    /// Подключиться сейчас и сказать, каким путём пошло (для «Проверить соединение»).
    pub async fn check(&self) -> Result<Path, Error> {
        self.conn().await?;
        Ok(self.path().await)
    }

    pub async fn path(&self) -> Path {
        let slot = self.conn.lock().await;
        let live = slot.as_ref().filter(|c| c.close_reason().is_none());
        match live.and_then(selected) {
            Some((Some(addr), rtt_ms)) => Path::Direct { addr, rtt_ms },
            Some((None, rtt_ms)) => Path::Relayed { rtt_ms },
            None => Path::None,
        }
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

/// Сторона программы сотрудника: локальный порт → узел сервера.
pub struct Client {
    link: Arc<Dialer>,
    port: u16,
    task: JoinHandle<()>,
}

impl Client {
    /// Открыть локальный порт (`0` — любой). Соединение с узлом устанавливается
    /// при первом запросе, так что открыть можно и без сети.
    pub async fn open(local_port: u16, dial: Dial) -> Result<Self, Error> {
        let link = Arc::new(Dialer::new(dial).await?);
        let listener = TcpListener::bind(("127.0.0.1", local_port)).await?;
        let port = listener.local_addr()?.port();
        let l = link.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut tcp, _)) = listener.accept().await {
                let l = l.clone();
                tokio::spawn(async move {
                    if let Ok(mut stream) = l.stream().await {
                        let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
                    }
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

    pub async fn check(&self) -> Result<Path, Error> {
        self.link.check().await
    }

    pub async fn path(&self) -> Path {
        self.link.path().await
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}
