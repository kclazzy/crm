//! VPN WireGuard из админ-панели (ТЗ §9): ключи, файлы и QR-коды для сотрудников.
//!
//! Сервер пишет конфигурацию туннеля в папку данных; включает её на Windows
//! пункт трея «Применить настройки VPN» (одно подтверждение администратора).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use crm_core::vpn::{client_conf, next_address, parse_endpoint, server_conf, Peer, DEFAULT_PORT};
use serde::{Deserialize, Serialize};

use crate::auth::{new_id, now_ms, Owner};
use crate::http::{ApiError, AppState};

pub use crm_core::vpn::SERVER_IP;

const KEY: &str = "vpn";

pub fn is_vpn_ip(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if crm_core::vpn::is_vpn_ip(v4))
}

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    endpoint: String,
    port: u16,
    server_public: String,
}

fn dir(app: &AppState) -> PathBuf {
    app.rt.config().storage.db_dir.join("wg")
}

/// Путь к файлу туннеля — его включает пункт трея.
pub fn conf_path(db_dir: &std::path::Path) -> PathBuf {
    db_dir
        .join("wg")
        .join(format!("{}.conf", crm_core::vpn::TUNNEL))
}

pub fn is_configured(app: &AppState) -> bool {
    dir(app).join("server.key").exists()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Пара ключей x25519 в формате WireGuard (base64).
fn keypair() -> Result<(String, String), ApiError> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|e| ApiError::Failed(format!("генератор случайных чисел: {e}")))?;
    // Ограничение битов по RFC 7748 — так же хранит ключи сам WireGuard.
    secret[0] &= 248;
    secret[31] &= 127;
    secret[31] |= 64;
    let public = x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES);
    Ok((b64(&secret), b64(&public)))
}

async fn settings(app: &AppState) -> Result<Option<Settings>, ApiError> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(KEY)
        .fetch_optional(&app.pool)
        .await?;
    raw.map(|r| {
        serde_json::from_str(&r).map_err(|e| ApiError::Corrupt(format!("настройки VPN: {e}")))
    })
    .transpose()
}

fn io(e: std::io::Error) -> ApiError {
    ApiError::Failed(format!("файлы VPN: {e}"))
}

