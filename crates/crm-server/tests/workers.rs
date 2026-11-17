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
async fn archive_is_manual_restorable_and_purged_only_by_owner() {
    let app = App::new().await;
    let (owner, emp_id, emp, client) = setup(&app).await;
    let (_, mgr) = app.member(&owner, "manager1", "manager", "10.0.0.6").await;
    // Постановщик — руководитель, исполнитель — сотрудник.
    let (s, t) = app
        .call("POST", "/api/tasks", LOCAL, Some(&mgr), Some(json!({
            "client_id": client, "title": "Отчёт", "priority": 3, "deadline_at": now() + HOUR, "assignees": [emp_id]
        })))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let t = t["id"].as_str().unwrap().to_owned();
    let arch = |tok: String| {
        let (app, t) = (&app, t.clone());
        async move {
            app.call(
                "POST",
                &format!("/api/tasks/{t}/archive"),
                LOCAL,
                Some(&tok),
                None,
            )
            .await
        }
    };
    let (s, v) = arch(emp.clone()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "не завершена: {v}");
    app.call(
        "POST",
        &format!("/api/tasks/{t}/status"),
        LOCAL,
        Some(&emp),
        Some(json!({"status": "completed"})),
    )
    .await;
    let (s, v) = arch(mgr.clone()).await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "постановщик не убирает в архив: {v}"
    );
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("исполнитель или владелец"));
    let (s, _) = arch(emp.clone()).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (_, list) = app
        .call("GET", "/api/tasks", LOCAL, Some(&owner), None)
        .await;
    assert!(
        list.as_array().unwrap().is_empty(),
        "в обычных списках архива нет"
    );
    let (_, list) = app
        .call("GET", "/api/tasks?archived=true", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, card) = app
        .call("GET", &format!("/api/tasks/{t}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(card["archived_by"], "Сотрудник");

    let (s, _) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/unarchive"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "восстановить может исполнитель");
    // Восстановленную из архива завершённую задачу можно вернуть в работу.
    let (s, v) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/status"),
            LOCAL,
            Some(&emp),
            Some(json!({"status": "in_progress"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/status"),
            LOCAL,
            Some(&emp),
            Some(json!({"status": "completed"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    arch(owner.clone()).await;

    // «Из архива в работу» — одним действием: вышла из архива и сразу «Выполняется».
    let (s, v) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/unarchive?reopen=1"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (_, card) = app
        .call("GET", &format!("/api/tasks/{t}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(card["status"], "in_progress");
    assert!(card["archived_at"].is_null());
    let (_, chat) = app
        .call(
            "GET",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert!(
        chat.to_string()
            .contains("вернул(а) задачу из архива в работу"),
        "{chat}"
    );
    app.call(
        "POST",
        &format!("/api/tasks/{t}/status"),
        LOCAL,
        Some(&emp),
        Some(json!({"status": "completed"})),
    )
    .await;
    arch(owner.clone()).await;
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/tasks/{t}/purge"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "удаляет из архива только владелец"
    );
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/tasks/{t}/purge"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app
        .call("GET", &format!("/api/tasks/{t}"), LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
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
    for h in hits.as_array().unwrap() {
        if h["kind"] != "client" {
            assert_eq!(
                h["task_id"],
                t.as_str(),
                "этап и сообщение ведут в задачу: {h}"
            );
        }
    }
    // Архивная задача (и её этапы, чат) — только при поиске по архиву.
    sqlx::query(
        "UPDATE tasks SET status = 'completed', completed_at = 1, archived_at = 1 WHERE id = ?",
    )
    .bind(&t)
    .execute(&app.pool)
    .await
    .unwrap();
    let (_, hits) = app
        .call("GET", "/api/search?q=договор", LOCAL, Some(&owner), None)
        .await;
    let kinds: Vec<&str> = hits
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["client"], "без архива: {hits}");
    let (_, hits) = app
        .call(
            "GET",
            "/api/search?q=договор&archive=true",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(hits.as_array().unwrap().len(), 4, "с архивом: {hits}");
    sqlx::query(
        "UPDATE tasks SET status = 'waiting', completed_at = NULL, archived_at = NULL WHERE id = ?",
    )
    .bind(&t)
    .execute(&app.pool)
    .await
    .unwrap();
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

#[tokio::test]
async fn archive_all_completed_takes_only_what_you_may_archive() {
    let app = App::new().await;
    let (owner, emp_id, emp, client) = setup(&app).await;
    let mut ids = Vec::new();
    for title in ["Первая", "Вторая", "Открытая"] {
        let (_, t) = app
            .call("POST", "/api/tasks", LOCAL, Some(&owner), Some(json!({
                "client_id": client, "title": title, "priority": 2, "deadline_at": now() + HOUR, "assignees": [emp_id]
            })))
            .await;
        ids.push(t["id"].as_str().unwrap().to_owned());
    }
    for t in &ids[..2] {
        app.call(
            "POST",
            &format!("/api/tasks/{t}/status"),
            LOCAL,
            Some(&emp),
            Some(json!({"status": "completed"})),
        )
        .await;
    }
    let (s, v) = app
        .call(
            "POST",
            "/api/tasks/archive-completed",
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], 2, "{v}");
    let (_, list) = app
        .call("GET", "/api/tasks", LOCAL, Some(&owner), None)
        .await;
    let left: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(left, ["Открытая"]);
    let (_, v) = app
        .call(
            "POST",
            "/api/tasks/archive-completed",
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(v["archived"], 0, "повтор ничего не меняет");
}
