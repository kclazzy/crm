//! Вход: первый владелец, приглашения, логин, сессии (ТЗ §3.2).

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::header::SET_COOKIE;
use axum::http::request::Parts;
use axum::http::{HeaderName, StatusCode};
use axum::response::AppendHeaders;
use axum::Json;
use chrono::{Duration, Utc};
use crm_core::auth::{check_password, parse_display_name, Role, Username};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::http::{ApiError, AppState};
use crate::secrets::{hash_password, new_token, token_hash, verify_password};
use crm_core::totp::Login;

pub const INVITE_TTL: Duration = Duration::hours(72);
pub const SESSION_TTL: Duration = Duration::days(30);
pub const LOCKOUT_WINDOW: Duration = Duration::minutes(15);
pub const LOCKOUT_LIMIT: i64 = 5;
/// Как часто продлевать сессию (чтобы не писать в БД на каждый запрос).
const TOUCH_EVERY: Duration = Duration::minutes(1);

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

// ---------- Кто делает запрос ----------

#[derive(Debug, Clone, Serialize)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: Role,
    #[serde(skip)]
    pub session_id: String,
}

/// Сессия и её владелец — одной строкой из базы.
#[derive(sqlx::FromRow)]
struct SessionRow {
    session_id: String,
    id: String,
    username: String,
    display_name: String,
    role: String,
    last_seen_at: i64,
    needs_setup: bool,
    mfa_ok: bool,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &AppState) -> Result<Self, ApiError> {
        let token = bearer(parts)
            .or_else(|| session_cookie(parts))
            .ok_or(ApiError::Unauthorized)?;
        let now = now_ms();
        let row: Option<SessionRow> = sqlx::query_as(
            "SELECT s.id AS session_id, u.id, u.username, u.display_name, u.role, s.last_seen_at,
                    s.needs_2fa_setup AS needs_setup, s.mfa_ok
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.revoked_at IS NULL AND s.expires_at > ? AND u.is_active = 1",
        )
        .bind(token_hash(token))
        .bind(now)
        .fetch_optional(&app.pool)
        .await?;
        let SessionRow {
            session_id,
            id,
            username,
            display_name,
            role,
            last_seen_at: last_seen,
            needs_setup,
            mfa_ok,
        } = row.ok_or(ApiError::Unauthorized)?;
        // Вошли в офисе без кода, а запрос пришёл из интернета: та же проверка, что при входе.
        let path = parts.uri.path();
        if !mfa_ok && path != "/api/auth/verify" && path != "/api/auth/logout" {
            let from = parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string());
            if from.is_some_and(|ip| !crm_core::remote::is_office_addr(&ip)) {
                match crm_core::totp::login_rule(
                    crate::two_factor::policy(&app.pool).await?,
                    crate::two_factor::enrolled(&app.pool, &id).await?,
                    false,
                ) {
                    Login::Allow => {}
                    Login::NeedCode => return Err(ApiError::NeedCode),
                    Login::Setup | Login::Refuse => return Err(ApiError::TwoFactorInOffice),
                }
            }
        }
        // Организация требует код, а он ещё не настроен: доступны только профиль и выход.
        if needs_setup
            && !(path == "/api/me" || path.starts_with("/api/me/2fa") || path == "/api/auth/logout")
        {
            return Err(ApiError::SetupTwoFactor);
        }

        if now - last_seen > TOUCH_EVERY.num_milliseconds() {
            sqlx::query("UPDATE sessions SET last_seen_at = ?, expires_at = ? WHERE id = ?")
                .bind(now)
                .bind(now + SESSION_TTL.num_milliseconds())
                .bind(&session_id)
                .execute(&app.pool)
                .await?;
        }
        Ok(Self {
            id,
            username,
            display_name,
            role: role
                .parse()
                .map_err(|_| ApiError::Corrupt(format!("роль «{role}»")))?,
            session_id,
        })
    }
}

/// Токен из заголовка `Authorization: Bearer` (приложение, скрипты).
fn bearer(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Токен из cookie браузера.
fn session_cookie(parts: &Parts) -> Option<&str> {
    let prefix = format!("{COOKIE}=");
    parts
        .headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(prefix.as_str()))
}