/// Переписать конфигурацию сервера по текущему списку устройств.
async fn write_server_conf(app: &AppState, s: &Settings) -> Result<(), ApiError> {
    let d = dir(app);
    let private = std::fs::read_to_string(d.join("server.key")).map_err(io)?;
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT name, public_key, address FROM vpn_peers ORDER BY address")
            .fetch_all(&app.pool)
            .await?;
    let peers: Vec<Peer<'_>> = rows
        .iter()
        .filter_map(|(name, key, addr)| {
            Some(Peer {
                name,
                public_key: key,
                address: addr.parse().ok()?,
            })
        })
        .collect();
    let text = server_conf(private.trim(), s.port, &peers);
    let path = conf_path(&app.rt.config().storage.db_dir);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PeerOut {
    id: String,
    name: String,
    user_name: Option<String>,
    address: String,
    created_at: i64,
    last_seen_at: Option<i64>,
}

#[derive(Serialize)]
pub struct VpnInfo {
    configured: bool,
    endpoint: Option<String>,
    port: u16,
    /// Туннель поднят на этом компьютере (есть интерфейс 10.77.0.1).
    tunnel_up: bool,
    conf_path: String,
    peers: Vec<PeerOut>,
}

fn tunnel_up() -> bool {
    if_addrs::get_if_addrs()
        .map(|v| v.iter().any(|i| i.ip() == IpAddr::V4(SERVER_IP)))
        .unwrap_or(false)
}

pub async fn info(State(app): State<AppState>, Owner(_): Owner) -> Result<Json<VpnInfo>, ApiError> {
    let s = settings(&app).await?;
    let peers = sqlx::query_as(
        "SELECT p.id, p.name, u.display_name AS user_name, p.address, p.created_at, p.last_seen_at
         FROM vpn_peers p LEFT JOIN users u ON u.id = p.user_id ORDER BY p.created_at",
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(VpnInfo {
        configured: s.is_some() && is_configured(&app),
        endpoint: s.as_ref().map(|s| s.endpoint.clone()),
        port: s.as_ref().map_or(DEFAULT_PORT, |s| s.port),
        tunnel_up: tunnel_up(),
        conf_path: conf_path(&app.rt.config().storage.db_dir)
            .display()
            .to_string(),
        peers,
    }))
}

#[derive(Deserialize)]
pub struct SetupIn {
    endpoint: String,
    #[serde(default)]
    port: Option<u16>,
}

/// Шаг 1 мастера: внешний адрес офиса. Ключи сервера создаются один раз.
pub async fn setup(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<SetupIn>,
) -> Result<StatusCode, ApiError> {
    let endpoint = parse_endpoint(&req.endpoint).map_err(|e| ApiError::BadRequest(e.into()))?;
    let port = req.port.unwrap_or(DEFAULT_PORT);
    if port < 1024 {
        return Err(ApiError::BadRequest("порт VPN: от 1024 до 65535".into()));
    }
    let d = dir(&app);
    std::fs::create_dir_all(&d).map_err(io)?;
    crate::db::make_private(&d).map_err(|e| ApiError::Failed(e.to_string()))?;
    let key_file = d.join("server.key");
    let server_public = match settings(&app).await? {
        Some(s) if key_file.exists() => s.server_public,
        _ => {
            let (private, public) = keypair()?;
            std::fs::write(&key_file, private).map_err(io)?;
            public
        }
    };
    let s = Settings {
        endpoint,
        port,
        server_public,
    };
    let json = serde_json::to_string(&s).map_err(|e| ApiError::Failed(e.to_string()))?;
    sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(KEY)
        .bind(json)
        .execute(&app.pool)
        .await?;
    write_server_conf(&app, &s).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PeerIn {
    name: String,
    user_id: Option<String>,
}

#[derive(Serialize)]
pub struct NewPeer {
    id: String,
    address: String,
    /// Содержимое файла для приложения WireGuard — показывается один раз.
    conf: String,
    file_name: String,
    qr_svg: String,
}

/// Выдать доступ: ключи устройства, файл и QR-код. Закрытый ключ устройства не сохраняется.
pub async fn add_peer(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<PeerIn>,
) -> Result<(StatusCode, Json<NewPeer>), ApiError> {
    let s = settings(&app)
        .await?
        .ok_or_else(|| ApiError::BadRequest("сначала укажите внешний адрес офиса".into()))?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::BadRequest(
            "название устройства: 1–60 символов".into(),
        ));
    }
    let taken: Vec<String> = sqlx::query_scalar("SELECT address FROM vpn_peers")
        .fetch_all(&app.pool)
        .await?;
    let taken: Vec<Ipv4Addr> = taken.iter().filter_map(|a| a.parse().ok()).collect();
    let address = next_address(&taken).ok_or(ApiError::Conflict(
        "адреса VPN закончились (253 устройства)",
    ))?;
    let (private, public) = keypair()?;
    let id = new_id();
    sqlx::query(
        "INSERT INTO vpn_peers(id, user_id, name, public_key, address, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.user_id)
    .bind(name)
    .bind(&public)
    .bind(address.to_string())
    .bind(now_ms())
    .execute(&app.pool)
    .await?;
    write_server_conf(&app, &s).await?;
    let conf = client_conf(&private, address, &s.server_public, &s.endpoint, s.port);
    let qr_svg = qrcode::QrCode::new(conf.as_bytes())
        .map_err(|e| ApiError::Failed(format!("QR-код: {e}")))?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .build();
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    Ok((
        StatusCode::CREATED,
        Json(NewPeer {
            id,
            address: address.to_string(),
            conf,
            file_name: format!("CRM-{safe}.conf"),
            qr_svg,
        }),
    ))
}

/// Отозвать доступ устройства.
pub async fn remove_peer(
    State(app): State<AppState>,
    Owner(_): Owner,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("DELETE FROM vpn_peers WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    if let Some(s) = settings(&app).await? {
        write_server_conf(&app, &s).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Когда устройство последний раз заходило через VPN — пишем не чаще раза в минуту.
#[derive(Default)]
pub struct Seen(std::sync::Mutex<HashMap<Ipv4Addr, i64>>);

pub async fn track(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let IpAddr::V4(ip) = addr.ip() {
        if crm_core::vpn::is_vpn_ip(ip) && ip != SERVER_IP {
            let now = now_ms();
            let due = match app.rt.vpn_seen.0.lock() {
                Ok(mut m) => {
                    let last = m.get(&ip).copied().unwrap_or(0);
                    if now - last > 60_000 {
                        m.insert(ip, now);
                        true
                    } else {
                        false
                    }
                }
                Err(_) => false,
            };
            if due {
                let _ = sqlx::query("UPDATE vpn_peers SET last_seen_at = ? WHERE address = ?")
                    .bind(now)
                    .bind(ip.to_string())
                    .execute(&app.pool)
                    .await;
            }
        }
    }
    next.run(req).await
}
