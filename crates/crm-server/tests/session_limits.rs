//! Сколько живёт вход: владелец задаёт сроки для офиса и для интернета.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use crm_core::totp::{base32_decode, code_at, STEP_SECS};
use serde_json::json;

const HOME: &str = "85.12.34.56";
const HOUR: i64 = 3600 * 1000;

fn step() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap() / STEP_SECS
}

async fn set(app: &App, owner: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    app.call(
        "PUT",
        "/api/admin/session-limits",
        LOCAL,
        Some(owner),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn owner_sets_session_limits_within_sensible_bounds() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call(
            "GET",
            "/api/admin/session-limits",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["office_days"], 30, "по умолчанию — как раньше");
    assert_eq!(v["remote_idle_hours"], 720);
    assert_eq!(v["remote_code_days"], 0, "код повторно не спрашиваем");
    let (s, _) = set(
        &app,
        &owner,
        json!({"office_days": 0, "remote_idle_hours": 12, "remote_code_days": 7}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = set(
        &app,
        &owner,
        json!({"office_days": 30, "remote_idle_hours": 5000, "remote_code_days": 7}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = set(
        &app,
        &owner,
        json!({"office_days": 14, "remote_idle_hours": 12, "remote_code_days": 7}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = app
        .call(
            "GET",
            "/api/admin/session-limits",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(v["office_days"], 14);
    let (emp_id, emp) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let _ = emp_id;
    let (s, _) = set(
        &app,
        &emp,
        json!({"office_days": 1, "remote_idle_hours": 1, "remote_code_days": 1}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn idle_internet_session_ends_office_one_lives_on() {
    let app = App::new().await;
    let owner = app.owner().await;
    // Код из приложения выключен — проверяем только срок.
    app.call(
        "PUT",
        "/api/admin/2fa",
        LOCAL,
        Some(&owner),
        Some(json!({"policy": "off"})),
    )
    .await;
    set(
        &app,
        &owner,
        json!({"office_days": 30, "remote_idle_hours": 12, "remote_code_days": 0}),
    )
    .await;
    let (_, emp) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let home = app.login("ivan", "пароль-сотрудника-1", HOME).await;

    // 13 часов без действий.
    sqlx::query("UPDATE sessions SET last_seen_at = last_seen_at - ?")
        .bind(13 * HOUR)
        .execute(&app.pool)
        .await
        .unwrap();
    let (s, _) = app.call("GET", "/api/me", HOME, Some(&home), None).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "из интернета — выход после 12 ч простоя"
    );
    let (s, _) = app
        .call("GET", "/api/me", OFFICE_PC, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::OK, "в офисе тот же простой не мешает");
    let (s, _) = app.call("GET", "/api/me", HOME, Some(&home), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "закрытый вход не оживает");
}

#[tokio::test]
async fn internet_session_asks_the_code_again_after_n_days() {
    let app = App::new().await;
    let owner = app.owner().await;
    set(
        &app,
        &owner,
        json!({"office_days": 30, "remote_idle_hours": 720, "remote_code_days": 7}),
    )
    .await;
    // Владелец включает код и входит из дома с кодом.
    let (_, v) = app
        .call("POST", "/api/me/2fa/start", LOCAL, Some(&owner), None)
        .await;
    let secret = base32_decode(v["secret"].as_str().unwrap()).unwrap();
    let c = |n: u64| format!("{:06}", code_at(&secret, step() + n));
    let (s, v) = app
        .call(
            "POST",
            "/api/me/2fa/confirm",
            LOCAL,
            Some(&owner),
            Some(json!({"code": c(0)})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let spare = v["backup_codes"][0].as_str().unwrap().to_owned();
    let (s, v) = app
        .call(
            "POST",
            "/api/auth/login",
            HOME,
            None,
            Some(json!({
                "username": "owner", "password": "очень-надёжный-пароль",
                "device_id": "home", "device_name": "Дом", "code": c(1)
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let home = v["token"].as_str().unwrap().to_owned();
    let (s, _) = app.call("GET", "/api/me", HOME, Some(&home), None).await;
    assert_eq!(s, StatusCode::OK);

    // Прошло 8 дней с ввода кода.
    sqlx::query("UPDATE sessions SET mfa_at = mfa_at - ?")
        .bind(8 * 24 * HOUR)
        .execute(&app.pool)
        .await
        .unwrap();
    let (s, v) = app.call("GET", "/api/tasks", HOME, Some(&home), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(
        v["need_code"], true,
        "просим код заново, вход не теряется: {v}"
    );
    let (s, v) = app
        .call(
            "POST",
            "/api/auth/verify",
            HOME,
            Some(&home),
            Some(json!({"code": spare})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, _) = app.call("GET", "/api/tasks", HOME, Some(&home), None).await;
    assert_eq!(s, StatusCode::OK, "после кода работаем дальше");
}
