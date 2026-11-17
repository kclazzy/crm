//! Кому уходят события задачи (чат, изменения) — ровно тем, кто её видит.
//! Эталон — полная проверка прав `Scope` для каждого сотрудника: быстрая рассылка
//! обязана давать тот же список на группах, подзадачах, корзине и отключённых.
#![allow(clippy::unwrap_used)]

mod common;

use common::App;
use crm_server::access::{audience, Scope};
use sqlx::SqlitePool;

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .unwrap();
}

async fn oracle(pool: &SqlitePool, task: &str) -> Vec<String> {
    let users: Vec<(String, String)> =
        sqlx::query_as("SELECT id, role FROM users WHERE is_active = 1")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut out = Vec::new();
    for (id, role) in users {
        let scope = Scope::load_for(pool, &id, role.parse().unwrap())
            .await
            .unwrap();
        if scope.task(task).can_read() {
            out.push(id);
        }
    }
    out.sort();
    out
}

/// База с группами, руководителями, подзадачами, корзиной и отключёнными сотрудниками.
async fn varied(app: &App) {
    app.owner().await;
    let p = &app.pool;
    let user = |id: &str, role: &str, active: i32| {
        format!("INSERT INTO users(id, username, display_name, role, is_active, created_at) VALUES ('{id}', '{id}', '{id}', '{role}', {active}, 0)")
    };
    for (id, role, active) in [
        ("m1", "manager", 1),
        ("m2", "manager", 1),
        ("m3", "manager", 0),
        ("e1", "employee", 1),
        ("e2", "employee", 1),
        ("e3", "employee", 1),
        ("e4", "employee", 1),
        ("e5", "employee", 0),
    ] {
        exec(p, &user(id, role, active)).await;
    }
    exec(p, "INSERT INTO groups(id, name, created_at) VALUES ('gA', 'Группа А', 0), ('gB', 'Группа Б', 0)").await;
    exec(p, "INSERT INTO group_members(group_id, user_id) VALUES ('gA', 'm1'), ('gB', 'm2'), ('gA', 'm3'), ('gA', 'e1')").await;
    let client = |id: &str, group: &str, deleted: &str| {
        format!("INSERT INTO clients(id, kind, name, group_id, created_by, created_at, deleted_at) VALUES ('{id}', 'company', '{id}', '{group}', 'e1', 0, {deleted})")
    };
    exec(p, &client("cA", "gA", "NULL")).await;
    exec(p, &client("cB", "gB", "NULL")).await;
    exec(p, &client("cD", "gA", "1")).await;
    let task = |id: &str, client: &str, parent: &str, deleted: &str| {
        format!("INSERT INTO tasks(id, client_id, parent_task_id, title, priority, status, created_by, created_at, deadline_at, deleted_at) VALUES ('{id}', '{client}', {parent}, '{id}', 2, 'in_progress', 'e1', 0, 0, {deleted})")
    };
    for (id, client, parent, deleted) in [
        ("t1", "cA", "NULL", "NULL"),
        ("t11", "cA", "'t1'", "NULL"),
        ("t111", "cA", "'t11'", "NULL"),
        ("t2", "cA", "NULL", "NULL"),
        ("t3", "cA", "NULL", "1"),
        ("t31", "cA", "'t3'", "NULL"),
        ("t4", "cB", "NULL", "NULL"),
        ("t41", "cB", "'t4'", "NULL"),
        ("t5", "cD", "NULL", "NULL"),
    ] {
        exec(p, &task(id, client, parent, deleted)).await;
    }
    exec(
        p,
        "INSERT INTO task_assignees(task_id, user_id) VALUES
        ('t1', 'e1'), ('t11', 'e2'), ('t2', 'm2'), ('t3', 'e4'), ('t31', 'e3'),
        ('t4', 'e1'), ('t41', 'e5'), ('t41', 'e2'), ('t5', 'e1'), ('t111', 'm3')",
    )
    .await;
}

const TASKS: [&str; 10] = [
    "t1",
    "t11",
    "t111",
    "t2",
    "t3",
    "t31",
    "t4",
    "t41",
    "t5",
    "нет-такой",
];

#[tokio::test]
async fn event_audience_matches_full_access_check() {
    let app = App::new().await;
    varied(&app).await;
    let p = &app.pool;
    for t in TASKS {
        let mut got = audience(p, t).await.unwrap();
        got.sort();
        assert_eq!(got, oracle(p, t).await, "задача {t}");
    }
}

/// Проверка прав для одной задачи или одного клиента (обработчики чата, карточки
/// задачи, этапов) — та же, что полная по всей базе, но без чтения всей базы.
#[tokio::test]
async fn narrow_access_checks_match_full_scope() {
    let app = App::new().await;
    varied(&app).await;
    let p = &app.pool;
    let users: Vec<(String, String)> = sqlx::query_as("SELECT id, role FROM users")
        .fetch_all(p)
        .await
        .unwrap();
    for (id, role) in users {
        let role = role.parse().unwrap();
        let full = Scope::load_for(p, &id, role).await.unwrap();
        for t in TASKS {
            let one = Scope::for_task(p, &id, role, t).await.unwrap();
            assert_eq!(
                (one.task(t).can_read(), one.task(t).can_manage()),
                (full.task(t).can_read(), full.task(t).can_manage()),
                "{id} → задача {t}"
            );
        }
        for c in ["cA", "cB", "cD", "нет-такого"] {
            let one = Scope::for_client(p, &id, role, c).await.unwrap();
            assert_eq!(one.client(c), full.client(c), "{id} → клиент {c}");
            // Задачи клиента — тоже как в полной проверке.
            for t in TASKS {
                if full.client_of(t) == Some(c) {
                    assert_eq!(
                        (one.task(t).can_read(), one.task(t).can_manage()),
                        (full.task(t).can_read(), full.task(t).can_manage()),
                        "{id} → клиент {c}, задача {t}"
                    );
                }
            }
        }
    }
}

/// Права для списков читают только задачи, которые касаются сотрудника, — и дают
/// ровно то же, что расчёт по всей базе: тот же уровень (включая «только путь»),
/// те же клиенты.
#[tokio::test]
async fn own_scope_matches_scope_over_everything() {
    let app = App::new().await;
    varied(&app).await;
    let p = &app.pool;
    let users: Vec<(String, String)> = sqlx::query_as("SELECT id, role FROM users")
        .fetch_all(p)
        .await
        .unwrap();
    for (id, role) in users {
        let role = role.parse().unwrap();
        let own = Scope::load_for(p, &id, role).await.unwrap();
        let all = Scope::load_everything(p, &id, role).await.unwrap();
        for t in TASKS {
            assert_eq!(own.task(t), all.task(t), "{id} → задача {t}");
            if all.task(t).can_read() {
                assert_eq!(
                    own.client_of(t),
                    all.client_of(t),
                    "{id} → клиент задачи {t}"
                );
            }
        }
        for c in ["cA", "cB", "cD", "нет-такого"] {
            assert_eq!(own.client(c), all.client(c), "{id} → клиент {c}");
        }
    }
}
