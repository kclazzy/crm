//! Два узла на одной машине: запрос идёт программа → локальный порт → QUIC → сервер.
#![allow(clippy::unwrap_used)]

use crm_p2p::{Client, Dial, Host, Path, Relay};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// «Сервер CRM»: отвечает на каждое соединение одной строкой с тем, что получил.
async fn fake_crm() -> std::net::SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                let n = s.read(&mut buf).await.unwrap();
                let body = format!("crm got: {}", String::from_utf8_lossy(&buf[..n]));
                s.write_all(body.as_bytes()).await.unwrap();
            });
        }
    });
    addr
}

async fn ask(port: u16, text: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(text.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_travel_by_key_without_internet_and_survive_host_restart() {
    let crm = fake_crm().await;
    let key = [7u8; 32];
    let host = Host::start(key, 0, crm, Relay::Off).await.unwrap();
    let dial = Dial {
        id: host.id(),
        direct: vec![([127, 0, 0, 1], host.udp_port()).into()],
        relay: Relay::Off,
        key: None,
    };

    let client = Client::open(0, dial.clone()).await.unwrap();
    assert_eq!(ask(client.port(), "ping").await, "crm got: ping");
    // Несколько запросов подряд и одновременно — по одному каналу.
    let (a, b) = tokio::join!(ask(client.port(), "a"), ask(client.port(), "b"));
    assert_eq!((a.as_str(), b.as_str()), ("crm got: a", "crm got: b"));
    assert!(
        matches!(client.path().await, Path::Direct { .. }),
        "в одной сети соединение прямое: {:?}",
        client.path().await
    );

    // Сервер перезапустили с тем же ключом — программа подключается заново сама.
    let port = host.udp_port();
    host.stop().await;
    // Порт освобождается не мгновенно — как и при настоящем перезапуске, пробуем несколько раз.
    let mut restarted = None;
    for _ in 0..20 {
        match Host::start(key, port, crm, Relay::Off).await {
            Ok(h) => {
                restarted = Some(h);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let host = restarted.expect("узел перезапустился на том же порту");
    assert_eq!(host.id(), dial.id, "ключ тот же — адрес узла тот же");
    assert_eq!(ask(client.port(), "again").await, "crm got: again");

    // Чужой ключ: к другому узлу по этому адресу не подключиться.
    let stranger = Dial {
        id: Host::start([9u8; 32], 0, crm, Relay::Off)
            .await
            .unwrap()
            .id(),
        ..dial
    };
    let c = Client::open(0, stranger).await.unwrap();
    assert!(
        c.check().await.is_err(),
        "по адресу отвечает узел с другим ключом — соединения нет"
    );
    host.stop().await;
}
