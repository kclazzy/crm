//! Туннель: окно программы ↔ 127.0.0.1:ПОРТ ↔ TLS (закреплённый CA) ↔ сервер.
//!
//! Работает на уровне байтов, поэтому HTTP, загрузка файлов и WebSocket идут без изменений.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::probe::connect;

/// Куда ведёт туннель; меняется, если сотрудник ушёл из офиса и подключается иначе.
#[derive(Clone)]
pub enum Target {
    /// По адресу: TLS с закреплённым сертификатом сервера.
    Tls { addr: String, port: u16, fp: String },
    /// По ключу узла: сервер находится сам, шифрование — между узлами.
    Node(Arc<crm_p2p::Dialer>),
}

impl Target {
    /// Куда вести туннель по итогам проверки (`None` — сервер не отвечает).
    pub async fn of(server: &crate::Server, probe: &crate::probe::Probe) -> Option<Self> {
        if probe.state != crate::probe::State::Online {
            return None;
        }
        match &probe.addr {
            Some(addr) => Some(Self::Tls {
                addr: addr.clone(),
                port: server.port,
                fp: server.fp.clone(),
            }),
            None => crate::node::dialer(server).await?.ok().map(Self::Node),
        }
    }
}

/// Сервер недоступен — понятная страница вместо оборванного соединения.
async fn explain(mut inbound: tokio::net::TcpStream, e: String) {
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
         <h2>Сервер CRM не отвечает</h2><p style=\"color:#64748b\">{e}</p>\
         <h3>Что сделать</h3><ol style=\"line-height:1.7\">\
         <li><b>В офисе?</b> Проверьте Wi-Fi или кабель и обновите страницу (F5).</li>\
         <li><b>Не в офисе?</b> Нажмите внизу «← В программу CRM», затем «Подключиться» → «🏠 Не в офисе» и вставьте ключ подключения от владельца.</li>\
         <li><b>Сервер выключен?</b> Сообщите владельцу: компьютер с CRM должен быть включён.</li>\
         <li>Там же, в программе, кнопка «Проверить» покажет, что именно не отвечает.</li></ol>"
    );
    let resp = format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = inbound.write_all(resp.as_bytes()).await;
    let _ = inbound.shutdown().await;
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
                    match t {
                        Target::Tls { addr, port, fp } => match connect(&addr, port, &fp).await {
                            Ok(mut outbound) => {
                                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound)
                                    .await;
                            }
                            Err(e) => explain(inbound, e.to_string()).await,
                        },
                        Target::Node(dialer) => match dialer.stream().await {
                            Ok(mut outbound) => {
                                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound)
                                    .await;
                            }
                            Err(e) => explain(inbound, e.to_string()).await,
                        },
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
