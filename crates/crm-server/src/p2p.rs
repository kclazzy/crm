//! Подключение по ключу узла: CRM отвечает программе сотрудника без открытого порта
//! на роутере и без белого IP (crm-p2p). Включает владелец; по умолчанию выключено.
//!
//! Каждый поток узла — обычное HTTP-соединение с тем же приложением, что и на порту
//! сервера. Откуда пришёл сотрудник, сервер решает сам: прямой путь из сети офиса —
//! офис, всё остальное — «из интернета» (код из приложения, лимиты — как обычно).

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::OnceLock;

use axum::extract::{ConnectInfo, State};
use axum::{Extension, Json, Router};
use crm_p2p::{Host, Peer, Relay, Stream};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::Owner;
use crate::http::{ApiError, AppState};

const KEY: &str = "p2p_enabled";

/// Работающий узел: что положить в ключ подключения.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    pub udp_port: u16,
}

/// Узел и приложение, которое он обслуживает.
#[derive(Default)]
pub struct Slot {
    host: tokio::sync::Mutex<Option<Host>>,
    router: OnceLock<Router>,
}

impl Slot {
    pub fn set_router(&self, router: Router) {
        let _ = self.router.set(router);
    }

    pub async fn node(&self) -> Option<Node> {
        self.host.lock().await.as_ref().map(|h| Node {
            id: h.id(),
            udp_port: h.udp_port(),
        })
    }

    pub async fn stop(&self) {
        if let Some(h) = self.host.lock().await.take() {
            h.stop().await;
        }
    }
}

pub async fn enabled(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(KEY)
        .fetch_optional(pool)
        .await?;
    Ok(v.as_deref() == Some("1"))
}

pub async fn set_enabled(pool: &SqlitePool, on: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings(key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(KEY)
    .bind(if on { "1" } else { "0" })
    .execute(pool)
    .await?;
    Ok(())
}

/// Начало условных адресов устройств, пришедших по ключу узла не из сети офиса.
const DEVICE_PREFIX: &str = "2001:db8:";

/// Условный адрес устройства, пришедшего по ключу сервера не из сети офиса.
/// До HTTP такое устройство доходит, только пройдя одобрение (или если владелец его выключил).
pub fn is_device_addr(ip: &str) -> bool {
    ip.starts_with(DEVICE_PREFIX)
}

/// Шаблон адресов для учёта неудачных входов. Условный адрес устройства выводится из
/// его ключа, а ключ выбирает сам подключающийся — поэтому неудачи всех таких
/// устройств считаются вместе: смена ключа не даёт новых попыток.
pub fn lockout_scope(ip: &str) -> String {
    if ip.starts_with(DEVICE_PREFIX) {
        format!("{DEVICE_PREFIX}%")
    } else {
        // Обычный адрес: точное совпадение (спецсимволов LIKE в адресах нет).
        ip.to_owned()
    }
}

/// Адрес, под которым сервер видит сотрудника: по нему решается «офис или интернет»,
/// считаются лимиты и пишется журнал входов.
///
/// Прямой путь из сети офиса — настоящий адрес. Иначе (ретранслятор, внешний адрес) —
/// условный адрес из ключа устройства: не офисный и свой у каждого устройства.
pub fn origin(peer: &Peer) -> SocketAddr {
    if let Some(a) = peer.direct {
        if crm_core::remote::is_office_addr(&a.ip().to_string()) {
            return a;
        }
    }
    // 2001:db8::/32 — адреса «для документации», в настоящих сетях не встречаются.
    let mut seg = [0x2001u16, 0x0db8, 0, 0, 0, 0, 0, 0];
    for (i, s) in seg.iter_mut().enumerate().skip(2) {
        let at = (i - 2) * 4;
        *s = peer
            .id
            .get(at..at + 4)
            .and_then(|h| u16::from_str_radix(h, 16).ok())
            .unwrap_or(0);
    }
    SocketAddr::new(IpAddr::V6(Ipv6Addr::from(seg)), 0)
}

// ---------- Одобрение устройств ----------

const APPROVAL_KEY: &str = "p2p_device_approval";
/// Больше ожидающих заявок не храним: посторонний не должен завалить список.
const MAX_PENDING: i64 = 30;

/// Пускать ли устройство к CRM.
#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    Serve,
    /// Ждёт одобрения; `code` сотрудник сообщает владельцу.
    Pending {
        code: String,
    },
}

/// Короткий код устройства для сверки по телефону: `1A2B-3C4D`.
fn device_code(id: &str) -> String {
    let head: String = id.chars().take(8).collect::<String>().to_uppercase();
    match head.split_at_checked(4) {
        Some((a, b)) => format!("{a}-{b}"),
        None => head,
    }
}

