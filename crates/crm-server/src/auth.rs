//! Вход: первый владелец, приглашения, логин, сессии (ТЗ §3.2).

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};
use crm_core::auth::{check_password, parse_display_name, Role, Username};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::http::{ApiError, AppState};
use crate::secrets::{hash_password, new_token, token_hash, verify_password};

pub const INVITE_TTL: Duration = Duration::hours(72);
pub const SESSION_TTL: Duration = Duration::days(30);
pub const LOCKOUT_WINDOW: Duration = Duration::minutes(15);
pub const LOCKOUT_LIMIT: i64 = 5;
/// Как часто продлевать сессию (чтобы не писать в БД на каждый запрос).
const TOUCH_EVERY: Duration = Duration::minutes(1);

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn new_id() -> String {
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

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &AppState) -> Result<Self, ApiError> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        let now = now_ms();
        let row: Option<(String, String, String, String, String, i64)> = sqlx::query_as(
            "SELECT s.id, u.id, u.username, u.display_name, u.role, s.last_seen_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.revoked_at IS NULL AND s.expires_at > ? AND u.is_active = 1",
        )
        .bind(token_hash(token))
        .bind(now)
        .fetch_optional(&app.pool)
        .await?;
        let (session_id, id, username, display_name, role, last_seen) =
            row.ok_or(ApiError::Unauthorized)?;

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
}

async fn audit(
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
) -> Result<SessionOut, ApiError> {
    let token = new_token()?;
    let now = now_ms();
    sqlx::query(
        "INSERT INTO sessions(id, user_id, token_hash, device_id, device_name, ip, created_at, last_seen_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .execute(pool)
    .await?;
    audit(pool, Some(&user.id), "login.ok", Some(&user.username), ip).await?;
    Ok(SessionOut { token, user })
}

#[derive(Serialize)]
pub struct InviteOut {
    invite_token: String,
    invite_expires_at: i64,
}

async fn issue_invite(
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

// ---------- Вход и выход ----------

#[derive(Deserialize)]
pub struct LoginIn {
    username: String,
    password: String,
    #[serde(flatten)]
    device: Device,
}

pub async fn login(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginIn>,
) -> Result<Json<SessionOut>, ApiError> {
    req.device.check()?;
    let ip = addr.ip().to_string();
    // Ключ блокировки — то, что ввели, даже если такого логина нет.
    let key = req.username.trim().to_lowercase();

    let since = now_ms() - LOCKOUT_WINDOW.num_milliseconds();
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log
         WHERE action = 'login.failed' AND entity_id = ? AND ip = ? AND created_at > ?",
    )
    .bind(&key)
    .bind(&ip)
    .bind(since)
    .fetch_one(&app.pool)
    .await?;
    if failures >= LOCKOUT_LIMIT {
        return Err(ApiError::TooManyAttempts);
    }

    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, display_name, role, password_hash FROM users WHERE username = ? AND is_active = 1",
    )
    .bind(&key)
    .fetch_optional(&app.pool)
    .await?;
    let stored = row.as_ref().and_then(|r| r.3.clone());

    if !verify_password(req.password, stored).await? {
        audit(&app.pool, None, "login.failed", Some(&key), &ip).await?;
        return Err(ApiError::BadCredentials);
    }
    let (id, display_name, role, _) = row.ok_or(ApiError::BadCredentials)?;
    let user = CurrentUser {
        id,
        username: key,
        display_name,
        role: role
            .parse()
            .map_err(|_| ApiError::Corrupt(format!("роль «{role}»")))?,
        session_id: String::new(),
    };
    Ok(Json(
        start_session(&app.pool, user, &req.device, &ip).await?,
    ))
}

pub async fn logout(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, ApiError> {
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&user.session_id)
        .execute(&app.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
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

pub async fn accept_invite(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<AcceptIn>,
) -> Result<Json<SessionOut>, ApiError> {
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

    check_password(&req.password, &Username::parse(&username)?)?;
    let hash = hash_password(req.password).await?;

    let mut tx = app.pool.begin().await?;
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
    Ok(Json(
        start_session(&app.pool, user, &req.device, &addr.ip().to_string()).await?,
    ))
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

    let mut tx = app.pool.begin().await?;
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
    Owner(_): Owner,
    Path(id): Path<String>,
) -> Result<Json<InviteOut>, ApiError> {
    let mut tx = app.pool.begin().await?;
    let found =
        sqlx::query("UPDATE users SET password_hash = NULL WHERE id = ? AND role <> 'host'")
            .bind(&id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if found == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let invite = issue_invite(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(invite))
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
