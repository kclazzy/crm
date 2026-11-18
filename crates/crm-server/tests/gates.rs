//! Потолок памяти: большие выборки строятся не больше чем по N одновременно,
//! остальные ждут очереди; лёгкие запросы очередь не замечают.

mod common;

use std::time::Duration;

use common::*;

const HEAVY: [&str; 7] = [
    "/api/tasks",
    "/api/tasks/trash",
    "/api/clients",
    "/api/search?q=тест",
    "/api/messenger",
    "/api/export/clients",
    "/api/export/tasks",
];

#[tokio::test]
async fn big_lists_wait_for_a_free_slot_light_requests_do_not() {
    let app = App::new().await;
    let owner = app.owner().await;
    let slots = &app.state.rt.heavy;
    let all = slots
        .acquire_many(slots.available_permits() as u32)
        .await
        .unwrap();

    for path in HEAVY {
        let waited = tokio::time::timeout(
            Duration::from_millis(300),
            app.call("GET", path, LOCAL, Some(&owner), None),
        )
        .await;
        assert!(waited.is_err(), "{path} должен ждать свободного места");
    }
    for path in ["/api/me", "/api/unread", "/api/people"] {
        let (s, _) = tokio::time::timeout(
            Duration::from_secs(5),
            app.call("GET", path, LOCAL, Some(&owner), None),
        )
        .await
        .unwrap_or_else(|_| panic!("{path} не должен ждать"));
        assert_eq!(s, axum::http::StatusCode::OK, "{path}");
    }

    drop(all);
    for path in HEAVY {
        let (s, _) = tokio::time::timeout(
            Duration::from_secs(5),
            app.call("GET", path, LOCAL, Some(&owner), None),
        )
        .await
        .unwrap_or_else(|_| panic!("{path} после освобождения"));
        assert_eq!(s, axum::http::StatusCode::OK, "{path}");
    }
}

/// Мест немного (память на большой ответ — десятки МБ), но не одно: два
/// сотрудника открывают списки параллельно.
#[tokio::test]
async fn a_few_big_lists_run_at_once() {
    let app = App::new().await;
    let n = app.state.rt.heavy.available_permits();
    assert!((2..=8).contains(&n), "мест: {n}");
}

/// Ключ с PIN шифруется через Argon2 на 64 МБ — он стоит в той же очереди,
/// что и проверки паролей; ключ без PIN её не ждёт.
#[tokio::test]
async fn pin_key_waits_in_the_password_line() {
    let app = App::new().await;
    let owner = app.owner().await;
    let line = crm_server::secrets::hashing_slots();
    let all = line
        .acquire_many(line.available_permits() as u32)
        .await
        .unwrap();
    let key = |pin: Option<&str>| {
        app.call(
            "POST",
            "/api/admin/connection-key",
            LOCAL,
            Some(&owner),
            Some(serde_json::json!({ "pin": pin })),
        )
    };
    let waited = tokio::time::timeout(Duration::from_millis(300), key(Some("482913"))).await;
    assert!(waited.is_err(), "ключ с PIN должен ждать очереди");
    let (s, _) = tokio::time::timeout(Duration::from_secs(5), key(None))
        .await
        .expect("ключ без PIN не ждёт");
    assert_eq!(s, axum::http::StatusCode::OK);
    drop(all);
    let (s, v) = tokio::time::timeout(Duration::from_secs(30), key(Some("482913")))
        .await
        .expect("после освобождения");
    assert_eq!(s, axum::http::StatusCode::OK, "{v}");
    assert_eq!(v["with_pin"], true);
}
