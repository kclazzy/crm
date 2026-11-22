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
    // Ещё две свежие: меньше трёх копий уборка не оставляет.
    for h in [1, 2] {
        let t = chrono::Utc::now() - chrono::Duration::hours(h);
        std::fs::write(dir.join(crm_core::backup::file_name(t)), b"x").unwrap();
    }
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
    let hour_ago = crm_core::backup::file_name(chrono::Utc::now() - chrono::Duration::hours(1));
    std::fs::write(dir.join(&hour_ago), b"x").unwrap();
    app.call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".zst"))
        .collect();
    assert_eq!(
        left.len(),
        3,
        "свежая, часовой давности и 5-дневная: {left:?}"
    );
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

#[tokio::test]
async fn unavailable_folder_does_not_stop_copies_to_the_others() {
    let app = App::new().await;
    let owner = app.owner().await;
    let usb = app.dir().join("usb");
    let disk = app.dir().join("disk");
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup",
            LOCAL,
            Some(&owner),
            Some(json!({ "dirs": [usb, disk] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Флешку вынули: на месте папки — файл, писать некуда.
    std::fs::remove_dir_all(&usb).ok();
    std::fs::write(&usb, b"not a folder").unwrap();

    let (s, report) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "копия сохранилась хоть куда-то: {report}"
    );
    let name = report["name"].as_str().unwrap();
    assert!(disk.join(name).exists(), "вторая папка получила копию");

    let (_, list) = app
        .call("GET", "/api/admin/backups", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list["last"]["ok"], false, "владелец видит проблему: {list}");
    assert_eq!(list["last"]["dirs"][0]["ok"], false);
    assert_eq!(list["last"]["dirs"][1]["ok"], true);
    assert!(list["last"]["dirs"][0]["error"]
        .as_str()
        .unwrap()
        .contains("usb"));
}

#[tokio::test]
async fn copy_carries_remote_access_key_and_restore_brings_it_back() {
    let app = App::new().await;
    let owner = app.owner().await;
    let data = app.dir().join("data");
    std::fs::write(data.join("p2p.key"), [7u8; 32]).unwrap();
    let dir = app.dir().join("backups");
    let (s, _) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(std::fs::read(dir.join("p2p.key")).unwrap(), [7u8; 32]);
    assert!(!dir.join("backup.key").exists());

    std::fs::remove_file(data.join("p2p.key")).unwrap();
    let cfg = app.state.rt.config();
    crm_server::restore::stage(&cfg.storage, &dir, None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(data.join("p2p.key")).unwrap(),
        [7u8; 32],
        "ключи сотрудников продолжат работать"
    );
}

#[tokio::test]
async fn encrypted_copies_need_password_on_another_computer() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup/password",
            LOCAL,
            Some(&owner),
            Some(json!({ "password": "короткий" })),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "слишком короткий пароль: {v}");
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/backup/password",
            LOCAL,
            Some(&owner),
            Some(json!({ "password": "пароль копий 2026" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, report) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    let dir = app.dir().join("backups");
    let file = dir.join(report["name"].as_str().unwrap());
    assert!(crm_server::backup_file::is_encrypted(&file).unwrap());
    let (_, list) = app
        .call("GET", "/api/admin/backups", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list["encrypted"], true);

    let cfg = app.state.rt.config();
    // Этот сервер открывает свои копии сам.
    crm_server::restore::stage(&cfg.storage, &dir, None)
        .await
        .unwrap();
    // Новый компьютер: ключа нет — нужен пароль.
    std::fs::remove_file(cfg.storage.db_dir.join("backup.key")).unwrap();
    let e = crm_server::restore::stage(&cfg.storage, &dir, None)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("пароль"), "{e}");
    assert!(
        crm_server::restore::stage(&cfg.storage, &dir, Some("не тот"))
            .await
            .is_err()
    );
    crm_server::restore::stage(&cfg.storage, &dir, Some("пароль копий 2026"))
        .await
        .unwrap();

    let (s, _) = app
        .call(
            "POST",
            "/api/admin/restore",
            LOCAL,
            Some(&owner),
            Some(json!({})),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "без пароля восстановление объясняет, чего не хватает"
    );
}

#[tokio::test]
async fn deleted_attachments_leave_backup_folder_with_the_last_copy_that_had_them() {
    let app = App::new().await;
    let owner = app.owner().await;
    let dir = app.dir().join("backups");
    let (_, f) = app.upload(&owner, "scan.pdf", b"%PDF scan").await;
    let sha = f["sha256"].as_str().unwrap().to_owned();
    // Вложение, на которое не ссылается ни одна копия базы (давно удалённое).
    let stale = dir.join("attachments").join("ff").join("ff".repeat(32));
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, b"old").unwrap();

    let (s, report) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{report}");
    let name = report["name"].as_str().unwrap();
    assert!(
        dir.join(format!("{name}.files")).exists(),
        "список вложений копии"
    );
    assert!(dir.join("attachments").join(&sha[..2]).join(&sha).exists());
    assert!(
        !stale.exists(),
        "ни одна копия на него не ссылается — убран"
    );
}

