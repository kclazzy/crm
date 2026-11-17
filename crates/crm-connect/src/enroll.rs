//! Подключение в офисе по короткому коду: адрес сервера и 8 символов от владельца →
//! ключ подключения (протокол — `crm_core::enroll`).

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use crm_core::enroll::{program_start, Answer, Code};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Порт сервера CRM по умолчанию.
pub const DEFAULT_PORT: u16 = 8443;
const WAIT: Duration = Duration::from_secs(5);

/// Почему не получилось — человеческими словами для экрана.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnrollFailure {
    #[error("{0}")]
    BadInput(String),
    #[error("сервер не отвечает по этому адресу: {0}")]
    NoServer(String),
    #[error("код не подошёл или устарел — проверьте его или попросите у владельца новый")]
    WrongCode,
    #[error("сервер ответил ошибкой: {0}")]
    Server(String),
}

/// `office-pc`, `192.168.1.10:8443` → (адрес, порт).
fn split_addr(input: &str) -> Result<(String, u16), EnrollFailure> {
    let s = input.trim();
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (
            h,
            p.parse::<u16>()
                .map_err(|_| EnrollFailure::BadInput("порт — число, например 8443".into()))?,
        ),
        _ => (s, DEFAULT_PORT),
    };
    let host = crm_core::remote::manual_address(host, port)
        .map_err(|e| EnrollFailure::BadInput(e.to_owned()))?;
    Ok((host, port))
}

/// Выполнить обмен по коду и вернуть строку ключа подключения `CRM1-…`.
pub async fn enroll(addr: &str, code: &str) -> Result<String, EnrollFailure> {
    let code = Code::parse(code).map_err(|e| EnrollFailure::BadInput(e.to_string()))?;
    let (host, port) = split_addr(addr)?;
    let no_server = |e: &dyn std::fmt::Display| EnrollFailure::NoServer(e.to_string());

    let tcp = tokio::time::timeout(WAIT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| no_server(&"нет ответа"))?
        .map_err(|e| no_server(&e))?;
    let (cfg, seen) = crate::pin::learning_config();
    let name = crate::pin::server_name(&host).map_err(|e| no_server(&e))?;
    let mut tls = tokio::time::timeout(
        WAIT,
        tokio_rustls::TlsConnector::from(cfg).connect(name, tcp),
    )
    .await
    .map_err(|_| no_server(&"нет ответа"))?
    .map_err(|e| no_server(&e))?;
    let fp = seen
        .lock()
        .ok()
        .and_then(|s| s.clone())
        .ok_or_else(|| no_server(&"по этому адресу отвечает не сервер CRM"))?;

    let (program, msg) = program_start(&code, &fp);
    let body = serde_json::json!({ "id": code.id(), "msg": B64.encode(msg) }).to_string();
    let req = format!(
        "POST /api/enroll HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(req.as_bytes())
        .await
        .map_err(|e| no_server(&e))?;
    let mut buf = Vec::new();
    tokio::time::timeout(WAIT, tls.read_to_end(&mut buf))
        .await
        .map_err(|_| no_server(&"нет ответа"))?
        // Сервер может закрыть TLS без close_notify — ответ уже получен.
        .or_else(|e| {
            if buf.is_empty() {
                Err(no_server(&e))
            } else {
                Ok(0)
            }
        })?;

    let text = String::from_utf8_lossy(&buf);
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| EnrollFailure::Server("непонятный ответ".into()))?;
    let v: serde_json::Value = text
        .split("\r\n\r\n")
        .nth(1)
        .and_then(|b| serde_json::from_str(b.trim()).ok())
        .unwrap_or_default();
    match status {
        200 => {}
        404 => return Err(EnrollFailure::WrongCode),
        _ => {
            let why = v["error"]
                .as_str()
                .unwrap_or("неизвестная ошибка")
                .to_owned();
            return Err(EnrollFailure::Server(why));
        }
    }
    let field = |k: &str| {
        v[k].as_str()
            .and_then(|s| B64.decode(s).ok())
            .ok_or_else(|| EnrollFailure::Server("ответ повреждён".into()))
    };
    let answer = Answer {
        msg: field("msg")?,
        nonce: field("nonce")?
            .try_into()
            .map_err(|_| EnrollFailure::Server("ответ повреждён".into()))?,
        sealed: field("sealed")?,
    };
    program
        .finish(&answer)
        .map_err(|_| EnrollFailure::WrongCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_is_typed_with_or_without_port() {
        assert_eq!(
            split_addr(" OFFICE-PC ").unwrap(),
            ("office-pc".into(), 8443)
        );
        assert_eq!(
            split_addr("192.168.1.10:9443").unwrap(),
            ("192.168.1.10".into(), 9443)
        );
        assert!(split_addr("").is_err());
        assert!(split_addr("pc:порт").is_err());
    }
}
