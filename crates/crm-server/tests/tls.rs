//! Сервер по HTTPS глазами клиента: доверяем CA сервера — соединение есть, нет — отказ.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use crm_server::config::Config;
use crm_server::server;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

fn config(dir: &std::path::Path, tls: bool) -> Config {
    Config::from_toml(&format!(
        "listen = \"127.0.0.1:0\"\n[tls]\nenabled = {tls}\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.join("data"),
        dir.join("files"),
    ))
    .unwrap()
}

/// Сырой HTTP/1.1 GET поверх TLS с заданным набором доверенных CA.
async fn https_get(
    addr: std::net::SocketAddr,
    host: &str,
    trusted: &[CertificateDer<'static>],
    path: &str,
) -> Result<String, std::io::Error> {
    let mut roots = rustls::RootCertStore::empty();
    for c in trusted {
        roots.add(c.clone()).unwrap();
    }
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let mut tls = TlsConnector::from(Arc::new(cfg))
        .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp)
        .await?;
    tls.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut out = String::new();
    tls.read_to_string(&mut out).await?;
    Ok(out)
}

fn ca_of(dir: &std::path::Path) -> CertificateDer<'static> {
    CertificateDer::from_pem_file(dir.join("data").join("tls").join("ca.crt")).unwrap()
}

#[tokio::test]
async fn client_trusting_server_ca_connects_by_name_and_ip() {
    let dir = tempfile::tempdir().unwrap();
    let running = server::start(&config(dir.path(), true)).await.unwrap();
    let ca = ca_of(dir.path());

    let by_name = https_get(
        running.addr,
        "localhost",
        std::slice::from_ref(&ca),
        "/api/health",
    )
    .await
    .unwrap();
    assert!(by_name.starts_with("HTTP/1.1 200"), "{by_name}");
    let by_ip = https_get(running.addr, "127.0.0.1", &[ca], "/api/health")
        .await
        .unwrap();
    assert!(by_ip.starts_with("HTTP/1.1 200"), "{by_ip}");
    running.stop().await.unwrap();
}

#[tokio::test]
async fn client_not_trusting_ca_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let running = server::start(&config(dir.path(), true)).await.unwrap();
    let stranger = tempfile::tempdir().unwrap();
    let other = crm_server::tls::ensure(
        stranger.path(),
        &crm_server::tls::local_sans(),
        time::OffsetDateTime::now_utc(),
    )
    .unwrap();
    let other_ca = CertificateDer::from_pem_slice(other.ca_pem.as_bytes()).unwrap();

    assert!(
        https_get(running.addr, "localhost", &[other_ca], "/api/health")
            .await
            .is_err()
    );
    running.stop().await.unwrap();
}

#[tokio::test]
async fn ca_certificate_can_be_downloaded_for_browser_install() {
    let dir = tempfile::tempdir().unwrap();
    let running = server::start(&config(dir.path(), true)).await.unwrap();
    let res = https_get(
        running.addr,
        "localhost",
        &[ca_of(dir.path())],
        "/api/tls/ca",
    )
    .await
    .unwrap();
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    assert!(res
        .to_lowercase()
        .contains("content-disposition: attachment; filename=\"crm-ca.crt\""));
    let file = std::fs::read_to_string(dir.path().join("data/tls/ca.crt")).unwrap();
    assert!(res.ends_with(&file));
    assert_eq!(running.ca_fingerprint.as_deref().map(str::len), Some(95));
    let health = https_get(
        running.addr,
        "localhost",
        &[ca_of(dir.path())],
        "/api/health",
    )
    .await
    .unwrap();
    let fp = running.ca_fingerprint.clone().unwrap();
    assert!(
        health.contains(&fp),
        "отпечаток CA виден в /api/health для сверки: {health}"
    );
    running.stop().await.unwrap();
}

#[test]
fn network_listening_without_tls_is_refused() {
    let insecure = "listen = \"0.0.0.0:8443\"\n[tls]\nenabled = false\n";
    let err = Config::from_toml(insecure).err().unwrap();
    assert!(err.to_string().contains("HTTPS"), "{err}");
    assert!(Config::from_toml("listen = \"127.0.0.1:8443\"\n[tls]\nenabled = false\n").is_ok());
    assert!(
        Config::from_toml("listen = \"0.0.0.0:8443\"\n").is_ok(),
        "по умолчанию TLS включён"
    );
}

#[tokio::test]
async fn server_waits_for_port_freed_by_previous_process() {
    // Как при перезапуске службы: порт ещё занят старым процессом и освобождается чуть позже.
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::from_toml(&format!(
        "listen = \"127.0.0.1:{port}\"\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\n",
        dir.path().join("data"),
        dir.path().join("files"),
    ))
    .unwrap();
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        drop(busy);
    });
    let running = server::start(&cfg).await.unwrap();
    assert_eq!(running.addr.port(), port);
    release.await.unwrap();
    running.stop().await.unwrap();
}
