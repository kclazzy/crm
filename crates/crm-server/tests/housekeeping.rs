//! Уборка базы: служебные записи, корзина по решению владельца, обслуживание.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL};
use serde_json::json;

const DAY: i64 = 24 * 3600 * 1000;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn count(app: &App, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn old_service_records_are_removed_fresh_ones_stay() {
    let app = App::new().await;
    let owner = app.owner().await;
    let uid: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let n = now();
    for (id, created, hidden) in [
        ("n-old", n - 100 * DAY, None),
        ("n-hidden", n - 50 * DAY, Some(n - 40 * DAY)),
        ("n-fresh", n - DAY, None),
        ("n-just-hidden", n - 5 * DAY, Some(n - DAY)),
    ] {
        sqlx::query("INSERT INTO notifications(id, user_id, kind, payload, created_at, hidden_at) VALUES (?, ?, 'assigned', '{}', ?, ?)")
            .bind(id).bind(&uid).bind(created).bind(hidden).execute(&app.pool).await.unwrap();
    }
    for (id, created, hidden) in [
        ("a-old", n - 400 * DAY, None),
        ("a-hidden", n - 50 * DAY, Some(n - 40 * DAY)),
        ("a-fresh", n - 300 * DAY, None),
    ] {
        sqlx::query("INSERT INTO audit_log(id, action, created_at, hidden_at) VALUES (?, 'login.failed', ?, ?)")
            .bind(id).bind(created).bind(hidden).execute(&app.pool).await.unwrap();
    }
    for (id, expires, revoked) in [
        ("s-expired", n - 40 * DAY, None),
        ("s-revoked", n + DAY, Some(n - 40 * DAY)),
        ("s-recent", n - 5 * DAY, None),
    ] {
        sqlx::query("INSERT INTO sessions(id, user_id, token_hash, device_id, device_name, ip, created_at, last_seen_at, expires_at, revoked_at) VALUES (?, ?, ?, 'd', 'd', '1.1.1.1', 0, 0, ?, ?)")
            .bind(id).bind(&uid).bind(format!("h-{id}")).bind(expires).bind(revoked).execute(&app.pool).await.unwrap();
    }

    let report = crm_server::housekeeping::tidy(&app.state).await.unwrap();
    assert!(report.removed >= 6, "{report:?}");

    let ids = |t: &str| {
        format!("SELECT count(*) FROM {t} WHERE id IN ('n-old','n-hidden','a-old','a-hidden','s-expired','s-revoked')")
    };
    assert_eq!(count(&app, &ids("notifications")).await, 0);
    assert_eq!(count(&app, &ids("audit_log")).await, 0);
    assert_eq!(count(&app, &ids("sessions")).await, 0);
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM notifications WHERE id IN ('n-fresh','n-just-hidden')"
        )
        .await,
        2
    );
    assert_eq!(
        count(&app, "SELECT count(*) FROM audit_log WHERE id = 'a-fresh'").await,
        1
    );
    assert_eq!(
        count(&app, "SELECT count(*) FROM sessions WHERE id = 's-recent'").await,
        1
    );
    // Текущий вход владельца не задет.
    let (s, _) = app.call("GET", "/api/me", LOCAL, Some(&owner), None).await;
    assert_eq!(s, StatusCode::OK);
}

async fn trashed_task(app: &App, owner: &str, title: &str, days_ago: i64) -> String {
    let (_, c) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(owner),
            Some(json!({"kind": "person", "name": format!("Клиент {title}")})),
        )
        .await;
    let (s, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(owner),
            Some(json!({"client_id": c["id"], "title": title, "priority": 2, "deadline_at": now() + DAY})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_owned();
    sqlx::query("UPDATE tasks SET deleted_at = ? WHERE id = ?")
        .bind(now() - days_ago * DAY)
        .bind(&id)
        .execute(&app.pool)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn trash_is_cleaned_only_when_owner_turns_it_on() {
    let app = App::new().await;
    let owner = app.owner().await;
    let old = trashed_task(&app, &owner, "Старая", 100).await;
    let recent = trashed_task(&app, &owner, "Недавняя", 10).await;

    let (s, p) = app
        .call("GET", "/api/admin/trash-policy", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["days"], serde_json::Value::Null, "по умолчанию выключено");
    assert_eq!(p["old_items"], 1, "предложение: есть что убрать");
    assert_eq!(p["suggest"], true);

    crm_server::housekeeping::tidy(&app.state).await.unwrap();
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM tasks WHERE id = '{old}'")
        )
        .await,
        1,
        "без согласия не удаляем"
    );

    let (s, _) = app
        .call(
            "POST",
            "/api/admin/trash-policy/dismiss",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, p) = app
        .call("GET", "/api/admin/trash-policy", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(
        p["suggest"], false,
        "«Не сейчас» — больше не подсказываем какое-то время"
    );

    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/trash-policy",
            LOCAL,
            Some(&owner),
            Some(json!({"days": 7})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "только из списка сроков: {v}");
    let (s, _) = app
        .call(
            "PUT",
            "/api/admin/trash-policy",
            LOCAL,
            Some(&owner),
            Some(json!({"days": 90})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    crm_server::housekeeping::tidy(&app.state).await.unwrap();
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM tasks WHERE id = '{old}'")
        )
        .await,
        0,
        "старше 90 дней — удалена"
    );
    assert_eq!(
        count(
            &app,
            &format!("SELECT count(*) FROM tasks WHERE id = '{recent}'")
        )
        .await,
        1
    );

    let (s, _) = app
        .call(
            "PUT",
            "/api/admin/trash-policy",
            LOCAL,
            Some(&owner),
            Some(json!({"days": null})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "можно выключить");
}

#[tokio::test]
async fn maintenance_shrinks_database_after_large_deletions() {
    let app = App::new().await;
    let _owner = app.owner().await;
    let uid: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let big = "x".repeat(4000);
    for i in 0..2000 {
        sqlx::query("INSERT INTO notifications(id, user_id, kind, payload, created_at) VALUES (?, ?, 'assigned', ?, 0)")
            .bind(format!("big-{i}")).bind(&uid).bind(&big).execute(&app.pool).await.unwrap();
    }
    sqlx::query("DELETE FROM notifications WHERE id LIKE 'big-%'")
        .execute(&app.pool)
        .await
        .unwrap();
    let before = count(&app, "SELECT page_count FROM pragma_page_count()").await;
    let done = crm_server::housekeeping::maintain(&app.state)
        .await
        .unwrap();
    assert!(done.vacuumed, "много пустого места — сжали: {done:?}");
    let after = count(&app, "SELECT page_count FROM pragma_page_count()").await;
    assert!(after * 2 < before, "{before} → {after}");
    let again = crm_server::housekeeping::maintain(&app.state)
        .await
        .unwrap();
    assert!(!again.vacuumed, "сжимать больше нечего");
}
