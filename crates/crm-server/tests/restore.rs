//! Восстановление из копии и переезд на другой компьютер.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{config_in, App, LOCAL};
use serde_json::json;

async fn names(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM clients ORDER BY name")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn add_client(app: &App, owner: &str, name: &str) {
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(owner),
            Some(json!({"kind": "person", "name": name})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn restore_from_backup_replaces_db_on_restart_and_keeps_previous() {
    let app = App::new().await;
    let owner = app.owner().await;
    let backups = app.dir().join("backups");
    app.call(
        "PUT",
        "/api/admin/backup",
        LOCAL,
        Some(&owner),
        Some(json!({"dirs": [backups]})),
    )
    .await;
    add_client(&app, &owner, "Был до копии").await;
    let (s, b) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let name = b["name"].as_str().unwrap().to_owned();
    // Копии делаются раз в минуту по имени — следующая получит другое имя.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    add_client(&app, &owner, "Добавлен после").await;

    let (s, st) = app
        .call(
            "POST",
            "/api/admin/restore",
            LOCAL,
            Some(&owner),
            Some(json!({"name": name})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{st}");
    assert_eq!(st["name"], name.as_str());
    // До перезапуска — только чтение.
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "person", "name": "X"})),
        )
        .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/restore",
            LOCAL,
            Some(&owner),
            Some(json!({"name": "../../etc/passwd"})),
        )
        .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

    // «Перезапуск»: база открывается заново — копия на месте, прежняя сохранена.
    app.pool.close().await;
    let cfg = config_in(app.dir());
    let pool = crm_server::db::open(&cfg.storage).await.unwrap();
    assert_eq!(names(&pool).await, ["Был до копии"]);
    let kept: Vec<_> = std::fs::read_dir(&cfg.storage.db_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("before_restore_") && n.ends_with(".db"))
        .collect();
    assert_eq!(kept.len(), 1, "прежняя база сохранена");
}

#[tokio::test]
async fn corrupt_backup_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_in(dir.path());
    let bad = dir.path().join("backups");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(
        bad.join("backup_2026_11_13_14_00.db.zst"),
        zstd::encode_all(&b"not a database at all"[..], 3).unwrap(),
    )
    .unwrap();
    let e = crm_server::restore::stage(&cfg.storage, &bad)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("целостности"), "{e}");
    assert!(!cfg
        .storage
        .db_dir
        .join(crm_server::restore::STAGED)
        .exists());
}

#[tokio::test]
async fn move_package_carries_data_files_and_freezes_old_server() {
    let app = App::new().await;
    let owner = app.owner().await;
    let backups = app.dir().join("backups");
    app.call(
        "PUT",
        "/api/admin/backup",
        LOCAL,
        Some(&owner),
        Some(json!({"dirs": [backups]})),
    )
    .await;
    add_client(&app, &owner, "Переезжаю").await;
    let (_, c) = app
        .call("GET", "/api/clients", LOCAL, Some(&owner), None)
        .await;
    let (_, t) = app
        .call(
            "POST",
            "/api/tasks",
            LOCAL,
            Some(&owner),
            Some(json!({
                "client_id": c[0]["id"], "title": "С файлом", "priority": 2,
                "deadline_at": chrono::Utc::now().timestamp_millis() + 86_400_000
            })),
        )
        .await;
    let (_, f) = app.upload(&owner, "dogovor.pdf", b"%PDF data").await;
    app.call(
        "POST",
        &format!("/api/tasks/{}/messages", t["id"].as_str().unwrap()),
        LOCAL,
        Some(&owner),
        Some(json!({"attachments": [{"sha256": f["sha256"], "file_name": "dogovor.pdf"}]})),
    )
    .await;

    let usb = app.dir().join("usb");
    let (s, packed) = app
        .call(
            "POST",
            "/api/admin/move-out",
            LOCAL,
            Some(&owner),
            Some(json!({"dir": usb})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{packed}");
    let pack = std::path::PathBuf::from(packed["file"].as_str().unwrap());
    assert!(pack.extension().unwrap() == "crmpack");
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "person", "name": "Поздно"})),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "старый сервер больше не принимает изменения"
    );

    // Новый компьютер: пустые папки, восстановление из пакета.
    let new_pc = tempfile::tempdir().unwrap();
    let cfg = config_in(new_pc.path());
    let staged = crm_server::restore::stage(&cfg.storage, &pack)
        .await
        .unwrap();
    assert_eq!(staged.attachments_copied, 1);
    let pool = crm_server::db::open(&cfg.storage).await.unwrap();
    assert_eq!(names(&pool).await, ["Переезжаю"]);
    let sha = f["sha256"].as_str().unwrap();
    assert_eq!(
        std::fs::read(cfg.storage.attachments_dir.join(&sha[..2]).join(sha)).unwrap(),
        b"%PDF data"
    );
}

