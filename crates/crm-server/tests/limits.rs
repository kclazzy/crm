//! Защита от перегрузки: лимиты запросов по IP, перебор логинов, размер запроса.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::json;

const ATTACKER: &str = "192.168.1.66";

#[tokio::test]
async fn flood_from_one_address_is_throttled_others_keep_working() {
    let app = App::new().await;
    app.owner().await;
    let mut throttled = 0;
    for _ in 0..150 {
        if app.call("GET", "/api/health", ATTACKER, None, None).await.0
            == StatusCode::TOO_MANY_REQUESTS
        {
            throttled += 1;
        }
    }
    assert!(throttled > 0, "поток запросов с одного IP ограничивается");
    assert_eq!(
        app.call("GET", "/api/health", OFFICE_PC, None, None)
            .await
            .0,
        StatusCode::OK,
        "другие сотрудники не страдают"
    );
}

#[tokio::test]
async fn login_attempts_are_limited_per_address_even_with_different_logins() {
    let app = App::new().await;
    app.owner().await;
    let mut last = StatusCode::OK;
    for i in 0..15 {
        last = app
            .try_login(&format!("user{i}"), "какой-то-пароль-1", ATTACKER)
            .await
            .0;
    }
    assert_eq!(
        last,
        StatusCode::TOO_MANY_REQUESTS,
        "перебор разных логинов с одного IP останавливается"
    );
    assert_eq!(
        app.try_login("owner", "очень-надёжный-пароль", LOCAL)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn slow_spraying_across_logins_is_blocked_by_failure_history() {
    let app = App::new().await;
    app.owner().await;
    // 20 неудач с разными логинами за последние минуты (как при медленном переборе).
    for i in 0..20 {
        sqlx::query("INSERT INTO audit_log(id, action, entity, entity_id, ip, created_at) VALUES (?, 'login.failed', 'user', ?, ?, ?)")
            .bind(format!("f{i}"))
            .bind(format!("user{i}"))
            .bind(ATTACKER)
            .bind(chrono::Utc::now().timestamp_millis() - 60_000)
            .execute(&app.pool)
            .await
            .unwrap();
    }
    assert_eq!(
        app.try_login("owner", "очень-надёжный-пароль", ATTACKER)
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.try_login("owner", "очень-надёжный-пароль", OFFICE_PC)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn oversized_request_is_rejected() {
    let app = App::new().await;
    let big = "x".repeat(2 * 1024 * 1024);
    let (s, _) = app
        .call(
            "POST",
            "/api/auth/login",
            OFFICE_PC,
            None,
            Some(json!({ "username": big, "password": "p" })),
        )
        .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn owner_sees_who_was_throttled() {
    let app = App::new().await;
    let owner = app.owner().await;
    for i in 0..15 {
        app.try_login(&format!("user{i}"), "какой-то-пароль-1", ATTACKER)
            .await;
    }
    let (s, v) = app
        .call("GET", "/api/admin/security", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let row = v["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["ip"] == ATTACKER)
        .cloned()
        .unwrap();
    assert!(row["count"].as_u64().unwrap() >= 1);
    assert!(v["failed_logins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["ip"] == ATTACKER));
}
