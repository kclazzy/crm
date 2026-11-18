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