/// Копия сервера «Старый офис» — путь к файлу копии.
async fn backup_of(app: &App, owner: &str) -> std::path::PathBuf {
    let backups = app.dir().join("backups");
    app.call(
        "PUT",
        "/api/admin/backup",
        LOCAL,
        Some(owner),
        Some(json!({"dirs": [backups]})),
    )
    .await;
    let (s, b) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    backups.join(b["name"].as_str().unwrap())
}

#[tokio::test]
async fn import_db_merges_into_existing_server_without_duplicates() {
    // Старый сервер: Иван ведёт задачу клиента «ООО Старое».
    let old = App::new().await;
    let old_owner = old.owner().await;
    let (old_ivan, ivan_tok) = old.member(&old_owner, "ivan", "employee", "10.0.0.2").await;
    let (s, c) = old
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&old_owner),
            Some(json!({"kind": "company", "name": "ООО Старое", "tags": ["VIP"]})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let deadline = chrono::Utc::now().timestamp_millis() + 86_400_000;
    let (s, t) = old
        .call("POST", "/api/tasks", LOCAL, Some(&old_owner), Some(json!({
            "client_id": c["id"], "title": "Перенести архив", "priority": 2, "deadline_at": deadline, "assignees": [old_ivan]
        })))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let task = t["id"].as_str().unwrap().to_owned();
    old.call(
        "POST",
        &format!("/api/tasks/{task}/messages"),
        LOCAL,
        Some(&ivan_tok),
        Some(json!({"content": "Начал перенос"})),
    )
    .await;
    let file = backup_of(&old, &old_owner).await;

    // Новый сервер: свой владелец, свой Иван (тот же логин) и Пётр, клиент «ООО Новое».
    let new = App::new().await;
    let owner = new.owner().await;
    let (ivan, _) = new.member(&owner, "ivan", "employee", "10.0.0.2").await;
    new.member(&owner, "petr", "employee", "10.0.0.4").await;
    add_client(&new, &owner, "ООО Новое").await;

    let body = json!({"source": file, "mode": "merge"});
    let (_, emp) = new.member(&owner, "olga", "employee", "10.0.0.5").await;
    let (s, _) = new
        .call(
            "POST",
            "/api/admin/import-db",
            LOCAL,
            Some(&emp),
            Some(body.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, r) = new
        .call(
            "POST",
            "/api/admin/import-db",
            LOCAL,
            Some(&owner),
            Some(body.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["added"]["clients"], 1, "{r}");
    assert_eq!(r["added"]["tasks"], 1, "{r}");
    assert_eq!(r["merged_users"], 2, "владелец и Иван совпали: {r}");

    // Клиенты обоих серверов; задача у «нашего» Ивана; сообщение на месте; владелец один.
    assert_eq!(names(&new.pool).await, ["ООО Новое", "ООО Старое"]);
    let (_, card) = new
        .call(
            "GET",
            &format!("/api/tasks/{task}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(card["assignees"][0]["id"], ivan.as_str(), "{card}");
    let (_, chat) = new
        .call(
            "GET",
            &format!("/api/tasks/{task}/messages"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert!(chat.to_string().contains("Начал перенос"), "{chat}");
    let hosts: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'host'")
        .fetch_one(&new.pool)
        .await
        .unwrap();
    assert_eq!(hosts, 1);
    let ivans: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE username = 'ivan'")
        .fetch_one(&new.pool)
        .await
        .unwrap();
    assert_eq!(ivans, 1);

    // Повтор ничего не добавляет.
    let (s, r) = new
        .call(
            "POST",
            "/api/admin/import-db",
            LOCAL,
            Some(&owner),
            Some(body),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["added"]["clients"], 0, "{r}");
    assert_eq!(names(&new.pool).await, ["ООО Новое", "ООО Старое"]);
}

#[tokio::test]
async fn import_db_replace_stages_like_restore() {
    let old = App::new().await;
    let old_owner = old.owner().await;
    add_client(&old, &old_owner, "Только на старом").await;
    let file = backup_of(&old, &old_owner).await;
    let new = App::new().await;
    let owner = new.owner().await;
    let (s, r) = new
        .call(
            "POST",
            "/api/admin/import-db",
            LOCAL,
            Some(&owner),
            Some(json!({"source": file, "mode": "replace"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["restart"], true, "{r}");
    assert!(
        new.dir().join("data").join("crm.db.restore").exists() || r["staged"].is_object(),
        "{r}"
    );
}
