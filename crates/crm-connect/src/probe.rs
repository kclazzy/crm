//! Статус сервера до подключения: какой адрес отвечает и как быстро.

use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::pin::{client_config, server_name};
use crate::store::Server;

/// Сколько ждать один адрес: в офисной сети ответ приходит за миллисекунды.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
/// «Проверить соединение» ждёт дольше: Windows сообщает об отказе закрытого порта
/// примерно через 2 секунды, а нам важно отличить «CRM остановлена» от «нет ответа».
const DIAGNOSE_TIMEOUT: Duration = Duration::from_secs(3);
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
    connect_within(addr, port, fp, CONNECT_TIMEOUT).await
}

async fn connect_within(
    addr: &str,
    port: u16,
    fp: &str,
    wait: Duration,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, ProbeError> {
    let tcp = tokio::time::timeout(wait, TcpStream::connect((addr, port)))
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
    health_within(addr, port, fp, CONNECT_TIMEOUT).await
}

async fn health_within(
    addr: &str,
    port: u16,
    fp: &str,
    wait: Duration,
) -> Result<Health, ProbeError> {
    let mut s = connect_within(addr, port, fp, wait).await?;
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

/// Почему адрес (не) отвечает — по одному на каждый адрес сервера.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Online,
    /// Тишина: компьютер выключен, другая сеть или порт закрыт роутером.
    NoAnswer,
    /// Компьютер ответил отказом: CRM не запущена или порт не тот.
    Refused,
    /// Имя не найдено в сети.
    NotFound,
    /// Отвечает другой сервер — сертификат не совпал.
    WrongServer,
    /// Отвечает, но не CRM.
    NotCrm,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddrCheck {
    pub addr: String,
    /// Введён сотрудником — его можно убрать.
    pub manual: bool,
    pub via_internet: bool,
    pub verdict: Verdict,
    pub latency_ms: Option<u64>,
    pub version: Option<String>,
    /// Объяснение для человека.
    pub message: String,
}

fn verdict(e: &ProbeError) -> Verdict {
    use std::io::ErrorKind as K;
    match e {
        ProbeError::Timeout => Verdict::NoAnswer,
        ProbeError::Tls(_) => Verdict::WrongServer,
        ProbeError::BadAnswer => Verdict::NotCrm,
        ProbeError::Io(e) => match e.kind() {
            K::ConnectionRefused | K::ConnectionReset | K::ConnectionAborted => Verdict::Refused,
            K::TimedOut | K::HostUnreachable | K::NetworkUnreachable => Verdict::NoAnswer,
            // Имя не нашлось: у ошибки поиска имени нет своего вида.
            _ => Verdict::NotFound,
        },
    }
}

fn explain(v: Verdict, via_internet: bool, latency_ms: Option<u64>) -> String {
    match v {
        Verdict::Online => format!(
            "отвечает, {} мс — {}",
            latency_ms.unwrap_or(0),
            if via_internet {
                "через интернет"
            } else {
                "в сети офиса"
            }
        ),
        Verdict::NoAnswer if via_internet => {
            "не отвечает: доступ через интернет выключен или порт на роутере закрыт".into()
        }
        Verdict::NoAnswer => "не отвечает: компьютер выключен или вы не в сети офиса".into(),
        Verdict::Refused => {
            "компьютер на связи, но CRM на нём не отвечает — сервер остановлен".into()
        }
        Verdict::NotFound => "адрес не найден — проверьте написание".into(),
        Verdict::WrongServer => {
            "отвечает другой сервер (сертификат не совпал) — подключение запрещено".into()
        }
        Verdict::NotCrm => "по этому адресу отвечает не CRM".into(),
    }
}

/// Проверить все адреса сервера разом — для экрана «Проверить соединение».
pub async fn diagnose(server: &Server) -> Vec<AddrCheck> {
    let tasks: Vec<_> = server
        .addresses_in_order()
        .into_iter()
        .map(|addr| {
            let (port, fp) = (server.port, server.fp.clone());
            let manual = server.manual.contains(&addr);
            tokio::spawn(async move {
                let via_internet = !crm_core::remote::is_office_addr(&addr);
                let started = Instant::now();
                let (verdict, latency_ms, version) =
                    match health_within(&addr, port, &fp, DIAGNOSE_TIMEOUT).await {
                        Ok(h) => (
                            Verdict::Online,
                            Some(started.elapsed().as_millis() as u64),
                            Some(h.version),
                        ),
                        Err(e) => (verdict(&e), None, None),
                    };
                AddrCheck {
                    message: explain(verdict, via_internet, latency_ms),
                    addr,
                    manual,
                    via_internet,
                    verdict,
                    latency_ms,
                    version,
                }
            })
        })
        .collect();
    let mut out = Vec::new();
    for t in tasks {
        if let Ok(c) = t.await {
            out.push(c);
        }
    }
    out
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