/// Нужно ли одобрение владельца (по умолчанию — да).
pub async fn approval_required(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(APPROVAL_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(v.as_deref() != Some("0"))
}

/// Решить, пускать ли устройство. Из сети офиса — всегда; вне офиса — одобренные
/// (или все, если владелец отключил одобрение). Новое устройство записывается в заявки.
pub async fn admit(pool: &SqlitePool, peer: &Peer) -> Result<Admit, sqlx::Error> {
    if crm_core::remote::is_office_addr(&origin(peer).ip().to_string())
        || !approval_required(pool).await?
    {
        return Ok(Admit::Serve);
    }
    let now = crate::auth::now_ms();
    let approved: Option<bool> =
        sqlx::query_scalar("UPDATE p2p_devices SET last_seen = ? WHERE id = ? RETURNING approved")
            .bind(now)
            .bind(&peer.id)
            .fetch_optional(pool)
            .await?;
    if approved == Some(true) {
        return Ok(Admit::Serve);
    }
    if approved.is_none() {
        sqlx::query("INSERT INTO p2p_devices(id, first_seen, last_seen) VALUES (?, ?, ?)")
            .bind(&peer.id)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?;
        // Лишние заявки — самые давние — выбрасываются.
        sqlx::query(
            "DELETE FROM p2p_devices WHERE approved = 0 AND id NOT IN (
                 SELECT id FROM p2p_devices WHERE approved = 0
                 ORDER BY last_seen DESC, rowid DESC LIMIT ?)",
        )
        .bind(MAX_PENDING)
        .execute(pool)
        .await?;
    }
    Ok(Admit::Pending {
        code: device_code(&peer.id),
    })
}

/// Программа назвала ключ своего устройства, подключаясь из сети офиса: такое
/// устройство считается своим и вне офиса одобрения не ждёт.
pub async fn seen_in_office(pool: &SqlitePool, device: &str) -> Result<(), sqlx::Error> {
    if device.len() != 64 || !device.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(());
    }
    let now = crate::auth::now_ms();
    sqlx::query(
        "INSERT INTO p2p_devices(id, approved, first_seen, last_seen) VALUES (?, 1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET approved = 1, last_seen = excluded.last_seen",
    )
    .bind(device.to_ascii_lowercase())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Новое устройство ждёт одобрения — сказать владельцу (об одном устройстве — раз в сутки).
pub async fn alert_pending(app: &AppState, peer: &Peer) -> Result<(), ApiError> {
    crate::notify::alert_owners(
        app,
        "device_pending",
        "code",
        serde_json::json!({ "code": device_code(&peer.id) }),
        24 * 60 * 60 * 1000,
    )
    .await
}

/// Страница вместо CRM для неодобренного устройства.
async fn refuse(mut stream: Stream, code: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut sink = [0u8; 8192];
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        stream.read(&mut sink),
    )
    .await;
    let body = format!(
        "<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=10><title>CRM</title>\
         <body style=\"font-family:'Segoe UI',sans-serif;max-width:560px;margin:60px auto;padding:0 24px;color:#0f172a\">\
         <h2>Это устройство ещё не одобрено</h2>\
         <p>Вы подключаетесь не из офиса. Сообщите владельцу CRM код устройства — он одобрит его \
         в разделе «Удалённый доступ»:</p>\
         <p style=\"font:700 34px/1.2 Consolas,monospace;letter-spacing:.08em\">{code}</p>\
         <p style=\"color:#475569\">Страница обновится сама, когда владелец одобрит устройство. \
         В офисе одобрение не требуется.</p>"
    );
    let resp = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Device {
    id: String,
    name: String,
    approved: bool,
    first_seen: i64,
    last_seen: i64,
    #[sqlx(skip)]
    code: String,
}

#[derive(Serialize)]
pub struct Devices {
    approval_required: bool,
    devices: Vec<Device>,
}

