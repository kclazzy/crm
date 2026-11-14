//! Поведение сервера снаружи: старт на пустой папке, здоровье, настройки БД, поиск.
#![allow(clippy::unwrap_used)] // в тестах допускается (ТЗ §10)

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::config_in;
use crm_server::{config::Config, db, http};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn health_reports_ok_on_fresh_install() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&config_in(dir.path()).storage).await.unwrap();

    let res = http::router(http::AppState::new(pool, Config::default(), None))
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn database_is_created_in_wal_mode_with_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&config_in(dir.path()).storage).await.unwrap();

    assert!(dir.path().join("data").join("crm.db").exists());
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fk, 1);
}

#[tokio::test]
async fn reopening_existing_database_is_safe() {
    let dir = tempfile::tempdir().unwrap();
    let storage = config_in(dir.path()).storage;
    let count = |pool: sqlx::SqlitePool| async move {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        n
    };
    let first = count(db::open(&storage).await.unwrap()).await;
    let second = count(db::open(&storage).await.unwrap()).await;
    assert!(first >= 1);
    assert_eq!(
        first, second,
        "повторное открытие не применяет миграции заново"
    );
}

#[tokio::test]
async fn client_is_found_by_part_of_word_in_russian() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&config_in(dir.path()).storage).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO users(id, username, display_name, password_hash, role, created_at) VALUES ('u','owner','Владелец','x','host',0);
         INSERT INTO groups(id, name, created_at) VALUES ('g','Офис',0);
         INSERT INTO clients(id, kind, name, description, group_id, created_by, created_at)
             VALUES ('c','person','Иванова Мария','Поставка оборудования','g','u',0);",
    )
    .execute(&pool)
    .await
    .unwrap();

    let found: Vec<String> =
        sqlx::query_scalar("SELECT entity_id FROM search_index WHERE search_index MATCH 'иванов'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(found, vec!["c".to_string()]);
    let by_description: Vec<String> =
        sqlx::query_scalar("SELECT entity_id FROM search_index WHERE search_index MATCH 'оборуд'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(by_description, vec!["c".to_string()]);
}

#[test]
fn config_rejects_unknown_fields_and_bad_address() {
    assert!(Config::from_toml("listen = \"not an address\"").is_err());
    assert!(Config::from_toml("listen = \"0.0.0.0:8443\"\ntypo = 1").is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn data_folder_is_private_to_the_server_account() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    db::open(&config_in(dir.path()).storage).await.unwrap();
    let mode = std::fs::metadata(dir.path().join("data"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o077,
        0,
        "другие пользователи компьютера не должны читать БД: {mode:o}"
    );
}
