//! Программа CRM раздаётся с самого сервера: в офисе её не нужно нести на флешке.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, OFFICE_PC};

#[tokio::test]
async fn office_computer_downloads_the_program_from_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let setup = dir.path().join("CRM-Setup.exe");
    // Установщик ещё не положил копию — скачивать нечего, и страница входа это знает.
    std::env::set_var("CRM_INSTALLER", &setup);
    let app = App::new().await;
    let (s, v) = app
        .call("GET", "/api/download", OFFICE_PC, None, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["available"], false);
    let (s, _, _) = app.raw("GET", "/download/CRM-Setup.exe", &[], None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    std::fs::write(&setup, b"MZ fake installer").unwrap();
    let (_, v) = app
        .call("GET", "/api/download", OFFICE_PC, None, None)
        .await;
    assert_eq!(v["available"], true, "{v}");
    assert_eq!(v["size"], 17);
    // Без входа: программа нужна как раз тому, кто ещё не может войти.
    let (s, h, body) = app.raw("GET", "/download/CRM-Setup.exe", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment"));
    assert_eq!(body, "MZ fake installer");
}
