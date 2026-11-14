//! Фоновые работы: напоминания, автоархив, поиск по правам.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL};
use serde_json::json;

const HOUR: i64 = 3600 * 1000;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn setup(app: &App) -> (String, String, String, String) {
    let owner = app.owner().await;
    // Без тихих часов: проверяем саму логику напоминаний.
    let (s, _) = app
        .call("PUT", "/api/admin/org", LOCAL, Some(&owner), Some(json!({
            "work_days": [1,2,3,4,5,6,7], "work_start": "00:00", "work_end": "23:59", "timezone": "UTC"
        })))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (emp_id, emp) = app.member(&owner, "ivan", "employee", "10.0.0.2").await;
    let (_, c) = app
        .call("POST", "/api/clients", LOCAL, Some(&owner), Some(json!({"kind": "person", "name": "Иванов", "description": "Особый договор поставки"})))
        .await;
    (owner, emp_id, emp, c["id"].as_str().unwrap().to_owned())
}

#[tokio::test]
async fn reminder_comes_once_and_restarts_after_deadline_change() {
    let app = App::new().await;
    let (owner, emp_id, emp, client) = setup(&app).await;
    let (_, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(&owner),
            Some(json!({
                "client_id": client, "title": "Срочно", "priority": 1,
                "deadline_at": now() + 3 * HOUR, "assignees": [emp_id]
            })),
        )
        .await;
    let t = t["id"].as_str().unwrap().to_owned();

    // Прошли отметки 24 ч и 4 ч — приходит одно напоминание, самое срочное.
    assert_eq!(crm_server::workers::reminders(&app.state).await.unwrap(), 1);
    assert_eq!(
        crm_server::workers::reminders(&app.state).await.unwrap(),
        0,
        "повтора нет"
    );
    let (_, bell) = app
        .call("GET", "/api/notifications", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(bell["unread"], 1);
    assert_eq!(bell["items"][0]["payload"]["reminder"], "h4");
    assert_eq!(bell["quiet"], false);
    let (_, bell) = app
        .call("GET", "/api/notifications", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(bell["unread"], 0, "владелец не исполнитель");

    // Перенос дедлайна — новый цикл.
    let (s, _) = app
        .call(
            "PATCH",
            &format!("/api/tasks/{t}"),
            LOCAL,
            Some(&owner),
            Some(json!({"deadline_at": now() + 2 * HOUR})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(crm_server::workers::reminders(&app.state).await.unwrap(), 1);

    let (s, _) = app
        .call("POST", "/api/notifications/read", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, bell) = app
        .call("GET", "/api/notifications", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(bell["unread"], 0);
}

#[tokio::test]
async fn completed_tasks_go_to_archive_and_come_back() {
    let app = App::new().await;
    let (owner, _, _, client) = setup(&app).await;
    let (_, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(&owner),
            Some(json!({
                "client_id": client, "title": "Старая", "priority": 3, "deadline_at": now() + HOUR
            })),
        )
        .await;
    let t = t["id"].as_str().unwrap().to_owned();
    app.call(
        "POST",
        &format!("/api/tasks/{t}/status"),
        LOCAL,
        Some(&owner),
        Some(json!({"status": "completed"})),
    )
    .await;
    assert_eq!(
        crm_server::workers::archive(&app.state).await.unwrap(),
        0,
        "ещё свежая"
    );
    sqlx::query("UPDATE tasks SET completed_at = completed_at - 8 * 24 * 3600 * 1000")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(crm_server::workers::archive(&app.state).await.unwrap(), 1);

    let (_, list) = app
        .call("GET", "/api/tasks", LOCAL, Some(&owner), None)
        .await;
    assert!(list.as_array().unwrap().is_empty());
    let (_, list) = app
        .call("GET", "/api/tasks?archived=true", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/status"),
            LOCAL,
            Some(&owner),
            Some(json!({"status": "in_progress"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "из архива сначала вернуть");
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/unarchive"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list) = app
        .call("GET", "/api/tasks", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn search_finds_only_what_user_may_see() {
    let app = App::new().await;
    let (owner, emp_id, emp, client) = setup(&app).await;
    let (_, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(&owner),
            Some(json!({
                "client_id": client, "title": "Согласовать договор", "priority": 2,
                "deadline_at": now() + 48 * HOUR, "stages": ["Проверить договор юристом"]
            })),
        )
        .await;
    let t = t["id"].as_str().unwrap().to_owned();
    app.call(
        "POST",
        &format!("/api/tasks/{t}/messages"),
        LOCAL,
        Some(&owner),
        Some(json!({"content": "Договор отправлен клиенту"})),
    )
    .await;

    let (_, hits) = app
        .call("GET", "/api/search?q=ДОГОВОР", LOCAL, Some(&owner), None)
        .await;
    let kinds: Vec<&str> = hits
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap())
        .collect();
    for k in ["client", "task", "stage", "message"] {
        assert!(kinds.contains(&k), "{k} в {hits}");
    }
    let (_, hits) = app
        .call("GET", "/api/search?q=договор", LOCAL, Some(&emp), None)
        .await;
    assert!(
        hits.as_array().unwrap().is_empty(),
        "сотрудник не назначен: {hits}"
    );

    app.call(
        "PATCH",
        &format!("/api/tasks/{t}"),
        LOCAL,
        Some(&owner),
        Some(json!({"assignees": [emp_id]})),
    )
    .await;
    let (_, hits) = app
        .call("GET", "/api/search?q=договор", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(hits.as_array().unwrap().len(), 4, "{hits}");
    let (_, hits) = app
        .call("GET", "/api/search?q=до", LOCAL, Some(&owner), None)
        .await;
    assert!(hits.as_array().unwrap().is_empty(), "меньше 3 символов");
}
