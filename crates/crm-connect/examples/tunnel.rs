//! Ручная проверка: `cargo run -p crm-connect --example tunnel -- АДРЕС ПОРТ ОТПЕЧАТОК ЛОКАЛЬНЫЙ_ПОРТ`.
use crm_connect::tunnel::{Target, Tunnel};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (addr, port, fp, local) = match a.as_slice() {
        [_, addr, port, fp, local] => (
            addr.clone(),
            port.parse().unwrap_or(8443),
            fp.clone(),
            local.parse().unwrap_or(0),
        ),
        _ => {
            eprintln!("использование: tunnel АДРЕС ПОРТ ОТПЕЧАТОК ЛОКАЛЬНЫЙ_ПОРТ");
            return Ok(());
        }
    };
    let t = Tunnel::open(local, Target::Tls { addr, port, fp }).await?;
    println!("{}", t.url());
    std::future::pending::<()>().await;
    Ok(())
}
