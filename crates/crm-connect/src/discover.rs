//! Поиск сервера CRM в сети офиса: запрос по UDP на широковещательные адреса всех
//! сетей компьютера, ответы — имя, адрес, порт, отпечаток. Найденный адрес только
//! подставляется в поле: подлинность сервера проверяет вход по коду.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crm_core::discovery::{decode, PORT, REQUEST};
use serde::Serialize;
use tokio::net::UdpSocket;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    /// Адрес, с которого ответил сервер.
    pub addr: String,
    pub name: String,
    pub port: u16,
    pub fp: String,
    pub version: String,
}

/// Широковещательные адреса сетей этого компьютера (и общий 255.255.255.255).
fn broadcasts() -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => v4.broadcast.map(|b| SocketAddr::from((b, PORT))),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    out.push(SocketAddr::from((Ipv4Addr::BROADCAST, PORT)));
    out.sort();
    out.dedup();
    out
}

/// Найти серверы в сети офиса (ждёт `wait`).
pub async fn find(wait: Duration) -> Vec<Found> {
    send_and_collect(&broadcasts(), wait).await
}

/// Спросить один адрес (проверка и тесты).
pub async fn ask(target: SocketAddr, wait: Duration) -> Vec<Found> {
    send_and_collect(&[target], wait).await
}

async fn send_and_collect(targets: &[SocketAddr], wait: Duration) -> Vec<Found> {
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)).await else {
        return Vec::new();
    };
    let _ = sock.set_broadcast(true);
    for t in targets {
        let _ = sock.send_to(REQUEST, t).await;
    }
    let mut out: Vec<Found> = Vec::new();
    let mut buf = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await
    {
        let Some(r) = decode(&buf[..n]) else { continue };
        let addr = match from.ip() {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => v6.to_string(),
        };
        // Один сервер может ответить с нескольких адресов — оставляем первый.
        if out.iter().any(|f| f.fp == r.fp && f.name == r.name) {
            continue;
        }
        out.push(Found {
            addr,
            name: r.name,
            port: r.port,
            fp: r.fp,
            version: r.version,
        });
    }
    out
}
