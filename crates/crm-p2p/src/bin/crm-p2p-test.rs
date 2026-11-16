//! Проверка связи «по ключу узла» на ваших сетях — до встраивания в CRM.
//!
//! На компьютере-сервере:   crm-p2p-test server
//! На втором компьютере:    строка, которую напечатал сервер.
//!
//! Данные CRM не затрагиваются: сервер отвечает тестовой строкой.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crm_p2p::{Client, Dial, Host, Path, Relay};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Постоянный порт узла в тесте: по нему в офисе подключаемся напрямую.
const UDP_PORT: u16 = 47_443;

type AnyError = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("server") => server().await,
        Some("client") if args.len() >= 2 => client(&args[1], &args[2..]).await,
        _ => {
            println!("На сервере:            crm-p2p-test server");
            println!("На втором компьютере:  crm-p2p-test client КЛЮЧ [адрес:порт]");
            Ok(())
        }
    };
    if let Err(e) = result {
        println!("Ошибка: {e}");
        std::process::exit(1);
    }
}

/// Адрес этого компьютера в локальной сети (пакеты не отправляются).
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

async fn server() -> Result<(), AnyError> {
    // Вместо CRM — ответчик одной строкой.
    let l = TcpListener::bind("127.0.0.1:0").await?;
    let crm = l.local_addr()?;
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut b = [0u8; 16];
                let _ = s.read(&mut b).await;
                let _ = s.write_all(b"crm-ok").await;
            });
        }
    });
    let key = iroh::SecretKey::generate().to_bytes();
    let host = Host::start(key, UDP_PORT, crm, Relay::Public).await?;
    let direct = lan_ip().map_or(String::new(), |ip| format!(" {ip}:{}", host.udp_port()));
    println!("Узел сервера запущен. На втором компьютере выполните:\n");
    println!("  crm-p2p-test client {}{direct}\n", host.id());
    println!("Не закрывайте это окно, пока идёт проверка. Остановить — Ctrl+C.");
    tokio::signal::ctrl_c().await?;
    host.stop().await;
    Ok(())
}

async fn round_trip(port: u16) -> Result<Duration, AnyError> {
    let started = Instant::now();
    let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
    s.write_all(b"ping").await?;
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), s.read_to_end(&mut out)).await??;
    if out != b"crm-ok" {
        return Err("сервер не ответил".into());
    }
    Ok(started.elapsed())
}

async fn client(id: &str, direct: &[String]) -> Result<(), AnyError> {
    let direct: Vec<SocketAddr> = direct.iter().filter_map(|a| a.parse().ok()).collect();
    let client = Client::open(
        0,
        Dial {
            id: id.to_owned(),
            direct,
            relay: Relay::Public,
        },
    )
    .await?;
    println!("Проверяю связь 3 минуты. Можно включать и выключать VPN — смотрите, что меняется.\n");
    let (mut ok, mut fail) = (0u32, 0u32);
    let began = Instant::now();
    for n in 1..=60 {
        let at = began.elapsed().as_secs();
        let at = format!("{}:{:02}", at / 60, at % 60);
        match round_trip(client.port()).await {
            Ok(t) => {
                ok += 1;
                let how = match client.path().await {
                    Path::Direct { addr, rtt_ms } => {
                        format!("напрямую ({addr}), задержка {rtt_ms} мс")
                    }
                    Path::Relayed { rtt_ms } => format!("через ретранслятор, задержка {rtt_ms} мс"),
                    Path::None => "путь не определён".into(),
                };
                println!("{n:>2}. [{at}] ответ за {:>4} мс — {how}", t.as_millis());
            }
            Err(e) => {
                fail += 1;
                println!("{n:>2}. [{at}] НЕТ СВЯЗИ: {e}");
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    println!("\nИтог: удачно {ok}, неудачно {fail}. Пришлите этот текст разработчику.");
    Ok(())
}
