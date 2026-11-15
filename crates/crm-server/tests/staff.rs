//! Сотрудник: отключить учётную запись (обратимо) или удалить (история остаётся).
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::{json, Value};

fn state_of<'a>(staff: &'a Value, username: &str) -> Option<&'a Value> {
    staff
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == username)
}

#[tokio::test]
async fn disabled_employee_cannot_work_until_enabled() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (id, emp) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let url = format!("/api/admin/users/{id}");

    let (s, _) = app
        .call("POST", &format!("{url}/disable"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "отключает только владелец");
    let (s, _) = app
        .call("POST", &format!("{url}/disable"), LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Сессии закрыты, вход не пускает, в выборе исполнителя его нет.
    let (s, _) = app
        .call("GET", "/api/me", OFFICE_PC, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = app
        .try_login("ivan", "пароль-сотрудника-1", OFFICE_PC)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (_, people) = app
        .call("GET", "/api/people", LOCAL, Some(&owner), None)
        .await;
    assert!(!people.to_string().contains(&id), "{people}");
    let (_, staff) = app
        .call("GET", "/api/admin/users", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(state_of(&staff, "ivan").unwrap()["state"], "disabled");

    // Включили — входит со своим паролем.
    let (s, _) = app
        .call("POST", &format!("{url}/enable"), LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = app
        .try_login("ivan", "пароль-сотрудника-1", OFFICE_PC)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Владельца не отключить.
    let (_, me) = app.call("GET", "/api/me", LOCAL, Some(&owner), None).await;
    let me = me["id"].as_str().unwrap();
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/admin/users/{me}/disable"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert!(
        s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND,
        "{s}"
    );
}

#[tokio::test]
async fn deleted_employee_hands_over_work_and_keeps_history() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (ivan, emp) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let (petr, _) = app.member(&owner, "petr", "employee", OFFICE_PC).await;
    let (s, c) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "company", "name": "ООО Ромашка"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let c = c["id"].as_str().unwrap().to_owned();
    let deadline = chrono::Utc::now().timestamp_millis() + 86_400_000;
    let (s, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(&owner),
            Some(json!({"client_id": c, "title": "Отчёт", "priority": 2, "deadline_at": deadline, "assignees": [ivan]})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let t = t["id"].as_str().unwrap().to_owned();
    app.call(
        "POST",
        &format!("/api/tasks/{t}/messages"),
        LOCAL,
        Some(&emp),
        Some(json!({"content": "Начал готовить отчёт"})),
    )
    .await;

    // Есть открытые задачи — без «кому передать» не удалить.
    let url = format!("/api/admin/users/{ivan}");
    let (s, v) = app.call("DELETE", &url, LOCAL, Some(&owner), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("1"),
        "сколько задач: {v}"
    );
    let (s, v) = app
        .call(
            "DELETE",
            &format!("{url}?reassign_to={petr}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");

    // Задача перешла к Петру, сообщение Ивана осталось с пометкой.
    let (_, card) = app
        .call("GET", &format!("/api/tasks/{t}"), LOCAL, Some(&owner), None)
        .await;
    let names = card["assignees"].to_string();
    assert!(names.contains(&petr) && !names.contains(&ivan), "{card}");
    let (_, chat) = app
        .call(
            "GET",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert!(chat.to_string().contains("(удалён)"), "{chat}");

    // В списках его нет, войти нельзя, логин свободен.
    let (_, staff) = app
        .call("GET", "/api/admin/users", LOCAL, Some(&owner), None)
        .await;
    assert!(state_of(&staff, "ivan").is_none(), "{staff}");
    let (s, _) = app
        .try_login("ivan", "пароль-сотрудника-1", OFFICE_PC)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    app.member(&owner, "ivan", "employee", OFFICE_PC).await;
}
