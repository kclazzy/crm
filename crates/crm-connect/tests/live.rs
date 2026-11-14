//! Против настоящего сервера CRM с HTTPS: статус, чужой сервер, туннель.
#![allow(clippy::unwrap_used)]

use crm_connect::probe::State;
use crm_connect::tunnel::{Target, Tunnel};
use crm_connect::{probe, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn start(dir: &std::path::Path) -> crm_server::server::Running {
    let cfg = crm_server::config::Config::from_toml(&format!(
        "listen = \"127.0.0.1:0\"\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.join("data"),
        dir.join("files"),
    ))
    .unwrap();
    crm_server::server::start(&cfg).await.unwrap()
}

fn server(port: u16, fp: &str, addrs: &[&str]) -> Server {
    Server {
        id: "x".into(),
        name: "Офис".into(),
        addrs: addrs.iter().map(|a| (*a).to_owned()).collect(),
        port,
        fp: fp.into(),
        last_addr: None,
        local_port: 0,
        user: None,
    }
}

#[tokio::test]
async fn probe_tells_online_offline_and_wrong_server() {
    let dir = tempfile::tempdir().unwrap();
    let running = start(dir.path()).await;
    let port = running.addr.port();
    let fp = running.ca_fingerprint.clone().unwrap();

    // Первый адрес не отвечает — берётся следующий.
    let p = probe(&server(port, &fp, &["127.0.0.2", "127.0.0.1"])).await;
    assert_eq!(p.state, State::Online, "{p:?}");
    assert_eq!(p.addr.as_deref(), Some("127.0.0.1"));
    assert_eq!(p.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));

    let other = "00".repeat(32);
    let p = probe(&server(port, &other, &["127.0.0.1"])).await;
    assert_eq!(p.state, State::WrongServer, "чужой отпечаток: {p:?}");

    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let p = probe(&server(free, &fp, &["127.0.0.1"])).await;
    assert_eq!(p.state, State::Offline);
    running.stop().await.unwrap();
}

#[tokio::test]
async fn tunnel_carries_plain_http_to_pinned_tls() {
    let dir = tempfile::tempdir().unwrap();
    let running = start(dir.path()).await;
    let fp = running.ca_fingerprint.clone().unwrap();
    let t = Tunnel::open(
        0,
        Target {
            addr: "127.0.0.1".into(),
            port: running.addr.port(),
            fp: fp.clone(),
        },
    )
    .await
    .unwrap();
    assert!(t.url().starts_with("http://127.0.0.1:"));

    let mut s = tokio::net::TcpStream::connect(t.local).await.unwrap();
    s.write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    assert!(buf.starts_with("HTTP/1.1 200"), "{buf}");
    assert!(buf.contains(&fp));

    // Сервер с чужим сертификатом — понятная страница вместо соединения.
    t.retarget(Target {
        addr: "127.0.0.1".into(),
        port: running.addr.port(),
        fp: "11".repeat(32),
    })
    .await;
    let mut s = tokio::net::TcpStream::connect(t.local).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    assert!(
        buf.starts_with("HTTP/1.1 502") && buf.contains("не отвечает"),
        "{buf}"
    );
    running.stop().await.unwrap();
}
