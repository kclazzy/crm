//! Статус сервера до подключения: какой адрес отвечает и как быстро.

use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::pin::{client_config, server_name};
use crate::store::Server;

/// Сколько ждать один адрес: в офисной сети ответ приходит за миллисекунды.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
const READ_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Отвечает, сертификат совпал.
    Online,
    /// Ни один адрес не отвечает.
    Offline,
    /// Отвечает, но сертификат чужой — подключаться нельзя.
    WrongServer,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub state: State,
    /// Какой адрес ответил.
    pub addr: Option<String>,
    /// Через интернет (внешний адрес), а не в сети офиса.
    pub via_internet: bool,
    /// Внешний адрес, который сообщил сервер (`None` — доступ через интернет выключен).
    pub public_addr: Option<String>,
    pub version: Option<String>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("нет ответа")]
    Timeout,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("сертификат: {0}")]
    Tls(String),
    #[error("сервер ответил непонятно")]
    BadAnswer,
}

/// Подключиться к адресу по TLS с закреплённым CA.
pub async fn connect(
    addr: &str,
    port: u16,
    fp: &str,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, ProbeError> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((addr, port)))
        .await
        .map_err(|_| ProbeError::Timeout)??;
    tcp.set_nodelay(true)?;
    let cfg = client_config(fp).map_err(|e| ProbeError::Tls(e.to_string()))?;
    let name = server_name(addr).map_err(|e| ProbeError::Tls(e.to_string()))?;
    tokio::time::timeout(
        CONNECT_TIMEOUT * 2,
        tokio_rustls::TlsConnector::from(cfg).connect(name, tcp),
    )
    .await
    .map_err(|_| ProbeError::Timeout)?
    .map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            ProbeError::Tls(e.to_string())
        } else {
            ProbeError::Io(e)
        }
    })
}

struct Health {
    version: String,
    public_addr: Option<String>,
}

async fn health(addr: &str, port: u16, fp: &str) -> Result<Health, ProbeError> {
    let mut s = connect(addr, port, fp).await?;
    let req = format!("GET /api/health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    tokio::time::timeout(READ_TIMEOUT, s.read_to_end(&mut buf))
        .await
        .map_err(|_| ProbeError::Timeout)?
        // Сервер может закрыть TLS без close_notify — данные уже получены.
        .or_else(|e| if buf.is_empty() { Err(e) } else { Ok(0) })?;
    let text = String::from_utf8_lossy(&buf);
    let body = text.split("\r\n\r\n").nth(1).ok_or(ProbeError::BadAnswer)?;
    let v: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|_| ProbeError::BadAnswer)?;
    Ok(Health {
        version: v["version"]
            .as_str()
            .map(str::to_owned)
            .ok_or(ProbeError::BadAnswer)?,
        public_addr: v["public_addr"].as_str().map(str::to_owned),
    })
}

/// Проверить адреса сервера по порядку: последний удачный, затем офис, затем интернет.
pub async fn probe(server: &Server) -> Probe {
    let mut last_error = None;
    let mut wrong = false;
    for addr in server.addresses_in_order() {
        let started = Instant::now();
        match health(&addr, server.port, &server.fp).await {
            Ok(h) => {
                return Probe {
                    state: State::Online,
                    via_internet: !crm_core::remote::is_office_addr(&addr),
                    public_addr: h.public_addr,
                    addr: Some(addr),
                    version: Some(h.version),
                    latency_ms: Some(started.elapsed().as_millis() as u64),
                    error: None,
                }
            }
            Err(e) => {
                wrong |= matches!(e, ProbeError::Tls(_));
                last_error = Some(format!("{addr}: {e}"));
            }
        }
    }
    Probe {
        state: if wrong {
            State::WrongServer
        } else {
            State::Offline
        },
        addr: None,
        via_internet: false,
        public_addr: None,
        version: None,
        latency_ms: None,
        error: last_error,
    }
}