pub const COOKIE: &str = "crm_session";

/// Cookie сессии для браузера: недоступна скриптам (HttpOnly), только по HTTPS,
/// не уходит с запросами с чужих сайтов (SameSite=Strict — защита от CSRF).
fn set_cookie(token: &str) -> String {
    format!(
        "{COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={}",
        SESSION_TTL.num_seconds()
    )
}

fn clear_cookie() -> String {
    format!("{COOKIE}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0")
}

type WithCookie<T> = (AppendHeaders<[(HeaderName, String); 1]>, T);

fn with_cookie<T>(cookie: String, body: T) -> WithCookie<T> {
    (AppendHeaders([(SET_COOKIE, cookie)]), body)
}

/// Пользователь с ролью владельца.
pub struct Owner(pub CurrentUser);

impl FromRequestParts<AppState> for Owner {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &AppState) -> Result<Self, ApiError> {
        let user = CurrentUser::from_request_parts(parts, app).await?;
        if user.role == Role::Host {
            Ok(Self(user))
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

// ---------- Общие куски ----------

#[derive(Deserialize)]
pub struct Device {
    device_id: String,
    device_name: String,
}

impl Device {
    fn check(&self) -> Result<(), ApiError> {
        let ok = |s: &str, max: usize| !s.trim().is_empty() && s.chars().count() <= max;
        if ok(&self.device_id, 128) && ok(&self.device_name, 100) {
            Ok(())
        } else {
            Err(ApiError::BadRequest("некорректное устройство".into()))
        }
    }
}

#[derive(Serialize)]
pub struct SessionOut {
    token: String,
    user: CurrentUser,
    /// До настройки кода из приложения доступен только профиль.
    must_setup_2fa: bool,
}

/// Не больше [`LOCKOUT_LIMIT`] неудач действия `action` для пары «логин + IP» за [`LOCKOUT_WINDOW`].
pub(crate) async fn ensure_not_locked(
    pool: &SqlitePool,
    action: &str,
    key: &str,
    ip: &str,
) -> Result<(), ApiError> {
    let since = now_ms() - LOCKOUT_WINDOW.num_milliseconds();
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log
         WHERE action = ? AND entity_id = ? AND ip LIKE ? AND created_at > ?",
    )
    .bind(action)
    .bind(key)
    .bind(crate::p2p::lockout_scope(ip))
    .bind(since)
    .fetch_one(pool)
    .await?;
    if failures >= LOCKOUT_LIMIT {
        Err(ApiError::TooManyAttempts)
    } else {
        Ok(())
    }
}

/// Откуда пришёл человек — для уведомлений владельцу.
fn place(ip: &str) -> &'static str {
    if crate::p2p::is_device_addr(ip) {
        "device"
    } else if crm_core::remote::is_office_addr(ip) {
        "office"
    } else {
        "internet"
    }
}

/// Неудачная попытка входа уже записана в журнал: сказать владельцу — о первой
/// за окно блокировки и о самой блокировке (по одному уведомлению на логин).
async fn alert_failed_login(app: &AppState, key: &str, ip: &str) -> Result<(), ApiError> {
    let window = LOCKOUT_WINDOW.num_milliseconds();
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'login.failed' AND entity_id = ? AND created_at > ?",
    )
    .bind(key)
    .bind(now_ms() - window)
    .fetch_one(&app.pool)
    .await?;
    let payload = serde_json::json!({ "username": key, "place": place(ip) });
    crate::notify::alert_owners(app, "login_failed", "username", payload.clone(), window).await?;
    if failures >= LOCKOUT_LIMIT {
        crate::notify::alert_owners(app, "login_locked", "username", payload, window).await?;
    }
    Ok(())
}

/// Медленный перебор разных логинов с одного IP: не больше 20 неудач за 15 минут.
pub(crate) async fn ensure_ip_not_spraying(pool: &SqlitePool, ip: &str) -> Result<(), ApiError> {
    const IP_LIMIT: i64 = 20;
    let since = now_ms() - LOCKOUT_WINDOW.num_milliseconds();
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'login.failed' AND ip LIKE ? AND created_at > ?",
    )
    .bind(crate::p2p::lockout_scope(ip))
    .bind(since)
    .fetch_one(pool)
    .await?;
    if failures >= IP_LIMIT {
        Err(ApiError::TooManyAttempts)
    } else {
        Ok(())
    }
}

