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
            key: None,
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

fn remote_device(n: u16) -> crm_p2p::Peer {
    crm_p2p::Peer {
        id: format!("{n:04x}").repeat(16),
        direct: None,
    }
}

/// Новое устройство вне офиса не видит даже страницу входа, пока владелец его не одобрит.
#[tokio::test]
async fn new_remote_device_waits_for_owner_approval() {
    use crm_server::p2p::{admit, Admit};
    let app = common::App::new().await;
    let owner = app.owner().await;
    let (_, ivan) = app
        .member(&owner, "ivan", "employee", common::OFFICE_PC)
        .await;
    let pool = &app.state.pool;

    // Из сети офиса — без одобрения и без записи.
    let office = crm_p2p::Peer {
        id: "aa".repeat(32),
        direct: Some("192.168.1.20:5000".parse().unwrap()),
    };
    assert_eq!(admit(pool, &office).await.unwrap(), Admit::Serve);

    // Вне офиса: ждёт, владелец видит его в списке с тем же кодом.
    let dev = remote_device(7);
    let Admit::Pending { code } = admit(pool, &dev).await.unwrap() else {
        panic!("новое устройство пропущено без одобрения");
    };
    let url = "/api/admin/remote/devices";
    let (s, _) = app.call("GET", url, common::LOCAL, Some(&ivan), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "список видит только владелец");
    let (s, v) = app
        .call("GET", url, common::LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["devices"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["devices"][0]["code"], code.as_str());
    assert_eq!(v["devices"][0]["approved"], false);
    assert_eq!(v["approval_required"], true);

    // Одобрил — устройство проходит; повторное обращение ничего не дублирует.
    let (s, v) = app
        .call(
            "PUT",
            &format!("{url}/{}", dev.id),
            common::LOCAL,
            Some(&owner),
            Some(serde_json::json!({ "approved": true, "name": "Ноутбук Ивана" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(admit(pool, &dev).await.unwrap(), Admit::Serve);
    let (_, v) = app
        .call("GET", url, common::LOCAL, Some(&owner), None)
        .await;
    assert_eq!(v["devices"][0]["name"], "Ноутбук Ивана");
    assert_eq!(v["devices"][0]["approved"], true);

    // Удалил (ноутбук потерян) — снова ждёт.
    let (s, _) = app
        .call(
            "DELETE",
            &format!("{url}/{}", dev.id),
            common::LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(matches!(
        admit(pool, &dev).await.unwrap(),
        Admit::Pending { .. }
    ));

    // Посторонний не может завалить список заявками: хранится не больше 30 ожидающих.
    for n in 100..160 {
        admit(pool, &remote_device(n)).await.unwrap();
    }
    let (_, v) = app
        .call("GET", url, common::LOCAL, Some(&owner), None)
        .await;
    assert_eq!(v["devices"].as_array().unwrap().len(), 30, "{v}");

    // Владелец может отключить одобрение — тогда защита только пароль и код.
    let (s, _) = app
        .call(
            "PUT",
            url,
            common::LOCAL,
            Some(&owner),
            Some(serde_json::json!({ "approval_required": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        admit(pool, &remote_device(999)).await.unwrap(),
        Admit::Serve
    );
}

/// Компьютер, который хоть раз подключался в офисе, одобрять не нужно — иначе
/// владельцу в отъезде некому было бы одобрить собственный ноутбук.
#[tokio::test]
async fn device_seen_in_the_office_is_approved_automatically() {
    use crm_server::p2p::{admit, Admit};
    let app = common::App::new().await;
    app.owner().await;
    let pool = &app.state.pool;
    let laptop = remote_device(0x51);
    let stranger = remote_device(0x52);

    // Программа в офисе при проверке связи называет ключ своего устройства.
    let hello = |id: &str| format!("/api/health?device={id}");
    let (s, _) = app
        .call("GET", &hello(&laptop.id), common::OFFICE_PC, None, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(admit(pool, &laptop).await.unwrap(), Admit::Serve);

    // Из интернета так одобрить себя нельзя.
    let (s, _) = app
        .call("GET", &hello(&stranger.id), "85.12.34.56", None, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(matches!(
        admit(pool, &stranger).await.unwrap(),
        Admit::Pending { .. }
    ));
    // Мусор вместо ключа не записывается.
    let (s, _) = app
        .call(
            "GET",
            "/api/health?device=%27;drop",
            common::OFFICE_PC,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM p2p_devices")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}