#[tokio::test]
async fn trial_restore_checks_latest_copy_without_touching_working_db() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/backup/verify",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "копий ещё нет: {v}");
    let (_, report) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/backup/verify",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["users"].as_i64().unwrap() >= 1);
    let data = app.dir().join("data");
    assert!(!data.join("verify").exists(), "временный файл убран");
    assert!(
        !data.join("crm.db.restore").exists(),
        "восстановление не готовится"
    );

    // Копия испортилась уже на флешке.
    let file = app
        .dir()
        .join("backups")
        .join(report["name"].as_str().unwrap());
    std::fs::write(&file, b"broken").unwrap();
    let (_, v) = app
        .call(
            "POST",
            "/api/admin/backup/verify",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(v["ok"], false, "{v}");
    let (_, list) = app
        .call("GET", "/api/admin/backups", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list["verified"]["ok"], false);
    let (_, notes) = app
        .call("GET", "/api/notifications", LOCAL, Some(&owner), None)
        .await;
    assert!(
        notes["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "backup_problem"),
        "владелец узнаёт: {notes}"
    );
    // Рабочая база цела.
    let (s, _) = app.call("GET", "/api/me", LOCAL, Some(&owner), None).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn scheduled_copy_is_skipped_when_nothing_changed() {
    use crm_server::backup::{scheduled, Scheduled};
    let app = App::new().await;
    let owner = app.owner().await;
    let dir = app.dir().join("backups");
    let copies = || {
        std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".db.zst"))
            .count()
    };

    assert!(
        matches!(scheduled(&app.state).await, Scheduled::Saved(_)),
        "первая копия"
    );
    assert_eq!(copies(), 1);
    // Служебные записи (вход, уведомления, журнал) — не повод для новой копии.
    let (_, _) = app.call("GET", "/api/me", LOCAL, Some(&owner), None).await;
    sqlx::query("INSERT INTO audit_log(id, action, created_at) VALUES ('x', 'login.failed', 0)")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(
        matches!(scheduled(&app.state).await, Scheduled::Unchanged),
        "ночью и в выходные копии не пишутся"
    );
    assert_eq!(copies(), 1);
    let (_, list) = app
        .call("GET", "/api/admin/backups", LOCAL, Some(&owner), None)
        .await;
    assert!(
        list["last"]["unchanged_at"].as_i64().is_some(),
        "владелец видит, что проверка была: {list}"
    );

    // Сотрудник поменял данные — копия нужна.
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "person", "name": "Новый клиент"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(
        matches!(scheduled(&app.state).await, Scheduled::Saved(_)),
        "данные изменились — копия"
    );
}

#[tokio::test]
async fn temporary_files_go_to_backup_disk_not_to_database_disk() {
    let data = std::path::Path::new("/data");
    let usb = tempfile::tempdir().unwrap();
    let missing = usb.path().join("нет-такой").join("папки");
    let picked = crm_server::backup::workspace(&[missing.clone(), usb.path().to_owned()], data);
    assert_eq!(picked, usb.path(), "первая доступная папка копий");
    assert_eq!(
        crm_server::backup::workspace(&[missing], data),
        data,
        "нет доступных — рядом с базой"
    );

    // После копии временных файлов не остаётся ни там, ни там.
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, _) = app
        .call("POST", "/api/admin/backup/run", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    for d in [app.dir().join("data"), app.dir().join("backups")] {
        let tmp: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("tmp") || n.starts_with(".crm-"))
            .collect();
        assert!(tmp.is_empty(), "{}: {tmp:?}", d.display());
    }
}
