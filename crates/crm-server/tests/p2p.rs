//! Подключение по ключу узла к настоящему серверу CRM: без адреса и без TLS-порта.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use crm_p2p::{Client, Dial, Relay};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn config(dir: &std::path::Path) -> crm_server::config::Config {
    crm_server::config::Config::from_toml(&format!(
        "listen = \"127.0.0.1:0\"\np2p_relay = false\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.join("data"),
        dir.join("files"),
    ))
    .unwrap()
}

async fn get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: crm\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn crm_answers_by_node_key_only_when_owner_enabled_it() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());

    // По умолчанию выключено: узла нет.
    let running = crm_server::server::start(&cfg).await.unwrap();
    assert_eq!(running.p2p().await, None);
    running.stop().await.unwrap();

    // Владелец включил (настройка в базе) — узел поднимается при запуске.
    let pool = crm_server::db::open(&cfg.storage).await.unwrap();
    crm_server::p2p::set_enabled(&pool, true).await.unwrap();
    pool.close().await;
    let running = crm_server::server::start(&cfg).await.unwrap();
    let node = running.p2p().await.expect("узел запущен");

    let client = Client::open(
        0,
        Dial {
            id: node.id.clone(),
            direct: vec![([127, 0, 0, 1], node.udp_port).into()],
            relay: Relay::Off,
        },
    )
    .await
    .unwrap();
    let answer = get(client.port(), "/api/health").await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains("\"status\":\"ok\""), "{answer}");
    // Страница входа тоже отдаётся — окно программы откроет CRM как обычно.
    assert!(get(client.port(), "/").await.starts_with("HTTP/1.1 200"));
    running.stop().await.unwrap();

    // Ключ узла постоянный: после перезапуска программы сотрудников его не теряют.
    let running = crm_server::server::start(&cfg).await.unwrap();
    assert_eq!(running.p2p().await.unwrap().id, node.id);
    running.stop().await.unwrap();
}

#[test]
fn only_devices_inside_the_office_network_count_as_office() {
    use crm_core::remote::is_office_addr;
    use crm_server::p2p::origin;
    let peer = |direct: Option<&str>| crm_p2p::Peer {
        id: "ab".repeat(32),
        direct: direct.map(|a| a.parse().unwrap()),
    };
    // Прямой путь из сети офиса — офис, с настоящим адресом.
    let a = origin(&peer(Some("192.168.1.20:5000")));
    assert_eq!(a.ip().to_string(), "192.168.1.20");
    // Через ретранслятор или с внешнего адреса — «из интернета»: код из приложения обязателен.
    for direct in [None, Some("85.12.34.56:5000")] {
        let a = origin(&peer(direct));
        assert!(!is_office_addr(&a.ip().to_string()), "{a}");
    }
    // У каждого устройства свой адрес: лимит попыток входа не общий на всех.
    let other = crm_p2p::Peer {
        id: "cd".repeat(32),
        direct: None,
    };
    assert_ne!(origin(&peer(None)).ip(), origin(&other).ip());
    assert_eq!(origin(&peer(None)).ip(), origin(&peer(None)).ip());
}

/// Адрес устройства при входе по ключу сервера задаёт сам подключающийся (это его ключ),
/// поэтому смена ключа не должна давать новые попытки подбора пароля.
#[tokio::test]
async fn changing_the_device_key_does_not_reset_password_guess_limit() {
    let app = common::App::new().await;
    app.owner().await;
    let device = |n: u16| {
        let peer = crm_p2p::Peer {
            id: format!("{n:04x}").repeat(16),
            direct: None,
        };
        format!("[{}]", crm_server::p2p::origin(&peer).ip())
    };
    for n in 0..crm_server::auth::LOCKOUT_LIMIT as u16 {
        let (s, _) = app.try_login("owner", "не тот пароль", &device(n)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // Новое «устройство» — а попыток больше нет, даже с верным паролем.
    let (s, v) = app
        .try_login("owner", "очень-надёжный-пароль", &device(999))
        .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");
    // В офисе владелец входит как обычно: блокировка касается только входа по ключу.
    let (s, _) = app
        .try_login("owner", "очень-надёжный-пароль", common::OFFICE_PC)
        .await;
    assert_eq!(s, StatusCode::OK);
}
