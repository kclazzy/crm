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

async fn serve_stream(router: Router, peer: Peer, stream: Stream) {
    use hyper_util::rt::{TokioExecutor, TokioIo};
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
            let router = router.clone();
            Host::serve(key, port, relay, move |peer, stream| {
                serve_stream(router.clone(), peer, stream)
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
