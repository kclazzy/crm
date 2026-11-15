//! Ключ подключения для программы сотрудника (ТЗ §2.3).

use axum::extract::State;
use axum::Json;
use crm_core::connkey::{encode, ServerKey};
use serde::{Deserialize, Serialize};

use crate::auth::{issue_invite, Owner};
use crate::http::{ApiError, AppState};

#[derive(Deserialize)]
pub struct KeyIn {
    /// Для приглашённого сотрудника — ключ сразу с приглашением (старое перестаёт работать).
    user_id: Option<String>,
    /// Рекомендуется: PIN сообщают отдельно от ключа (голосом, в мессенджере).
    pin: Option<String>,
}

#[derive(Serialize)]
pub struct KeyOut {
    key: String,
    with_pin: bool,
    /// До какого времени действует приглашение внутри ключа.
    invite_expires_at: Option<i64>,
    addrs: Vec<String>,
}

/// Адреса сервера в порядке попыток: имена, IP офисной сети, внешний адрес — последним.
pub async fn server_addrs(app: &AppState) -> Result<Vec<String>, ApiError> {
    let sans = crate::tls::local_sans();
    let names = sans.names.iter().filter(|n| *n != "localhost").cloned();
    let ips = sans
        .ips
        .iter()
        .filter(|ip| ip.is_ipv4() && !ip.is_loopback())
        .map(ToString::to_string);
    let mut out: Vec<String> = names.chain(ips).collect();
    if let Some(a) = crate::remote::address(&app.pool).await? {
        out.retain(|x| *x != a);
        out.push(a);
    }
    Ok(out)
}

pub async fn connection_key(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<KeyIn>,
) -> Result<Json<KeyOut>, ApiError> {
    let fp = app
        .ca_fingerprint
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("ключ работает только с включённым HTTPS".into()))?
        .to_owned();
    let pin = req.pin.as_deref().map(str::trim).filter(|p| !p.is_empty());
    let mut invite = None;
    let mut user = None;
    let mut expires = None;
    if let Some(id) = &req.user_id {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT username, password_hash FROM users WHERE id = ? AND is_active = 1",
        )
        .bind(id)
        .fetch_optional(&app.pool)
        .await?;
        let (username, hash) = row.ok_or(ApiError::NotFound)?;
        if hash.is_some() {
            return Err(ApiError::Conflict(
                "сотрудник уже задал пароль — дайте ему общий ключ без приглашения",
            ));
        }
        let mut tx = app.pool.begin().await?;
        let inv = issue_invite(&mut tx, id).await?;
        tx.commit().await?;
        invite = Some(inv.invite_token);
        expires = Some(inv.invite_expires_at);
        user = Some(username);
    }
    let addrs = server_addrs(&app).await?;
    let key = ServerKey {
        name: gethostname::gethostname().to_string_lossy().into_owned(),
        addrs: addrs.clone(),
        port: app.rt.config().listen.port(),
        fp,
        invite,
        user,
    };
    let mut random = [0u8; 40];
    getrandom::fill(&mut random)
        .map_err(|e| ApiError::Failed(format!("генератор случайных чисел: {e}")))?;
    let key = encode(&key, pin, random).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(KeyOut {
        key,
        with_pin: pin.is_some(),
        invite_expires_at: expires,
        addrs,
    }))
}

#[derive(Serialize)]
pub struct PhoneLink {
    /// «cert», «office», «internet».
    kind: &'static str,
    url: String,
    qr_svg: String,
}

fn qr(text: &str) -> Result<String, ApiError> {
    Ok(qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| ApiError::Failed(format!("QR-код: {e}")))?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .build())
}

/// Ссылки и QR-коды для телефона: установить сертификат, открыть CRM в офисе и из интернета.
/// На телефонах имя компьютера часто не находится — поэтому IP-адреса.
pub async fn phone_links(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<Vec<PhoneLink>>, ApiError> {
    let port = app.rt.config().listen.port();
    let addrs = server_addrs(&app).await?;
    let office = addrs
        .iter()
        .find(|a| a.parse::<std::net::Ipv4Addr>().is_ok() && crm_core::remote::is_office_addr(a));
    let internet = crate::remote::address(&app.pool).await?;
    let mut out = Vec::new();
    // Сертификат ставят там, где телефон сейчас: в офисе или из интернета.
    if let Some(host) = office.or(internet.as_ref()) {
        let cert = format!("https://{host}:{port}/api/tls/ca");
        out.push(PhoneLink {
            kind: "cert",
            qr_svg: qr(&cert)?,
            url: cert,
        });
    }
    if let Some(ip) = office {
        let url = format!("https://{ip}:{port}/");
        out.push(PhoneLink {
            kind: "office",
            qr_svg: qr(&url)?,
            url,
        });
    }
    if let Some(host) = &internet {
        let url = format!("https://{host}:{port}/");
        out.push(PhoneLink {
            kind: "internet",
            qr_svg: qr(&url)?,
            url,
        });
    }
    Ok(Json(out))
}
