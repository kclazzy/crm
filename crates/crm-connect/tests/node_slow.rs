//! Первое соединение по ключу сервера бывает долгим: поиск узла, ретранслятор,
//! пробивка роутера — 5–12 секунд. Проверка «отвечает ли сервер» не должна
//! обрывать такое соединение раньше, чем оно успеет установиться: иначе каждый
//! следующий раз начинается с нуля и сервер «не отвечает» навсегда.
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crm_connect::store::Server;
use crm_p2p::{Host, Relay};
use tokio::net::UdpSocket;

/// Задержка пакетов на пути к узлу — как медленная первая связь через интернет.
const SLOW: Duration = Duration::from_secs(8);

/// UDP-посредник: пакеты к узлу идут с задержкой, обратно — сразу.
async fn slow_link(to: SocketAddr) -> u16 {
    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let port = sock.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut client: Option<SocketAddr> = None;
        let mut buf = vec![0u8; 65536];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                return;
            };
            let packet = buf[..n].to_vec();
            if from == to {
                if let Some(c) = client {
                    let _ = sock.send_to(&packet, c).await;
                }
            } else {
                client = Some(from);
                let s = sock.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(SLOW).await;
                    let _ = s.send_to(&packet, to).await;
                });
            }
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_first_connection_is_awaited_not_cut_off() {
    // Только прямой путь в «сети офиса» — без интернета и ретрансляторов.
    std::env::set_var("CRM_P2P_RELAY", "off");
    let crm = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Host::start(crm_p2p::new_key(), 0, crm.local_addr().unwrap(), Relay::Off)
        .await
        .unwrap();
    let udp = slow_link(SocketAddr::from(([127, 0, 0, 1], host.udp_port()))).await;
    let server = Server {
        id: "x".into(),
        name: "Офис".into(),
        addrs: vec!["127.0.0.1".into()],
        port: 1,
        fp: "00".into(),
        node: Some(host.id()),
        udp: Some(udp),
        manual: Vec::new(),
        last_addr: None,
        local_port: 0,
        user: None,
        invite: None,
    };

    let started = Instant::now();
    let found = crm_connect::node::check(&server).await.unwrap();
    assert!(
        found.is_ok(),
        "сервер отвечает, просто медленно ({:?}): {found:?}",
        started.elapsed()
    );
    assert!(started.elapsed() >= SLOW);
    host.stop().await;
}
