//! Метка «Новое»: задача, которую мне назначили, а я ещё не открывал.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::{json, Value};

async fn list(app: &App, token: &str) -> Vec<Value> {
    let (s, v) = app
        .call("GET", "/api/tasks", LOCAL, Some(token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

fn is_new(tasks: &[Value], id: &str) -> bool {
    tasks.iter().find(|t| t["id"] == id).unwrap()["is_new"] == true
}

#[tokio::test]
async fn assigned_task_is_new_until_opened() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (ivan_id, ivan) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let (_, c) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "person", "name": "Иванов"})),
        )
        .await;
    let (s, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(&owner),
            Some(json!({"client_id": c["id"], "title": "Позвонить", "priority": 2, "deadline_at": 4102444800000_i64, "assignees": [ivan_id]})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap();

    assert!(is_new(&list(&app, &ivan).await, id), "назначили — новая");
    assert!(
        !is_new(&list(&app, &owner).await, id),
        "свою задачу постановщик новой не видит"
    );

    let (s, _) = app
        .call("GET", &format!("/api/tasks/{id}"), LOCAL, Some(&ivan), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !is_new(&list(&app, &ivan).await, id),
        "открыл — больше не новая"
    );
}
