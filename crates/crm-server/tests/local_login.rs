//! Вход владельца на самом сервере без пароля: одноразовый билет.
//! Билет выдают двое — создание владельца (сразу после ввода пароля) и
//! администратор Windows через `crm-server owner-ticket` (у него и так есть доступ
//! к базе). Обменять билет на сессию можно только с этого же компьютера.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

const PW: &str = "очень-надёжный-пароль";

async fn create_owner(app: &App) -> String {
    let (s, v) = app
        .call(
            "POST",
            "/api/setup",
            LOCAL,
            None,
            Some(json!({"username": "boss", "display_name": "Шеф", "password": PW})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["username"], "boss");
    v["ticket"].as_str().expect("билет в ответе").to_owned()
}

/// Обменять билет: (статус, куда ведёт, cookie сессии если выдана).
async fn enter(app: &App, ip: &str, ticket: &str) -> (StatusCode, String, Option<String>) {
    let (s, h, _) = app
        .raw_from(
            ip,
            "GET",
            &format!("/api/auth/ticket?t={ticket}"),
            &[],
            None,
        )
        .await;
    let to = h
        .get("location")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let cookie = h
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().to_owned())
        .filter(|c| c.starts_with("crm_session=") && !c.starts_with("crm_session=;"))
        .map(|c| {
            c["crm_session=".len()..]
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        });
    (s, to, cookie)
}

#[tokio::test]
async fn owner_enters_right_after_creation_without_typing_password() {
    let app = App::new().await;
    let ticket = create_owner(&app).await;
    let (s, to, cookie) = enter(&app, LOCAL, &ticket).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(to, "/");
    let token = cookie.expect("сессия выдана");
    let (s, me) = app.call("GET", "/api/me", LOCAL, Some(&token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["username"], "boss");

    // Билет одноразовый.
    let (s, to, cookie) = enter(&app, LOCAL, &ticket).await;
    assert_eq!((s, to.as_str()), (StatusCode::SEE_OTHER, "/"));
    assert!(cookie.is_none(), "второй раз билет не действует");
}

#[tokio::test]
async fn ticket_works_only_on_the_server_itself() {
    let app = App::new().await;
    let ticket = create_owner(&app).await;
    for ip in [OFFICE_PC, "85.12.34.56", "[2001:db8::7]"] {
        let (_, _, cookie) = enter(&app, ip, &ticket).await;
        assert!(cookie.is_none(), "{ip}: билет только с этого компьютера");
    }
    // Попытка с чужого адреса билет не сжигает.
    assert!(enter(&app, LOCAL, &ticket).await.2.is_some());
}

#[tokio::test]
async fn wrong_or_stale_ticket_gives_nothing() {
    let app = App::new().await;
    let ticket = create_owner(&app).await;
    assert!(enter(&app, LOCAL, "не-тот-билет").await.2.is_none());
    sqlx::query("UPDATE local_tickets SET expires_at = 0")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(
        enter(&app, LOCAL, &ticket).await.2.is_none(),
        "просроченный"
    );
}

/// Администратор Windows: программа передаёт только хеш билета (виден в списке
/// процессов), сам билет остаётся у неё.
#[tokio::test]
async fn windows_admin_issues_ticket_for_the_owner() {
    let app = App::new().await;
    create_owner(&app).await;
    let ticket = "билет-от-администратора-windows-0123456789";
    let storage = common::config_in(app.dir()).storage;
    let name =
        crm_server::local_login::issue_offline(&storage, &crm_server::secrets::token_hash(ticket))
            .await
            .unwrap();
    assert_eq!(name, "boss");
    let token = enter(&app, LOCAL, ticket).await.2.expect("вход");
    let (_, me) = app.call("GET", "/api/me", LOCAL, Some(&token), None).await;
    assert_eq!(me["role"], "host");
}

#[tokio::test]
async fn no_owner_no_ticket() {
    let app = App::new().await;
    let storage = common::config_in(app.dir()).storage;
    assert!(
        crm_server::local_login::issue_offline(&storage, &"0".repeat(64))
            .await
            .is_err()
    );
}
