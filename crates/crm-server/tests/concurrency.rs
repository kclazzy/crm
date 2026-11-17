//! Одновременная работа: десятки сотрудников сохраняют в одну и ту же секунду —
//! ни одно сохранение не должно потеряться с ошибкой «база заблокирована».
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_saves_all_succeed() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (emp_id, _) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;

    let made = futures_util::future::join_all((0..60).map(|i| {
        let (app, owner) = (&app, &owner);
        async move {
            app.call(
                "POST",
                "/api/clients",
                LOCAL,
                Some(owner),
                Some(json!({"kind": "company", "name": format!("ООО Параллель {i}")})),
            )
            .await
        }
    }))
    .await;
    let failed: Vec<_> = made
        .iter()
        .filter(|(s, _)| *s != StatusCode::CREATED)
        .collect();
    assert!(
        failed.is_empty(),
        "клиенты: {} ошибок, первая {:?}",
        failed.len(),
        failed.first()
    );
    let client = made[0].1["id"].as_str().unwrap().to_owned();

    // Задачи создают транзакцией: чтение, затем запись — главный источник «заблокировано».
    let made = futures_util::future::join_all((0..60).map(|i| {
        let (app, owner, client, emp_id) = (&app, &owner, &client, &emp_id);
        async move {
            app.call(
                "POST",
                "/api/tasks",
                LOCAL,
                Some(owner),
                Some(json!({
                    "client_id": client, "title": format!("Задача {i}"), "priority": 2, "deadline_at": 4102444800000_i64,
                    "assignee_ids": [emp_id], "stages": ["Позвонить", "Отправить договор"]
                })),
            )
            .await
        }
    }))
    .await;
    let failed: Vec<_> = made
        .iter()
        .filter(|(s, _)| *s != StatusCode::CREATED)
        .collect();
    assert!(
        failed.is_empty(),
        "задачи: {} ошибок, первая {:?}",
        failed.len(),
        failed.first()
    );
    let task = made[0].1["id"].as_str().unwrap().to_owned();

    let sent = futures_util::future::join_all((0..60).map(|i| {
        let (app, owner, task) = (&app, &owner, &task);
        async move {
            app.call(
                "POST",
                &format!("/api/tasks/{task}/messages"),
                LOCAL,
                Some(owner),
                Some(json!({"content": format!("Сообщение {i}")})),
            )
            .await
        }
    }))
    .await;
    let failed: Vec<_> = sent.iter().filter(|(s, _)| !s.is_success()).collect();
    assert!(
        failed.is_empty(),
        "сообщения: {} ошибок, первая {:?}",
        failed.len(),
        failed.first()
    );
}