pub(crate) async fn audit(
    pool: &SqlitePool,
    actor: Option<&str>,
    action: &str,
    entity_id: Option<&str>,
    ip: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO audit_log(id, actor_id, action, entity, entity_id, ip, created_at)
         VALUES (?, ?, ?, 'user', ?, ?, ?)",
    )
    .bind(new_id())
    .bind(actor)
    .bind(action)
    .bind(entity_id)
    .bind(ip)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

async fn start_session(
    pool: &SqlitePool,
    user: CurrentUser,
    device: &Device,
    ip: &str,
    needs_2fa_setup: bool,
) -> Result<SessionOut, ApiError> {
    let token = new_token()?;
    let now = now_ms();
    // Новый вход с того же устройства заменяет его прежнюю сессию.
    sqlx::query(
        "UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND device_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&user.id)
    .bind(device.device_id.trim())
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO sessions(id, user_id, token_hash, device_id, device_name, ip, created_at, last_seen_at, expires_at, needs_2fa_setup)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(&user.id)
    .bind(token_hash(&token))
    .bind(device.device_id.trim())
    .bind(device.device_name.trim())
    .bind(ip)
    .bind(now)
    .bind(now)
    .bind(now + SESSION_TTL.num_milliseconds())
    .bind(needs_2fa_setup)
    .execute(pool)
    .await?;
    audit(pool, Some(&user.id), "login.ok", Some(&user.username), ip).await?;
    Ok(SessionOut {
        token,
        user,
        must_setup_2fa: needs_2fa_setup,
    })
}

#[derive(Serialize)]
pub struct InviteOut {
    pub(crate) invite_token: String,
    pub(crate) invite_expires_at: i64,
}

pub(crate) async fn issue_invite(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
) -> Result<InviteOut, ApiError> {
    let token = new_token()?;
    let expires = now_ms() + INVITE_TTL.num_milliseconds();
    sqlx::query("DELETE FROM invites WHERE user_id = ? AND used_at IS NULL")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO invites(id, user_id, token_hash, expires_at) VALUES (?, ?, ?, ?)")
        .bind(new_id())
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(expires)
        .execute(&mut **tx)
        .await?;
    Ok(InviteOut {
        invite_token: token,
        invite_expires_at: expires,
    })
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

// ---------- Первый запуск ----------

#[derive(Deserialize)]
pub struct SetupIn {
    username: String,
    display_name: String,
    password: String,
}

/// Создание владельца. Только с самой машины хоста и только пока пользователей нет —
/// иначе любой в сети мог бы «занять» свежеустановленный сервер.
pub async fn setup(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<SetupIn>,
) -> Result<(StatusCode, Json<CurrentUser>), ApiError> {
    if !addr.ip().is_loopback() {
        return Err(ApiError::Forbidden);
    }
    let username = Username::parse(&req.username)?;
    let display_name = parse_display_name(&req.display_name)?;
    check_password(&req.password, &username)?;
    let hash = hash_password(req.password).await?;

    let id = new_id();
    let inserted = sqlx::query(
        "INSERT INTO users(id, username, display_name, password_hash, role, created_at)
         SELECT ?, ?, ?, ?, 'host', ? WHERE NOT EXISTS (SELECT 1 FROM users)",
    )
    .bind(&id)
    .bind(username.as_str())
    .bind(&display_name)
    .bind(hash)
    .bind(now_ms())
    .execute(&app.pool)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Conflict("владелец уже создан"));
    }
    audit(
        &app.pool,
        Some(&id),
        "setup.owner_created",
        Some(username.as_str()),
        &addr.ip().to_string(),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(CurrentUser {
            id,
            username: username.as_str().to_owned(),
            display_name,
            role: Role::Host,
            session_id: String::new(),
        }),
    ))
}

#[derive(Serialize)]
pub struct SetupStatus {
    needs_owner: bool,
}

