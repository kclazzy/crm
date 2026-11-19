//! Удаление уведомлений и записей журнала безопасности. Удалённое пропадает из
//! списков, но защита на нём держится: блокировка за подбор пароля по-прежнему
//! считает неудачные попытки, а та же тревога не приходит снова сразу же.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::Value;

const HOME: &str = "85.12.34.56";

async fn bell(app: &App, token: &str) -> Value {
    let (s, v) = app
        .call("GET", "/api/notifications", LOCAL, Some(token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn journal(app: &App, owner: &str, q: &str) -> Vec<Value> {
    let (s, v) = app
        .call(
            "GET",
            &format!("/api/admin/journal{q}"),
            LOCAL,
            Some(owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

#[tokio::test]
async fn notifications_can_be_deleted_one_by_one_and_all_at_once() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    app.try_login("ivan", "не тот пароль", HOME).await;
    app.try_login("petr", "не тот пароль", HOME).await;
    let v = bell(&app, &owner).await;
    let items = v["items"].as_array().unwrap().clone();
    assert!(items.len() >= 2, "{v}");
    let unread = v["unread"].as_i64().unwrap();

    let first = items[0]["id"].as_str().unwrap();
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/notifications/{first}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let v = bell(&app, &owner).await;
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["id"] != first));
    assert_eq!(
        v["unread"].as_i64().unwrap(),
        unread - 1,
        "непрочитанные считаются без удалённых"
    );

    // Та же тревога сразу не возвращается: удалённое — как прочитанное.
    app.try_login("ivan", "не тот пароль", HOME).await;
    let again = bell(&app, &owner).await["items"].as_array().unwrap().len();
    assert_eq!(again, items.len() - 1, "удалённая тревога не пришла снова");

    let (s, _) = app
        .call("DELETE", "/api/notifications", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let v = bell(&app, &owner).await;
    assert_eq!(v["items"].as_array().unwrap().len(), 0);
    assert_eq!(v["unread"], 0);
}

#[tokio::test]
async fn nobody_deletes_someone_elses_notification() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, ivan) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    app.try_login("petr", "не тот пароль", HOME).await;
    let id = bell(&app, &owner).await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/notifications/{id}"),
            LOCAL,
            Some(&ivan),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // «Удалить все» сотрудника не трогает чужие.
    app.call("DELETE", "/api/notifications", LOCAL, Some(&ivan), None)
        .await;
    assert_eq!(
        bell(&app, &owner).await["items"].as_array().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn journal_entries_can_be_deleted_but_lockout_still_counts_them() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (ivan_id, ivan) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let limit = crm_server::auth::LOCKOUT_LIMIT;
    for _ in 0..limit {
        app.try_login("ivan", "не тот пароль", HOME).await;
    }
    let all = journal(&app, &owner, "").await;
    let failed: Vec<&Value> = all
        .iter()
        .filter(|e| e["action"] == "login.failed")
        .collect();
    assert!(!failed.is_empty());

    // Одна запись.
    let one = failed[0]["id"].as_str().unwrap();
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/admin/journal/{one}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(journal(&app, &owner, "")
        .await
        .iter()
        .all(|e| e["id"] != one));

    // Все записи по фильтру: только этого сотрудника, только входы.
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/admin/journal?user={ivan_id}&kind=login"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        journal(&app, &owner, &format!("?user={ivan_id}&kind=login"))
            .await
            .is_empty()
    );
    assert!(
        !journal(&app, &owner, "").await.is_empty(),
        "события других остались"
    );

    // Блокировка за подбор держится: удаление из журнала её не снимает.
    let (s, v) = app.try_login("ivan", "пароль-сотрудника-1", HOME).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");

    // Сотрудник журнал не чистит.
    let (s, _) = app
        .call("DELETE", "/api/admin/journal", LOCAL, Some(&ivan), None)
        .await;
    assert!(
        s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED,
        "{s}"
    );
}

/// «Сообщить о проблеме»: сотрудник пишет пару слов — владелец получает это в
/// колокольчик с именем, страницей и версией.
#[tokio::test]
async fn problem_report_reaches_the_owner() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, ivan) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let (s, _) = app
        .call(
            "POST",
            "/api/problem",
            LOCAL,
            Some(&ivan),
            Some(serde_json::json!({"text": "  ", "page": "/tasks"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "пустое не отправляется");
    let long = "не открывается задача ".repeat(200);
    let (s, _) = app
        .call(
            "POST",
            "/api/problem",
            LOCAL,
            Some(&ivan),
            Some(serde_json::json!({"text": long, "page": "/tasks/abc"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let v = bell(&app, &owner).await;
    let n = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "problem_report")
        .expect("в колокольчике владельца")
        .clone();
    assert_eq!(n["payload"]["from"], "Сотрудник");
    assert_eq!(n["payload"]["page"], "/tasks/abc");
    assert!(n["payload"]["text"].as_str().unwrap().chars().count() <= 1000);
    assert!(n["payload"]["version"].is_string());
}
