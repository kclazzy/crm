//! Хранилище и резервное копирование глазами владельца (ТЗ §2.4, §5).
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::json;

#[tokio::test]
async fn owner_chooses_backup_folder_and_backup_restores_to_working_db() {
    let app = App::new().await;
    let owner = app.owner().await;
    let dir = app.dir().join("usb-backups");
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup",
            LOCAL,
            Some(&owner),
            Some(json!({ "dirs": [dir] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let saved = std::fs::read_to_string(app.dir().join("config.toml")).unwrap();
    assert!(
        saved.contains("usb-backups"),
        "выбор сохраняется в config.toml: {saved}"
    );

    let (s, report) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let name = report["name"].as_str().unwrap();
    assert!(
        name.starts_with("backup_") && name.ends_with(".db.zst"),
        "{name}"
    );

    let packed = std::fs::read(dir.join(name)).unwrap();
    let raw = zstd::decode_all(packed.as_slice()).unwrap();
    let restored = app.dir().join("restored.db");
    std::fs::write(&restored, raw).unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", restored.display()))
        .await
        .unwrap();
    let owners: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'host'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owners, 1, "в копии — рабочая база");
    assert!(dir.join("tls").join("ca.crt").exists() || !app.dir().join("data/tls/ca.crt").exists());

    let (_, list) = app
        .call("GET", "/api/admin/backups", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list["last"]["ok"], true);
    assert_eq!(list["files"][0]["name"], name);
}

#[tokio::test]
async fn rotation_removes_old_copies_and_keeps_foreign_files() {
    let app = App::new().await;
    let owner = app.owner().await;
    let dir = app.dir().join("b");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("backup_2020_01_01_00_00.db.zst"), b"old").unwrap();
    std::fs::write(dir.join("мои-документы.txt"), b"keep").unwrap();
    app.call(
        "PUT",
        "/api/admin/backup",
        LOCAL,
        Some(&owner),
        Some(json!({ "dirs": [dir] })),
    )
    .await;
    let (s, _) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !dir.join("backup_2020_01_01_00_00.db.zst").exists(),
        "копия старше 30 дней удалена"
    );
    assert!(
        dir.join("мои-документы.txt").exists(),
        "чужие файлы не трогаем"
    );
}

#[tokio::test]
async fn bad_backup_folder_is_rejected() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup",
            LOCAL,
            Some(&owner),
            Some(json!({ "dirs": ["relative/path"] })),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn owner_sees_storage_and_server_addresses_by_name() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call("GET", "/api/admin/storage", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["data_dir"].as_str().unwrap().ends_with("data"));
    let vol = &v["volumes"][0];
    assert!(
        vol["total_bytes"].as_u64().unwrap() > 0 && vol["free_bytes"].as_u64().is_some(),
        "{v}"
    );
    assert!(vol["level"].is_string());

    let (s, v) = app
        .call("GET", "/api/admin/server", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let host = v["hostname"].as_str().unwrap().to_lowercase();
    let urls: Vec<String> = v["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    assert!(
        urls.iter().any(|u| u.contains(&host)),
        "адрес по имени компьютера: {urls:?}"
    );
}

#[tokio::test]
async fn employee_cannot_touch_storage_or_backups() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    let (_, v) = app.accept(&invite, "секретный-пароль-1", OFFICE_PC).await;
    let ivan = v["token"].as_str().unwrap().to_owned();
    for (m, p) in [
        ("GET", "/api/admin/storage"),
        ("GET", "/api/admin/backups"),
        ("POST", "/api/admin/backup/run"),
        ("GET", "/api/admin/server"),
    ] {
        assert_eq!(
            app.call(m, p, OFFICE_PC, Some(&ivan), None).await.0,
            StatusCode::FORBIDDEN,
            "{p}"
        );
    }
}

#[tokio::test]
async fn moving_data_copies_db_updates_config_and_freezes_writes_until_restart() {
    let app = App::new().await;
    let owner = app.owner().await;
    let target = app.dir().join("D-drive").join("CRM");
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/storage/move",
            LOCAL,
            Some(&owner),
            Some(json!({ "data_dir": target })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["restart_required"], true);
    assert!(target.join("crm.db").exists());
    let cfg = crm_server::config::Config::load(&app.dir().join("config.toml")).unwrap();
    assert_eq!(cfg.storage.db_dir, target);

    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", target.join("crm.db").display()))
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    let (s, v) = app
        .call(
            "POST",
            "/api/admin/users",
            LOCAL,
            Some(&owner),
            Some(json!({"username":"late","display_name":"Поздний","role":"employee"})),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "до перезапуска записи запрещены: {v}"
    );
    assert_eq!(
        app.call("GET", "/api/me", LOCAL, Some(&owner), None)
            .await
            .0,
        StatusCode::OK,
        "чтение работает"
    );
}