/// Нужно ли ещё создать владельца — для мастера первого запуска.
pub async fn setup_status(State(app): State<AppState>) -> Result<Json<SetupStatus>, ApiError> {
    let any: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(&app.pool)
        .await?;
    Ok(Json(SetupStatus { needs_owner: !any }))
}

// ---------- Вход и выход ----------

#[derive(Deserialize)]
pub struct LoginIn {
    username: String,
    password: String,
    /// Код из приложения или резервный — когда сервер его попросил.
    #[serde(default)]
    code: Option<String>,
    #[serde(flatten)]
    device: Device,
}

pub async fn login(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginIn>,
) -> Result<WithCookie<Json<SessionOut>>, ApiError> {
    req.device.check()?;
    let ip = addr.ip().to_string();
    // Ключ блокировки — то, что ввели, даже если такого логина нет. Логины не
    // длиннее 32 байт, так что обрезка до 64 символов ни с кем не совпадёт,
    // а в журнал и уведомление не уйдёт мегабайт текста.
    let key: String = req
        .username
        .trim()
        .to_lowercase()
        .chars()
        .take(64)
        .collect();

    ensure_not_locked(&app.pool, "login.failed", &key, &ip).await?;
    ensure_ip_not_spraying(&app.pool, &ip).await?;

    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, display_name, role, password_hash FROM users WHERE username = ? AND is_active = 1",
    )
    .bind(&key)
    .fetch_optional(&app.pool)
    .await?;
    let stored = row.as_ref().and_then(|r| r.3.clone());

    if !verify_password(req.password, stored).await? {
        audit(&app.pool, None, "login.failed", Some(&key), &ip).await?;
        alert_failed_login(&app, &key, &ip).await?;
        return Err(ApiError::BadCredentials);
    }
    let (id, display_name, role, _) = row.ok_or(ApiError::BadCredentials)?;
    let user = CurrentUser {
        id,
        username: key.clone(),
        display_name,
        role: role
            .parse()
            .map_err(|_| ApiError::Corrupt(format!("роль «{role}»")))?,
        session_id: String::new(),
    };
    let office = crm_core::remote::is_office_addr(&ip);
    let rule = crm_core::totp::login_rule(
        crate::two_factor::policy(&app.pool).await?,
        crate::two_factor::enrolled(&app.pool, &user.id).await?,
        office,
    );
    let mut mfa_ok = false;
    let needs_setup = match rule {
        Login::Allow => false,
        Login::Setup => true,
        Login::Refuse => {
            audit(
                &app.pool,
                Some(&user.id),
                "login.refused_no_2fa",
                Some(&key),
                &ip,
            )
            .await?;
            return Err(ApiError::TwoFactorInOffice);
        }
        Login::NeedCode => {
            let Some(code) = req.code.as_deref().filter(|c| !c.trim().is_empty()) else {
                return Err(ApiError::NeedCode);
            };
            if !crate::two_factor::check_code(&app.pool, &user.id, code).await? {
                // Неверный код — та же блокировка, что и для пароля.
                audit(&app.pool, None, "login.failed", Some(&key), &ip).await?;
                alert_failed_login(&app, &key, &ip).await?;
                return Err(ApiError::BadTwoFactorCode);
            }
            mfa_ok = true;
            false
        }
    };
    let out = start_session(&app.pool, user, &req.device, &ip, needs_setup).await?;
    if mfa_ok {
        sqlx::query("UPDATE sessions SET mfa_ok = 1 WHERE token_hash = ?")
            .bind(token_hash(&out.token))
            .execute(&app.pool)
            .await?;
    }
    Ok(with_cookie(set_cookie(&out.token), Json(out)))
}

#[derive(Deserialize)]
pub struct VerifyIn {
    code: String,
}