pub async fn devices(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<Devices>, ApiError> {
    let mut devices: Vec<Device> = sqlx::query_as(
        "SELECT id, name, approved, first_seen, last_seen FROM p2p_devices
         ORDER BY approved, last_seen DESC",
    )
    .fetch_all(&app.pool)
    .await?;
    for d in &mut devices {
        d.code = device_code(&d.id);
    }
    Ok(Json(Devices {
        approval_required: approval_required(&app.pool).await?,
        devices,
    }))
}

#[derive(Deserialize)]
pub struct ApprovalIn {
    approval_required: bool,
}

pub async fn set_approval(
    State(app): State<AppState>,
    owner: Owner,
    Json(req): Json<ApprovalIn>,
) -> Result<Json<Devices>, ApiError> {
    sqlx::query(
        "INSERT INTO settings(key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(APPROVAL_KEY)
    .bind(if req.approval_required { "1" } else { "0" })
    .execute(&app.pool)
    .await?;
    devices(State(app), owner).await
}

#[derive(Deserialize)]
pub struct DeviceIn {
    approved: bool,
    #[serde(default)]
    name: String,
}

pub async fn set_device(
    State(app): State<AppState>,
    Owner(_): Owner,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<DeviceIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let name: String = req.name.trim().chars().take(80).collect();
    let done = sqlx::query("UPDATE p2p_devices SET approved = ?, name = ? WHERE id = ?")
        .bind(req.approved)
        .bind(name)
        .bind(&id)
        .execute(&app.pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn remove_device(
    State(app): State<AppState>,
    Owner(_): Owner,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    sqlx::query("DELETE FROM p2p_devices WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn serve_stream(router: Router, app: AppState, peer: Peer, stream: Stream) {
    use hyper_util::rt::{TokioExecutor, TokioIo};
    match admit(&app.pool, &peer).await {
        Ok(Admit::Serve) => {}
        Ok(Admit::Pending { code }) => {
            let _ = alert_pending(&app, &peer).await;
            return refuse(stream, &code).await;
        }
        // База недоступна — не пускаем: безопаснее отказать, чем пропустить.
        Err(_) => return,
    }
    let app = router.layer(Extension(ConnectInfo(origin(&peer))));
    let service = hyper_util::service::TowerToHyperService::new(app);
    let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
    crate::server::limit_slow_clients(&mut builder);
    let _ = builder
        .serve_connection_with_upgrades(TokioIo::new(stream), service)
        .await;
}

/// Секрет узла: создаётся один раз и лежит рядом с базой (папка закрыта правами).
fn node_key(dir: &std::path::Path) -> std::io::Result<[u8; 32]> {
    let path = dir.join("p2p.key");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Ok(key);
        }
    }
    let key = crm_p2p::new_key();
    std::fs::write(&path, key)?;
    Ok(key)
}

/// Привести узел в соответствие с настройкой: запустить или остановить.
pub async fn apply(app: &AppState) -> Result<Option<Node>, String> {
    let slot = &app.rt.p2p;
    let on = enabled(&app.pool).await.map_err(|e| e.to_string())?;
    let mut host = slot.host.lock().await;
    if !on {
        if let Some(h) = host.take() {
            h.stop().await;
        }
        return Ok(None);
    }
    if host.is_none() {
        let router = slot.router.get().cloned().ok_or("сервер ещё запускается")?;
        let cfg = app.rt.config();
        let key = node_key(&cfg.storage.db_dir).map_err(|e| format!("ключ узла: {e}"))?;
        let relay = if cfg.p2p_relay {
            Relay::Public
        } else {
            Relay::Off
        };
        let serve = |port: u16| {
            let (router, app) = (router.clone(), app.clone());
            Host::serve(key, port, relay, move |peer, stream| {
                serve_stream(router.clone(), app.clone(), peer, stream)
            })
        };
        // Постоянный порт (как у сервера) — чтобы в офисе подключаться напрямую;
        // занят — любой свободный: через интернет узел найдётся и так.
        let started = match serve(cfg.listen.port()).await {
            Ok(h) => h,
            Err(_) => serve(0).await.map_err(|e| e.to_string())?,
        };
        *host = Some(started);
    }
    Ok(host.as_ref().map(|h| Node {
        id: h.id(),
        udp_port: h.udp_port(),
    }))
}

#[derive(Serialize)]
pub struct Info {
    enabled: bool,
    /// Узел работает — новые ключи подключения содержат его адрес.
    running: bool,
}

pub async fn info(State(app): State<AppState>, Owner(_): Owner) -> Result<Json<Info>, ApiError> {
    Ok(Json(Info {
        enabled: enabled(&app.pool).await?,
        running: app.rt.p2p.node().await.is_some(),
    }))
}

#[derive(Deserialize)]
pub struct SetIn {
    enabled: bool,
}

pub async fn set(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<SetIn>,
) -> Result<Json<Info>, ApiError> {
    set_enabled(&app.pool, req.enabled).await?;
    let node = apply(&app)
        .await
        .map_err(|e| ApiError::Failed(format!("подключение по ключу: {e}")))?;
    Ok(Json(Info {
        enabled: req.enabled,
        running: node.is_some(),
    }))
}
