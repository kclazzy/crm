//! Ответ на поиск сервера в сети офиса (UDP). Отвечает только адресам офиса и
//! этого компьютера: из интернета сервер себя не называет.

use std::net::SocketAddr;

use crm_core::discovery::{encode, Reply, REQUEST};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// Запустить ответчик. `None` — порт занят или поиск не нужен (сервер слушает
/// только этот компьютер).
pub async fn start(listen: SocketAddr, port: u16, reply: Reply) -> Option<(u16, JoinHandle<()>)> {
    if listen.ip().is_loopback() {
        return None;
    }
    let sock = match UdpSocket::bind(("0.0.0.0", port)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("поиск сервера в сети офиса не запущен (UDP {port}): {e}");
            return None;
        }
    };
    let bound = sock.local_addr().ok()?.port();
    let answer = encode(&reply);
    let task = tokio::spawn(async move {
        let mut buf = [0u8; 64];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                continue;
            };
            let ip = from.ip();
            let office = ip.is_loopback() || crm_core::remote::is_office_addr(&ip.to_string());
            if office && &buf[..n] == REQUEST {
                let _ = sock.send_to(&answer, from).await;
            }
        }
    });
    Some((bound, task))
}