/// Подтвердить кодом уже открытую сессию (вошли в офисе, работаем из дома).
pub async fn verify(
    State(app): State<AppState>,
    user: CurrentUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<VerifyIn>,
) -> Result<StatusCode, ApiError> {
    let ip = addr.ip().to_string();
    let key = user.username.clone();
    ensure_not_locked(&app.pool, "login.failed", &key, &ip).await?;
    if !crate::two_factor::check_code(&app.pool, &user.id, &req.code).await? {
        audit(&app.pool, None, "login.failed", Some(&key), &ip).await?;
        alert_failed_login(&app, &key, &ip).await?;
        return Err(ApiError::BadTwoFactorCode);
    }
    sqlx::query("UPDATE sessions SET mfa_ok = 1 WHERE id = ?")
        .bind(&user.session_id)
        .execute(&app.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn logout(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<WithCookie<StatusCode>, ApiError> {
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&user.session_id)
        .execute(&app.pool)
        .await?;
    Ok(with_cookie(clear_cookie(), StatusCode::NO_CONTENT))
}

pub async fn me(user: CurrentUser) -> Json<CurrentUser> {
    Json(user)
}

// ---------- Приглашения ----------

#[derive(Deserialize)]
pub struct AcceptIn {
    token: String,
    password: String,
    #[serde(flatten)]
    device: Device,
}

#[derive(Deserialize)]
pub struct InviteCheckIn {
    code: String,
}

/// Действует ли приглашение: программа хранит его, пока сотрудник не задал пароль,
/// и использованное ведёт на обычный вход, а не на «Задайте пароль».
pub async fn check_invite(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<InviteCheckIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM invites i JOIN users u ON u.id = i.user_id
         WHERE i.token_hash = ? AND i.used_at IS NULL AND i.expires_at > ? AND u.is_active = 1)",
    )
    .bind(token_hash(q.code.trim()))
    .bind(now_ms())
    .fetch_one(&app.pool)
    .await?;
    Ok(Json(serde_json::json!({ "valid": valid })))
}

pub async fn accept_invite(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<AcceptIn>,
) -> Result<WithCookie<Json<SessionOut>>, ApiError> {
    req.device.check()?;
    let row: Option<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT i.id, u.id, u.username, u.display_name, u.role
         FROM invites i JOIN users u ON u.id = i.user_id
         WHERE i.token_hash = ? AND i.used_at IS NULL AND i.expires_at > ? AND u.is_active = 1",
    )
    .bind(token_hash(&req.token))
    .bind(now_ms())
    .fetch_optional(&app.pool)
    .await?;
    let (invite_id, id, username, display_name, role) = row.ok_or(ApiError::BadInvite)?;

    // Приглашение лежит в ключе подключения, который пересылают по почте и в
    // мессенджерах. Если для этого адреса нужен код из приложения, принять
    // приглашение из интернета нельзя: иначе перехвативший ключ привяжет свой
    // телефон. В офисе — можно; по ключу сервера — тоже: устройство уже одобрил владелец.
    let ip = addr.ip().to_string();
    let required = match crate::two_factor::policy(&app.pool).await? {
        crm_core::totp::Policy::All => true,
        crm_core::totp::Policy::Remote => !crm_core::remote::is_office_addr(&ip),
        crm_core::totp::Policy::Off => false,
    };
    let trusted = crm_core::remote::is_office_addr(&ip) || crate::p2p::is_device_addr(&ip);
    if required && !trusted {
        return Err(ApiError::InviteInOffice);
    }

    check_password(&req.password, &Username::parse(&username)?)?;
    let hash = hash_password(req.password).await?;

    let mut tx = crate::db::write_tx(&app.pool).await?;
    let claimed = sqlx::query("UPDATE invites SET used_at = ? WHERE id = ? AND used_at IS NULL")
        .bind(now_ms())
        .bind(&invite_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if claimed != 1 {
        return Err(ApiError::BadInvite);
    }
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let user = CurrentUser {
        id,
        username,
        display_name,
        role: role
            .parse()
            .map_err(|_| ApiError::Corrupt(format!("роль «{role}»")))?,
        session_id: String::new(),
    };
    // Если организация требует код, его настраивают сразу после пароля.
    let needs_setup = required && !crate::two_factor::enrolled(&app.pool, &user.id).await?;
    let out = start_session(&app.pool, user, &req.device, &ip, needs_setup).await?;
    Ok(with_cookie(set_cookie(&out.token), Json(out)))
}

// ---------- Администрирование ----------

#[derive(Deserialize)]
pub struct NewUserIn {
    username: String,
    display_name: String,
    role: Role,
}

#[derive(Serialize)]
pub struct NewUserOut {
    user: CurrentUser,
    #[serde(flatten)]
    invite: InviteOut,
}