#[tokio::test]
async fn owner_sets_backup_schedule_and_retention() {
    let app = App::new().await;
    let owner = app.owner().await;
    let dir = app.dir().join("b");
    let policy = json!({ "enabled": true, "every_hours": 6, "keep_all_hours": 24, "keep_days": 7 });
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup",
            LOCAL,
            Some(&owner),
            Some(json!({ "dirs": [dir], "policy": policy })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, info) = app
        .call("GET", "/api/admin/storage", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(info["policy"]["every_hours"], 6);
    assert_eq!(
        info["expected_copies"],
        4 + 6,
        "4 копии за сутки + 6 ежедневных"
    );
    let cfg = crm_server::config::Config::load(&app.dir().join("config.toml")).unwrap();
    assert_eq!(cfg.backup.every_hours(), 6, "сохранено в config.toml");

    // Копия 10-дневной давности при хранении 7 дней удаляется.
    std::fs::write(
        dir.join(crm_core::backup::file_name(
            chrono::Utc::now() - chrono::Duration::days(10),
        )),
        b"x",
    )
    .unwrap();
    let kept = crm_core::backup::file_name(chrono::Utc::now() - chrono::Duration::days(5));
    std::fs::write(dir.join(&kept), b"x").unwrap();
    app.call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".zst"))
        .collect();
    assert_eq!(left.len(), 2, "свежая + 5-дневная: {left:?}");
    assert!(left.contains(&kept));
}

#[tokio::test]
async fn impossible_schedule_is_explained() {
    let app = App::new().await;
    let owner = app.owner().await;
    let policy = json!({ "enabled": true, "every_hours": 5, "keep_all_hours": 24, "keep_days": 7 });
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup",
            LOCAL,
            Some(&owner),
            Some(json!({ "dirs": [app.dir().join("b")], "policy": policy })),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("часа"), "{v}");
}

#[tokio::test]
async fn folder_picker_lists_and_creates_folders_on_the_server() {
    let app = App::new().await;
    let owner = app.owner().await;
    let base = tempfile::tempdir().unwrap();
    std::fs::create_dir(base.path().join("Копии")).unwrap();
    std::fs::write(base.path().join("файл.txt"), "x").unwrap();
    let enc = |p: &std::path::Path| {
        p.display()
            .to_string()
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };

    // Без пути — корни (диски).
    let (s, v) = app
        .call("GET", "/api/admin/fs", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v["entries"].as_array().unwrap().is_empty(), "{v}");

    // Папка — только вложенные папки, без файлов.
    let url = format!("/api/admin/fs?path={}", enc(base.path()));
    let (s, v) = app.call("GET", &url, LOCAL, Some(&owner), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let names: Vec<&str> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Копии"], "{v}");
    assert!(v["parent"].is_string());

    // Для импорта — ещё и файлы копий CRM (с пометкой file), чужие файлы — нет.
    std::fs::write(base.path().join("crm_move_1.crmpack"), "x").unwrap();
    std::fs::write(base.path().join("crm_2026.db.zst"), "x").unwrap();
    let (s, v) = app
        .call(
            "GET",
            &format!("{url}&files=backups"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let got: Vec<(&str, bool)> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap(),
                e["file"].as_bool().unwrap_or(false),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("Копии", false),
            ("crm_2026.db.zst", true),
            ("crm_move_1.crmpack", true)
        ],
        "{v}"
    );

    // Создать папку.
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/fs/mkdir",
            LOCAL,
            Some(&owner),
            Some(json!({"parent": base.path(), "name": "CRM-копии"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(base.path().join("CRM-копии").is_dir());
    for bad in ["", "..", "a/b", "a\\b"] {
        let (s, _) = app
            .call(
                "POST",
                "/api/admin/fs/mkdir",
                LOCAL,
                Some(&owner),
                Some(json!({"parent": base.path(), "name": bad})),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "имя «{bad}»");
    }

    let (_, emp) = app.member(&owner, "olga", "employee", "10.0.0.3").await;
    let (s, _) = app
        .call("GET", "/api/admin/fs", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
