//! Туннель: окно программы ↔ 127.0.0.1:ПОРТ ↔ TLS (закреплённый CA) ↔ сервер.
//!
//! Работает на уровне байтов, поэтому HTTP, загрузка файлов и WebSocket идут без изменений.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::probe::connect;

/// Куда ведёт туннель; адрес меняется, если сотрудник ушёл из офиса и подключается через интернет.
#[derive(Debug, Clone)]
pub struct Target {
    pub addr: String,
    pub port: u16,
    pub fp: String,
}

pub struct Tunnel {
    pub local: SocketAddr,
    target: Arc<RwLock<Target>>,
    task: JoinHandle<()>,
}

impl Tunnel {
    /// Открыть туннель на `127.0.0.1:port` (если занят — на свободном).
    pub async fn open(port: u16, target: Target) -> std::io::Result<Self> {
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let local = listener.local_addr()?;
        let target = Arc::new(RwLock::new(target));
        let shared = target.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    continue;
                };
                let t = shared.read().await.clone();
                tokio::spawn(async move {
                    let _ = inbound.set_nodelay(true);
                    match connect(&t.addr, t.port, &t.fp).await {
                        Ok(mut outbound) => {
                            let _ =
                                tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                        }
                        Err(e) => {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            // Сначала дочитать запрос — иначе закрытие сокета с непрочитанными
                            // данными оборвёт соединение (RST) и браузер не увидит ответ.
                            let mut sink = [0u8; 8192];
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_millis(300),
                                inbound.read(&mut sink),
                            )
                            .await;
                            let body = format!(
                                "<!doctype html><meta charset=utf-8><title>CRM</title>\
                                 <body style=\"font-family:sans-serif;padding:40px\">\
                                 <h2>Сервер CRM не отвечает</h2><p>{e}</p>\
                                 <p>Проверьте интернет или сеть офиса и обновите страницу (F5).</p>"
                            );
                            let resp = format!(
                                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = inbound.write_all(resp.as_bytes()).await;
                            let _ = inbound.shutdown().await;
                        }
                    }
                });
            }
        });
        Ok(Self {
            local,
            target,
            task,
        })
    }

    /// Новый адрес сервера (например, пропала офисная сеть — дальше через интернет).
    pub async fn retarget(&self, t: Target) {
        *self.target.write().await = t;
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.local)
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}
