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
