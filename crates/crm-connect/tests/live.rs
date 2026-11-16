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
        node: None,
        udp: None,
        manual: Vec::new(),
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
async fn diagnose_reports_every_address_with_a_reason() {
    use crm_connect::probe::{diagnose, Verdict};
    let dir = tempfile::tempdir().unwrap();
    let running = start(dir.path()).await;
    let port = running.addr.port();
    let fp = running.ca_fingerprint.clone().unwrap();

    let mut s = server(port, &fp, &["127.0.0.1"]);
    s.manual = vec!["no-such-host.invalid".into()];
    let d = diagnose(&s).await;
    let got: Vec<(&str, Verdict, bool)> = d
        .iter()
        .map(|c| (c.addr.as_str(), c.verdict, c.manual))
        .collect();
    assert_eq!(
        got,
        [
            ("no-such-host.invalid", Verdict::NotFound, true),
            ("127.0.0.1", Verdict::Online, false),
        ]
    );
    assert!(d[1].latency_ms.is_some());
    assert_eq!(d[1].version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(d[0].message.contains("не найден"), "{}", d[0].message);

    let d = diagnose(&server(port, &"00".repeat(32), &["127.0.0.1"])).await;
    assert_eq!(d[0].verdict, Verdict::WrongServer);

    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let d = diagnose(&server(free, &fp, &["127.0.0.1"])).await;
    assert_eq!(d[0].verdict, Verdict::Refused);
    running.stop().await.unwrap();
}

#[tokio::test]
async fn tunnel_carries_plain_http_to_pinned_tls() {
    let dir = tempfile::tempdir().unwrap();
    let running = start(dir.path()).await;
    let fp = running.ca_fingerprint.clone().unwrap();
    let t = Tunnel::open(
        0,
        Target::Tls {
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
    t.retarget(Target::Tls {
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

#[tokio::test(flavor = "multi_thread")]
async fn when_no_address_answers_the_node_key_connects() {
    // Только прямые соединения: в тесте нет интернета.
    std::env::set_var("CRM_P2P_RELAY", "off");
    let dir = tempfile::tempdir().unwrap();
    let cfg = crm_server::config::Config::from_toml(&format!(
        "listen = \"127.0.0.1:0\"\np2p_relay = false\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.path().join("data"),
        dir.path().join("files"),
    ))
    .unwrap();
    let pool = crm_server::db::open(&cfg.storage).await.unwrap();
    crm_server::p2p::set_enabled(&pool, true).await.unwrap();
    pool.close().await;
    let running = crm_server::server::start(&cfg).await.unwrap();
    let node = running.p2p().await.unwrap();
    let fp = running.ca_fingerprint.clone().unwrap();

    // TCP-порт сервера «недоступен» (как из дома без открытого порта на роутере).
    let dead = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut s = server(dead, &fp, &["127.0.0.1"]);
    assert_eq!(
        probe::probe(&s).await.state,
        State::Offline,
        "без узла связи нет"
    );
    s.node = Some(node.id.clone());
    s.udp = Some(node.udp_port);

    let p = probe::probe(&s).await;
    assert_eq!(p.state, State::Online, "{:?}", p.error);
    assert!(p.node && p.addr.is_none());
    assert!(!p.via_internet, "прямой путь внутри сети — это офис");

    let t = Tunnel::open(0, Target::of(&s, &p).await.unwrap())
        .await
        .unwrap();
    let mut c = tokio::net::TcpStream::connect(t.local).await.unwrap();
    c.write_all(b"GET /api/health HTTP/1.1\r\nHost: crm\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = String::new();
    c.read_to_string(&mut buf).await.unwrap();
    assert!(buf.starts_with("HTTP/1.1 200"), "{buf}");

    // Если адрес отвечает — он в приоритете, узел не нужен.
    let mut near = server(running.addr.port(), &fp, &["127.0.0.1"]);
    near.node = s.node.clone();
    let p = probe::probe(&near).await;
    assert!(p.state == State::Online && !p.node && p.addr.is_some());
    // …и сервер сам сообщает свой узел: программа запомнит его в офисе, новый ключ не нужен.
    assert_eq!(p.node_id.as_deref(), Some(node.id.as_str()));
    assert_eq!(p.node_udp, Some(node.udp_port));
    running.stop().await.unwrap();
}
