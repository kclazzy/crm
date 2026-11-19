//! Поиск сервера в сети офиса: настоящий сервер отвечает на запрос программы.
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn running_server_answers_discovery_with_its_name_port_and_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = crm_server::config::Config::from_toml(&format!(
        "listen = \"0.0.0.0:0\"\n[discovery]\nport = 0\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.path().join("data"),
        dir.path().join("files"),
    ))
    .unwrap();
    let running = crm_server::server::start(&cfg).await.unwrap();
    let udp = running.discovery_port.expect("поиск запущен");

    let found = crm_connect::discover::ask(
        SocketAddr::from(([127, 0, 0, 1], udp)),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(found.len(), 1, "{found:?}");
    let f = &found[0];
    assert_eq!(f.addr, "127.0.0.1");
    assert_eq!(f.port, running.addr.port());
    assert_eq!(Some(f.fp.clone()), running.ca_fingerprint);
    assert!(!f.name.is_empty());
    running.stop().await.unwrap();
}

#[tokio::test]
async fn server_listening_only_on_loopback_does_not_announce_itself() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = crm_server::config::Config::from_toml(&format!(
        "listen = \"127.0.0.1:0\"\n[discovery]\nport = 0\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.path().join("data"),
        dir.path().join("files"),
    ))
    .unwrap();
    let running = crm_server::server::start(&cfg).await.unwrap();
    assert_eq!(running.discovery_port, None);
    running.stop().await.unwrap();
}