pub async fn create_user(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<NewUserIn>,
) -> Result<(StatusCode, Json<NewUserOut>), ApiError> {
    if req.role == Role::Host {
        return Err(ApiError::BadRequest("владелец у сервера один".into()));
    }
    let username = Username::parse(&req.username)?;
    let display_name = parse_display_name(&req.display_name)?;
    let id = new_id();

    let mut tx = crate::db::write_tx(&app.pool).await?;
    sqlx::query(
        "INSERT INTO users(id, username, display_name, role, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(username.as_str())
    .bind(&display_name)
    .bind(req.role.as_str())
    .bind(now_ms())
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::Conflict("такой логин уже есть")
        } else {
            ApiError::Db(e)
        }
    })?;
    let invite = issue_invite(&mut tx, &id).await?;
    tx.commit().await?;
    tracing::info!(by = %owner.username, user = %username.as_str(), "создан пользователь");

    Ok((
        StatusCode::CREATED,
        Json(NewUserOut {
            user: CurrentUser {
                id,
                username: username.as_str().to_owned(),
                display_name,
                role: req.role,
                session_id: String::new(),
            },
            invite,
        }),
    ))
}

/// Сброс доступа сотрудника: пароль стирается, все его сессии завершаются,
/// выдаётся новое приглашение. Пароль владельца так не сбросить — только с машины хоста.
pub async fn reset_access(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<InviteOut>, ApiError> {
    let mut tx = crate::db::write_tx(&app.pool).await?;
    let username: Option<String> = sqlx::query_scalar(
        "UPDATE users SET password_hash = NULL WHERE id = ? AND role <> 'host' RETURNING username",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(username) = username else {
        return Err(ApiError::NotFound);
    };
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let invite = issue_invite(&mut tx, &id).await?;
    tx.commit().await?;
    // Снимает пометку «просит сбросить пароль».
    audit(
        &app.pool,
        Some(&owner.id),
        "access.reset",
        Some(&username),
        "",
    )
    .await?;
    Ok(Json(invite))
}

/// Сотрудник (не владелец, не удалён) — для отключения и удаления.
async fn staff_username(pool: &SqlitePool, id: &str) -> Result<String, ApiError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT username, role FROM users WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    match row {
        None => Err(ApiError::NotFound),
        Some((_, role)) if role == "host" => Err(ApiError::BadRequest(
            "владельца нельзя отключить или удалить".into(),
        )),
        Some((username, _)) => Ok(username),
    }
}

async fn close_sessions(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM invites WHERE user_id = ? AND used_at IS NULL")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Отключить: войти нельзя, все входы закрыты; задачи и данные на месте, можно включить обратно.
pub async fn disable_user(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let username = staff_username(&app.pool, &id).await?;
    let mut tx = crate::db::write_tx(&app.pool).await?;
    sqlx::query("UPDATE users SET is_active = 0 WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    close_sessions(&mut tx, &id).await?;
    tx.commit().await?;
    audit(
        &app.pool,
        Some(&owner.id),
        "user.disabled",
        Some(&username),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn enable_user(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let username = staff_username(&app.pool, &id).await?;
    sqlx::query("UPDATE users SET is_active = 1 WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?;
    audit(
        &app.pool,
        Some(&owner.id),
        "user.enabled",
        Some(&username),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct DeleteUserQuery {
    /// Кому передать открытые задачи и клиентов.
    reassign_to: Option<String>,
}

/// Удалить: из списков и входа — навсегда, логин свободен. Открытые задачи и клиенты
/// переходят к `reassign_to`; завершённые задачи и сообщения хранят имя с «(удалён)».
pub async fn delete_user(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<DeleteUserQuery>,
) -> Result<StatusCode, ApiError> {
    let username = staff_username(&app.pool, &id).await?;
    let (open, managed): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM task_assignees ta JOIN tasks t ON t.id = ta.task_id
                 WHERE ta.user_id = ?1 AND t.status <> 'completed' AND t.deleted_at IS NULL),
                (SELECT count(*) FROM clients WHERE manager_id = ?1 AND deleted_at IS NULL)",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await?;
    let heir = match q.reassign_to.as_deref().filter(|h| !h.is_empty()) {
        Some(h) => {
            let ok: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM users WHERE id = ? AND id <> ? AND is_active = 1 AND deleted_at IS NULL)",
            )
            .bind(h)
            .bind(&id)
            .fetch_one(&app.pool)
            .await?;
            if !ok {
                return Err(ApiError::BadRequest(
                    "передать можно только работающему сотруднику".into(),
                ));
            }
            Some(h.to_owned())
        }
        None if open > 0 || managed > 0 => {
            return Err(ApiError::ConflictWith(format!(
                "у сотрудника открытых задач: {open}, клиентов: {managed} — выберите, кому их передать"
            )));
        }
        None => None,
    };
    let mut tx = crate::db::write_tx(&app.pool).await?;
    if let Some(h) = &heir {
        sqlx::query(
            "INSERT OR IGNORE INTO task_assignees(task_id, user_id)
             SELECT ta.task_id, ? FROM task_assignees ta JOIN tasks t ON t.id = ta.task_id
             WHERE ta.user_id = ? AND t.status <> 'completed' AND t.deleted_at IS NULL",
        )
        .bind(h)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE clients SET manager_id = ? WHERE manager_id = ?")
            .bind(h)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
    }
    // Из открытых задач — убрать; в завершённых имя остаётся для истории.
    sqlx::query(
        "DELETE FROM task_assignees WHERE user_id = ?
         AND task_id IN (SELECT id FROM tasks WHERE status <> 'completed')",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM group_members WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    close_sessions(&mut tx, &id).await?;
    sqlx::query(
        "UPDATE users SET is_active = 0, deleted_at = ?, password_hash = NULL,
                totp_secret = NULL, totp_pending = NULL,
                username = 'deleted:' || id, display_name = display_name || ' (удалён)'
         WHERE id = ?",
    )
    .bind(now_ms())
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM totp_backup WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(
        &app.pool,
        Some(&owner.id),
        "user.deleted",
        Some(&username),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SessionInfo {
    id: String,
    username: String,
    display_name: String,
    device_name: String,
    ip: String,
    created_at: i64,
    last_seen_at: i64,
}

pub async fn list_sessions(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    let rows = sqlx::query_as(
        "SELECT s.id, u.username, u.display_name, s.device_name, s.ip, s.created_at, s.last_seen_at
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.revoked_at IS NULL AND s.expires_at > ?
         ORDER BY s.last_seen_at DESC",
    )
    .bind(now_ms())
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(rows))
}

pub async fn revoke_session(
    State(app): State<AppState>,
    Owner(_): Owner,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(&id)
        .execute(&app.pool)
        .await?
        .rows_affected();
    if n == 1 {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct StaffMember {
    id: String,
    username: String,
    display_name: String,
    role: String,
    /// `active` — работает; `invited` — пароль ещё не задан; `disabled` — отключён.
    state: String,
    created_at: i64,
    /// Сотрудник нажал «Забыли пароль?» за последнюю неделю, а доступ ещё не сброшен.
    reset_requested: bool,
    /// Код из приложения настроен.
    two_factor: bool,
}

/// Список сотрудников для админ-панели владельца.
pub async fn list_users(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<Vec<StaffMember>>, ApiError> {
    let rows = sqlx::query_as(
        "SELECT id, username, display_name, role,
                CASE WHEN is_active = 0 THEN 'disabled'
                     WHEN password_hash IS NULL THEN 'invited'
                     ELSE 'active' END AS state,
                created_at,
                EXISTS (
                    SELECT 1 FROM audit_log a
                    WHERE a.action = 'reset.requested' AND a.entity_id = users.username
                      AND a.created_at > ?
                      AND a.created_at > COALESCE((SELECT max(b.created_at) FROM audit_log b
                                                   WHERE b.action = 'access.reset' AND b.entity_id = users.username), 0)
                ) AS reset_requested,
                totp_secret IS NOT NULL AS two_factor
         FROM users
         WHERE deleted_at IS NULL
         ORDER BY role = 'host' DESC, display_name COLLATE NOCASE",
    )
    .bind(now_ms() - Duration::days(7).num_milliseconds())
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(rows))
}
