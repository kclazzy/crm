//! Роутер в один клик (UPnP): открыть TCP-порт сервера из интернета на этот компьютер
//! и узнать внешний адрес офиса. Работает, если на роутере включён UPnP (так делают
//! Keenetic, TP-Link, ASUS и большинство домашних и офисных роутеров).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use igd_next::aio::tokio::search_gateway;
use igd_next::{PortMappingProtocol, SearchOptions};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::auth::Owner;
use crate::http::{ApiError, AppState};

const KEY: &str = "upnp";
const SEARCH: Duration = Duration::from_secs(4);
/// Сколько держать правило, если роутер не берёт «навсегда»; продлеваем раньше.
const LEASE_SECS: u32 = 7 * 24 * 3600;
const RENEW_EVERY: Duration = Duration::from_secs(6 * 3600);
const DESCRIPTION: &str = "CRM";

const NO_ROUTER: &str = "Роутер не ответил. Включите на нём UPnP (обычно «Домашняя сеть → UPnP» или «NAT → UPnP») и нажмите ещё раз — или создайте правило вручную, шаг 2.";

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Адрес роутера в интернете, как он его называет.
    external_ip: Option<String>,
    /// Адрес «белый» — снаружи до него достучаться можно.
    public: bool,
    /// Куда роутер ведёт порт — этот компьютер.
    local_ip: String,
    port: u16,
    /// Внешний адрес, записанный в «Удалённый доступ» (подставлен сам, если был пуст).
    address: Option<String>,
}

/// Белый ли внешний адрес роутера: «серый» (100.64.x, 10.x…) значит, что провайдер
/// держит офис за своим NAT — переадресация на роутере не поможет.
pub fn is_public(external: IpAddr) -> bool {
    !crm_core::remote::is_office_addr(&external.to_string())
}

fn search_options() -> SearchOptions {
    let mut o = SearchOptions::default();
    o.timeout = Some(SEARCH);
    o.single_search_timeout = Some(SEARCH);
    o
}

async fn open_port(port: u16) -> Result<(Option<IpAddr>, IpAddr), ApiError> {
    let gw = search_gateway(search_options())
        .await
        .map_err(|_| ApiError::Upstream(NO_ROUTER.into()))?;
    // IP этого компьютера в сети роутера — откуда идём к нему.
    let probe = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| ApiError::Failed(format!("сеть: {e}")))?;
    probe
        .connect(gw.addr)
        .await
        .map_err(|e| ApiError::Failed(format!("сеть до роутера: {e}")))?;
    let local_ip = probe
        .local_addr()
        .map_err(|e| ApiError::Failed(format!("сеть: {e}")))?
        .ip();
    let to = SocketAddr::new(local_ip, port);
    let first = gw
        .add_port(PortMappingProtocol::TCP, port, to, 0, DESCRIPTION)
        .await;
    if first.is_err() {
        gw.add_port(PortMappingProtocol::TCP, port, to, LEASE_SECS, DESCRIPTION)
            .await
            .map_err(|e| {
                ApiError::Upstream(format!(
                    "Роутер не открыл порт {port}: {e}. Возможно, порт уже занят другим правилом — проверьте «Переадресацию портов» на роутере."
                ))
            })?;
    }
    Ok((gw.get_external_ip().await.ok(), local_ip))
}

async fn set_enabled(pool: &SqlitePool, on: bool) -> Result<(), ApiError> {
    if on {
        sqlx::query(
            "INSERT INTO settings(key, value) VALUES (?, '1') ON CONFLICT(key) DO UPDATE SET value = '1'",
        )
        .bind(KEY)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(KEY)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// «Настроить роутер автоматически».
pub async fn open(State(app): State<AppState>, Owner(_): Owner) -> Result<Json<Opened>, ApiError> {
    let port = app.rt.config().listen.port();
    let (external, local_ip) = open_port(port).await?;
    set_enabled(&app.pool, true).await?;
    let public = external.is_some_and(is_public);
    // Внешний адрес ещё не задан, а роутер назвал белый — подставляем сами.
    let mut address = crate::remote::address(&app.pool).await?;
    if address.is_none() && public {
        if let Some(ip) = external {
            sqlx::query(
                "INSERT INTO settings(key, value) VALUES ('remote_address', ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(ip.to_string())
            .execute(&app.pool)
            .await?;
            app.rt.tls_refresh.notify_one();
            address = Some(ip.to_string());
        }
    }
    Ok(Json(Opened {
        external_ip: external.map(|i| i.to_string()),
        public,
        local_ip: local_ip.to_string(),
        port,
        address,
    }))
}

/// Убрать правило с роутера.
pub async fn close(State(app): State<AppState>, Owner(_): Owner) -> Result<StatusCode, ApiError> {
    set_enabled(&app.pool, false).await?;
    let port = app.rt.config().listen.port();
    if let Ok(gw) = search_gateway(search_options()).await {
        let _ = gw.remove_port(PortMappingProtocol::TCP, port).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Продлевать правило, пока оно включено: роутер мог перезагрузиться или выдать срок.
pub async fn keep_alive(app: AppState) {
    loop {
        tokio::time::sleep(RENEW_EVERY).await;
        let on: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(KEY)
            .fetch_optional(&app.pool)
            .await
            .ok()
            .flatten();
        if on.is_some() {
            if let Err(e) = open_port(app.rt.config().listen.port()).await {
                tracing::warn!(error = %e, "продление правила на роутере");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_addresses_are_not_public() {
        assert!(is_public("178.206.250.161".parse().unwrap()));
        for grey in ["100.72.1.1", "10.0.0.1", "192.168.1.1"] {
            assert!(!is_public(grey.parse().unwrap()), "{grey}");
        }
    }
}
