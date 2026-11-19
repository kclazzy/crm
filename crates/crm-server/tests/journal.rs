//! Журнал безопасности владельца: входы, неудачные попытки, отключения, сбросы —
//! одним списком, новые сверху, с фильтром по сотруднику и виду. Работа с задачами
//! в него не попадает.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::{json, Value};

const HOME: &str = "85.12.34.56";

async fn journal(app: &App, token: &str, query: &str) -> Vec<Value> {
    let (s, v) = app
        .call(
            "GET",
            &format!("/api/admin/journal{query}"),
            LOCAL,
            Some(token),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

fn actions(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .map(|i| i["action"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn owner_sees_security_events_newest_first_with_filters() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (ivan_id, ivan) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    app.try_login("ivan", "не тот пароль", HOME).await;
    app.login("ivan", "пароль-сотрудника-1", OFFICE_PC).await;
    // Работа с задачами — не событие безопасности.
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "person", "name": "Иванов"})),
        )
        .await;
    assert!(s.is_success());
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/admin/users/{ivan_id}/disable"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let all = journal(&app, &owner, "").await;
    let a = actions(&all);
    assert_eq!(a[0], "user.disabled", "новые сверху: {a:?}");
    assert!(
        a.contains(&"login.failed") && a.contains(&"login.ok"),
        "{a:?}"
    );
    assert!(
        a.iter()
            .all(|x| !x.starts_with("client") && !x.starts_with("task")),
        "{a:?}"
    );

    let failed = all.iter().find(|i| i["action"] == "login.failed").unwrap();
    assert_eq!(failed["about"], "ivan");
    assert_eq!(
        failed["about_name"], "Сотрудник",
        "имя сотрудника, если логин его"
    );
    assert_eq!(failed["place"], "internet");
    assert_eq!(failed["title"], "Неверный пароль");
    let disabled = &all[0];
    assert_eq!(disabled["actor_name"], "Владелец");
    assert_eq!(disabled["title"], "Сотрудник отключён");

    // Фильтр по сотруднику: только его события (вход владельца сюда не попадает).
    let his = journal(&app, &owner, &format!("?user={ivan_id}")).await;
    assert!(!his.is_empty());
    assert!(his.iter().all(|i| i["about"] == "ivan"), "{his:?}");

    // Фильтр по виду: только входы.
    let logins = journal(&app, &owner, "?kind=login").await;
    assert!(actions(&logins)
        .iter()
        .all(|x| x.starts_with("login.") || x.starts_with("recover.") || x.starts_with("reset.")));
    assert!(!actions(&logins).contains(&"user.disabled"));

    // Постранично: before — время последней записи.
    let first = journal(&app, &owner, "?limit=2").await;
    assert_eq!(first.len(), 2);
    let before = first[1]["at"].as_i64().unwrap();
    let next = journal(&app, &owner, &format!("?limit=2&before={before}")).await;
    assert!(next.iter().all(|i| i["at"].as_i64().unwrap() < before));

    // Сотруднику журнал не виден.
    let (s, _) = app
        .call("GET", "/api/admin/journal", LOCAL, Some(&ivan), None)
        .await;
    assert!(
        s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED,
        "{s}"
    );
}
