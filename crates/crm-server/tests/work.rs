//! Клиенты, задачи, этапы, чат и права доступа — как их видят пользователи.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL};
use serde_json::{json, Value};

const DAY: i64 = 24 * 3600 * 1000;

fn soon(days: i64) -> i64 {
    chrono::Utc::now().timestamp_millis() + days * DAY
}

async fn client(app: &App, token: &str, name: &str) -> String {
    let (s, v) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(token),
            Some(json!({
                "kind": "company", "name": name, "phone": "+7 900 000-00-00",
                "description": "Поставщик оборудования, договор №12", "tags": ["VIP", "vip", "опт"]
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_owned()
}

async fn task(app: &App, token: &str, body: Value) -> (StatusCode, Value) {
    app.call("POST", "/api/tasks", LOCAL, Some(token), Some(body))
        .await
}

async fn new_task(app: &App, token: &str, body: Value) -> String {
    let (s, v) = task(app, token, body).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn client_card_has_description_tags_and_task_counts() {
    let app = App::new().await;
    let owner = app.owner().await;
    let c = client(&app, &owner, "ООО Ромашка").await;
    new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "Просроченная", "priority": 1, "deadline_at": soon(-1)}),
    )
    .await;
    let (s, v) = app
        .call(
            "GET",
            &format!("/api/clients/{c}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["description"], "Поставщик оборудования, договор №12");
    assert_eq!(
        v["tags"].as_array().unwrap().len(),
        2,
        "дубль тега без учёта регистра"
    );
    assert_eq!(v["counts"]["open"], 1);
    assert_eq!(v["counts"]["overdue"], 1);
    assert_eq!(v["tasks"][0]["deadline_state"], "overdue");
    assert_eq!(v["access"], "manage");

    let (_, list) = app
        .call("GET", "/api/clients?q=договор", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1, "поиск по описанию");
    let (_, list) = app
        .call("GET", "/api/clients?tag=опт", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn employee_sees_only_assigned_work() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (emp_id, emp) = app.member(&owner, "ivan", "employee", "10.0.0.2").await;
    let c = client(&app, &owner, "ИП Петров").await;
    let other = client(&app, &owner, "Чужой клиент").await;

    // Не назначен — клиента не видно вовсе.
    let (s, _) = app
        .call("GET", &format!("/api/clients/{c}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let t = new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "Подготовить КП", "priority": 2,
               "deadline_at": soon(3), "assignees": [emp_id], "stages": ["Цены", "Отправить"]}),
    )
    .await;

    let (_, list) = app
        .call("GET", "/api/clients", LOCAL, Some(&emp), None)
        .await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["ИП Петров"]);
    assert_eq!(list[0]["access"], "view");
    let (s, _) = app
        .call(
            "GET",
            &format!("/api/clients/{other}"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (_, card) = app
        .call("GET", &format!("/api/tasks/{t}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(card["access"], "participant");
    let stage = card["stages"][0]["id"].as_str().unwrap().to_owned();

    // Исполнитель работает: этап, статус.
    let (s, _) = app
        .call(
            "PATCH",
            &format!("/api/stages/{stage}"),
            LOCAL,
            Some(&emp),
            Some(json!({"done": true})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
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
    assert_eq!(v["open_stages"], 1, "предупреждение о неотмеченном этапе");

    // …но не управляет.
    let (s, _) = app
        .call(
            "PATCH",
            &format!("/api/tasks/{t}"),
            LOCAL,
            Some(&emp),
            Some(json!({"deadline_at": soon(9)})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&emp),
            Some(json!({"kind": "person", "name": "X"})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = task(
        &app,
        &emp,
        json!({"client_id": c, "title": "Своя", "priority": 3, "deadline_at": soon(1)}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (_, mine) = app
        .call("GET", "/api/tasks?view=mine", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["progress"], json!({"done": 1, "total": 2}));
}

#[tokio::test]
async fn subtask_rules_and_path_only_parents() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (emp_id, emp) = app.member(&owner, "maria", "employee", "10.0.0.3").await;
    let c = client(&app, &owner, "АО Вектор").await;
    let root = new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "Проект", "description": "секретные детали", "priority": 1, "deadline_at": soon(10)}),
    )
    .await;

    let (s, v) = task(
        &app,
        &owner,
        json!({"parent_id": root, "title": "Поздняя", "priority": 1, "deadline_at": soon(11)}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("дедлайн"));

    let sub = new_task(
        &app,
        &owner,
        json!({"parent_id": root, "title": "Монтаж", "priority": 2, "deadline_at": soon(5), "assignees": [emp_id]}),
    )
    .await;

    let (s, v) = app
        .call(
            "POST",
            &format!("/api/tasks/{root}/status"),
            LOCAL,
            Some(&owner),
            Some(json!({"status": "completed"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("подзадач"));

    let (s, v) = app
        .call(
            "PATCH",
            &format!("/api/tasks/{root}"),
            LOCAL,
            Some(&owner),
            Some(json!({"deadline_at": soon(4)})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // Сотрудник подзадачи видит родителя только как название в пути.
    let (s, _) = app
        .call(
            "GET",
            &format!("/api/tasks/{root}"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = app
        .call(
            "GET",
            &format!("/api/tasks/{root}/messages"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, card) = app
        .call("GET", &format!("/api/tasks/{sub}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(
        card["path"],
        json!([{"id": root, "title": "Проект", "readable": false}])
    );
    let (_, client_card) = app
        .call("GET", &format!("/api/clients/{c}"), LOCAL, Some(&emp), None)
        .await;
    let parent = client_card["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == root.as_str())
        .unwrap();
    assert_eq!(parent["access"], "path_only");
    assert!(parent.get("status").is_none(), "у пути нет подробностей");
    assert!(!client_card.to_string().contains("секретные"));

    // Завершили подзадачу — родителя можно закрыть; переоткрыть ребёнка закрытого родителя нельзя.
    for (t, tok) in [(&sub, &emp), (&root, &owner)] {
        let (s, v) = app
            .call(
                "POST",
                &format!("/api/tasks/{t}/status"),
                LOCAL,
                Some(tok),
                Some(json!({"status": "completed"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (s, v) = app
        .call(
            "POST",
            &format!("/api/tasks/{sub}/status"),
            LOCAL,
            Some(&emp),
            Some(json!({"status": "in_progress"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "родитель завершён: {v}");
    assert!(v["error"].as_str().unwrap().contains("родительскую"), "{v}");

    // Вернуть в работу: сначала родителя, потом подзадачу.
    for (t, tok) in [(&root, &owner), (&sub, &emp)] {
        let (s, v) = app
            .call(
                "POST",
                &format!("/api/tasks/{t}/status"),
                LOCAL,
                Some(tok),
                Some(json!({"status": "in_progress"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (_, card) = app
        .call("GET", &format!("/api/tasks/{sub}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(card["status"], "in_progress");
    assert!(card["completed_at"].is_null(), "отметка завершения снята");
    let (_, chat) = app
        .call(
            "GET",
            &format!("/api/tasks/{sub}/messages"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    assert!(
        chat.to_string().contains("начал(а) выполнение задачи"),
        "{chat}"
    );
}

#[tokio::test]
async fn fixed_statuses_for_creator_and_assignee_only() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (emp_id, emp) = app.member(&owner, "ivan", "employee", "10.0.0.2").await;
    let (_, mgr) = app.member(&owner, "manager1", "manager", "10.0.0.3").await;
    let c = client(&app, &owner, "Клиент").await;
    let t = new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "Запрос", "priority": 2, "deadline_at": soon(3), "assignees": [emp_id]}),
    )
    .await;
    let (_, card) = app
        .call("GET", &format!("/api/tasks/{t}"), LOCAL, Some(&emp), None)
        .await;
    assert_eq!(
        card["status"], "waiting",
        "после создания — «Ждёт выполнения»"
    );
    assert_eq!(card["can_change_status"], true);

    let status = |tok: String, to: &'static str| {
        let (app, t) = (&app, t.clone());
        async move {
            app.call(
                "POST",
                &format!("/api/tasks/{t}/status"),
                LOCAL,
                Some(&tok),
                Some(json!({"status": to})),
            )
            .await
        }
    };
    // Руководитель группы видит задачу, но не постановщик и не исполнитель.
    let (s, v) = status(mgr.clone(), "in_progress").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("постановщик, исполнитель"));
    let (s, _) = status(emp.clone(), "waiting").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "в «Ждёт» вручную не вернуть");
    for (who, to) in [
        (emp.clone(), "in_progress"),
        (owner.clone(), "postponed"),
        (emp.clone(), "in_progress"),
        (emp.clone(), "completed"),
    ] {
        let (s, v) = status(who, to).await;
        assert_eq!(s, StatusCode::OK, "{to}: {v}");
    }
    let (_, chat) = app
        .call(
            "GET",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&emp),
            None,
        )
        .await;
    let text = chat.to_string();
    assert!(
        text.contains("отложил(а) задачу") && text.contains("Статус: «Завершена»"),
        "{text}"
    );
}

#[tokio::test]
async fn assignee_changes_while_task_is_in_progress() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (emp_id, emp) = app.member(&owner, "ivan", "employee", "10.0.0.2").await;
    let (petr_id, petr) = app.member(&owner, "petr", "employee", "10.0.0.4").await;
    let (_, outsider) = app.member(&owner, "olga", "employee", "10.0.0.5").await;
    let c = client(&app, &owner, "Клиент").await;
    let t = new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "Отчёт", "priority": 2, "deadline_at": soon(3), "assignees": [emp_id]}),
    )
    .await;
    let url = format!("/api/tasks/{t}");
    let (s, _) = app
        .call(
            "POST",
            &format!("{url}/status"),
            LOCAL,
            Some(&emp),
            Some(json!({"status": "in_progress"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, card) = app.call("GET", &url, LOCAL, Some(&emp), None).await;
    assert_eq!(
        card["can_reassign"], true,
        "исполнитель может передать задачу"
    );

    let reassign = |tok: String, ids: Value| {
        let (app, url) = (&app, url.clone());
        async move {
            app.call(
                "PUT",
                &format!("{url}/assignees"),
                LOCAL,
                Some(&tok),
                Some(json!({"assignees": ids})),
            )
            .await
        }
    };
    let (s, _) = reassign(outsider.clone(), json!([petr_id])).await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN,
        "посторонний не меняет: {s}"
    );
    let (s, v) = reassign(emp.clone(), json!([])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "без исполнителя нельзя: {v}");

    // Исполнитель передаёт задачу в работе — статус и чат сохраняются.
    let (s, v) = reassign(emp.clone(), json!([petr_id])).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (_, card) = app.call("GET", &url, LOCAL, Some(&petr), None).await;
    assert_eq!(card["status"], "in_progress");
    assert_eq!(card["assignees"][0]["id"], petr_id.as_str());
    assert_eq!(
        card["can_change_status"], true,
        "новый исполнитель ведёт задачу"
    );
    // Новому исполнителю и постановщику — уведомления.
    let (_, notes) = app
        .call("GET", "/api/notifications", LOCAL, Some(&petr), None)
        .await;
    assert!(
        notes.to_string().contains("\"assigned\""),
        "новому исполнителю: {notes}"
    );
    let (_, notes) = app
        .call("GET", "/api/notifications", LOCAL, Some(&owner), None)
        .await;
    assert!(
        notes.to_string().contains("\"reassigned\""),
        "постановщику: {notes}"
    );
    // В карточке — сотрудники группы клиента: их предлагать первыми.
    let (_, card) = app.call("GET", &url, LOCAL, Some(&petr), None).await;
    assert!(card["team"].is_array(), "{card}");
    let (_, chat) = app
        .call("GET", &format!("{url}/messages"), LOCAL, Some(&petr), None)
        .await;
    let text = chat.to_string();
    assert!(
        text.contains("Сотрудник сменил(а) исполнителя: Сотрудник → Сотрудник"),
        "{text}"
    );

    // Прежний исполнитель больше не участник; постановщик может сменить обратно.
    let (s, _) = app.call("GET", &url, LOCAL, Some(&emp), None).await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN,
        "{s}"
    );
    let (s, _) = reassign(owner.clone(), json!([emp_id, petr_id])).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // В архиве не меняется.
    for to in ["completed"] {
        app.call(
            "POST",
            &format!("{url}/status"),
            LOCAL,
            Some(&petr),
            Some(json!({"status": to})),
        )
        .await;
    }
    app.call("POST", &format!("{url}/archive"), LOCAL, Some(&petr), None)
        .await;
    let (s, _) = reassign(owner.clone(), json!([petr_id])).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn unread_is_per_user() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (a_id, a) = app.member(&owner, "anna", "employee", "10.0.0.4").await;
    let (b_id, b) = app.member(&owner, "boris", "employee", "10.0.0.5").await;
    let c = client(&app, &owner, "Клиент").await;
    let t = new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "Общая", "priority": 2, "deadline_at": soon(2), "assignees": [a_id, b_id]}),
    )
    .await;
    let unread = |tok: String| {
        let app = &app;
        async move {
            let (_, v) = app
                .call("GET", "/api/unread", LOCAL, Some(&tok), None)
                .await;
            v["total"].as_i64().unwrap()
        }
    };
    // Запись «создал задачу» от владельца — непрочитанная у исполнителей, но не у него.
    assert_eq!(unread(owner.clone()).await, 0);
    assert_eq!(unread(a.clone()).await, 1);

    let (s, _) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&a),
            Some(json!({"content": "Привет!"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(unread(a.clone()).await, 0, "своё сообщение прочитано");
    assert_eq!(unread(b.clone()).await, 2);
    assert_eq!(unread(owner.clone()).await, 1);

    let (s, _) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/read"),
            LOCAL,
            Some(&b),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(unread(b.clone()).await, 0);
    assert_eq!(
        unread(owner.clone()).await,
        1,
        "у владельца осталось непрочитанным"
    );

    let (_, threads) = app
        .call(
            "GET",
            "/api/messenger?unread_only=true",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(threads[0]["last_message"]["excerpt"], "Привет!");
    let (_, chat) = app
        .call(
            "GET",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&b),
            None,
        )
        .await;
    let msgs = chat["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["kind"], "system");
    assert_eq!(msgs[1]["can_edit"], false, "чужое не редактируется");
    let mid = msgs[1]["id"].as_str().unwrap();
    let (s, _) = app
        .call(
            "PATCH",
            &format!("/api/messages/{mid}"),
            LOCAL,
            Some(&b),
            Some(json!({"content": "взлом"})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app
        .call(
            "PATCH",
            &format!("/api/messages/{mid}"),
            LOCAL,
            Some(&a),
            Some(json!({"content": "Привет всем!"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app
        .call("POST", "/api/unread/read-all", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(unread(owner).await, 0);
}

#[tokio::test]
async fn managers_work_within_their_groups() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (m_id, mgr) = app.member(&owner, "manager1", "manager", "10.0.0.6").await;
    let (s, g) = app
        .call(
            "POST",
            "/api/admin/groups",
            LOCAL,
            Some(&owner),
            Some(json!({"name": "Опт", "members": []})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let gid = g["id"].as_str().unwrap();
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/groups",
            LOCAL,
            Some(&owner),
            Some(json!({"name": "Опт"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Клиент чужой группы менеджеру не виден.
    let (s, v) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "company", "name": "Оптовик", "group_id": gid})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let opt = v["id"].as_str().unwrap().to_owned();
    let (s, _) = app
        .call(
            "GET",
            &format!("/api/clients/{opt}"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&mgr),
            Some(json!({"kind": "company", "name": "Мой", "group_id": gid})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // В основной группе — свой клиент, удалить своё можно, чужое нельзя.
    let mine = client(&app, &mgr, "Мой клиент").await;
    let owners = client(&app, &owner, "Клиент владельца").await;
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/clients/{owners}"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/clients/{mine}"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, trash) = app
        .call("GET", "/api/clients?trash=true", LOCAL, Some(&mgr), None)
        .await;
    assert_eq!(trash.as_array().unwrap().len(), 1);
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/clients/{mine}/purge"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "окончательно удаляет только владелец"
    );
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/clients/{mine}/restore"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Владелец переводит менеджера в группу «Опт» — клиент становится виден.
    let (s, _) = app
        .call(
            "PUT",
            &format!("/api/admin/groups/{gid}"),
            LOCAL,
            Some(&owner),
            Some(json!({"name": "Опт", "members": [m_id]})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app
        .call(
            "GET",
            &format!("/api/clients/{opt}"),
            LOCAL,
            Some(&mgr),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/admin/groups/{gid}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "в группе есть клиенты");
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/groups",
            LOCAL,
            Some(&mgr),
            Some(json!({"name": "Своя"})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn attachments_follow_task_access() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (a_id, a) = app.member(&owner, "anna", "employee", "10.0.0.7").await;
    let (_, outsider) = app.member(&owner, "oleg", "employee", "10.0.0.8").await;
    let c = client(&app, &owner, "Клиент").await;
    let t = new_task(
        &app,
        &owner,
        json!({"client_id": c, "title": "С файлом", "priority": 3, "deadline_at": soon(2), "assignees": [a_id]}),
    )
    .await;
    let (s, f) = app
        .upload(&owner, "%D1%81%D1%87%D1%91%D1%82.pdf", b"%PDF-1.4 test")
        .await;
    assert_eq!(s, StatusCode::CREATED, "{f}");
    assert_eq!(f["file_name"], "счёт.pdf");
    let sha = f["sha256"].as_str().unwrap().to_owned();
    let (s, v) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&owner),
            Some(json!({"content": "", "attachments": [{"sha256": sha, "file_name": "счёт.pdf"}]})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let (s, _, body) = app
        .raw(
            "GET",
            &format!("/api/attachments/{sha}"),
            &[("authorization", &format!("Bearer {a}"))],
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, "%PDF-1.4 test");
    let (s, _, _) = app
        .raw(
            "GET",
            &format!("/api/attachments/{sha}"),
            &[("authorization", &format!("Bearer {outsider}"))],
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _) = app
        .call(
            "POST",
            &format!("/api/tasks/{t}/messages"),
            LOCAL,
            Some(&owner),
            Some(json!({"attachments": [{"sha256": "0".repeat(64), "file_name": "x"}]})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "незагруженный файл");
}

#[tokio::test]
async fn organisation_hours_are_validated_and_checked() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/org",
            LOCAL,
            Some(&owner),
            Some(json!({"timezone": "Mars/Base"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("Mars"));
    let (s, _) = app
        .call(
            "PUT",
            "/api/admin/org",
            LOCAL,
            Some(&owner),
            Some(json!({"timezone": "Europe/Moscow", "warn_hours": 48})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = app.call("GET", "/api/org", LOCAL, Some(&owner), None).await;
    assert_eq!(v["warn_hours"], 48);
    // Суббота 14.11.2026 12:00 МСК.
    let saturday = 1_794_646_800_000_i64;
    let (_, v) = app
        .call(
            "GET",
            &format!("/api/org/check?at={saturday}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(v["working"], false);
    assert!(v["reason"].as_str().unwrap().contains("суббота"), "{v}");
}
