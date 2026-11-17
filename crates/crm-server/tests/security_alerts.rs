//! Владелец узнаёт в колокольчике о попытках войти, просьбах сбросить пароль
//! и новых устройствах вне офиса — а не только из журнала.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::{json, Value};

async fn bell(app: &App, owner: &str) -> Vec<(String, Value)> {
    let (s, v) = app
        .call("GET", "/api/notifications", LOCAL, Some(owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["kind"].as_str().unwrap().to_owned(), n["payload"].clone()))
        .collect()
}

fn kinds(items: &[(String, Value)], kind: &str) -> Vec<Value> {
    items
        .iter()
        .filter(|(k, _)| k == kind)
        .map(|(_, p)| p.clone())
        .collect()
}

#[tokio::test]
async fn owner_hears_about_failed_logins_and_lockout() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.member(&owner, "ivan", "employee", OFFICE_PC).await;

    let internet = "85.12.34.56";
    let (s, _) = app.try_login("ivan", "не тот пароль", internet).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let items = bell(&app, &owner).await;
    let failed = kinds(&items, "login_failed");
    assert_eq!(failed.len(), 1, "{items:?}");
    assert_eq!(failed[0]["username"], "ivan");
    assert_eq!(failed[0]["place"], "internet");

    // Следующие ошибки того же логина не засыпают колокольчик…
    for _ in 0..4 {
        app.try_login("ivan", "не тот пароль", internet).await;
    }
    let items = bell(&app, &owner).await;
    assert_eq!(kinds(&items, "login_failed").len(), 1, "{items:?}");
    // …но о блокировке владелец узнаёт отдельно.
    let locked = kinds(&items, "login_locked");
    assert_eq!(locked.len(), 1, "{items:?}");
    assert_eq!(locked[0]["username"], "ivan");

    // Несуществующий логин — тоже попытка входа, о ней стоит знать.
    app.try_login("admin", "123456", internet).await;
    let items = bell(&app, &owner).await;
    assert_eq!(kinds(&items, "login_failed").len(), 2, "{items:?}");

    // Сотрудник уведомлений о чужих входах не получает.
    let (_, emp) = app.member(&owner, "petr", "employee", OFFICE_PC).await;
    assert!(bell(&app, &emp).await.is_empty());
}

#[tokio::test]
async fn owner_hears_about_password_reset_requests() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.member(&owner, "ivan", "employee", OFFICE_PC).await;

    let forgot = |u: &'static str| {
        let app = &app;
        async move {
            app.call(
                "POST",
                "/api/auth/forgot",
                OFFICE_PC,
                None,
                Some(json!({ "username": u })),
            )
            .await
        }
    };
    let (s, _) = forgot("ivan").await;
    assert_eq!(s, StatusCode::OK);
    let items = bell(&app, &owner).await;
    let asked = kinds(&items, "reset_requested");
    assert_eq!(asked.len(), 1, "{items:?}");
    assert_eq!(asked[0]["username"], "ivan");

    // Повтор и несуществующий логин — без новых уведомлений.
    forgot("ivan").await;
    forgot("nobody").await;
    assert_eq!(kinds(&bell(&app, &owner).await, "reset_requested").len(), 1);
}

#[tokio::test]
async fn owner_hears_about_new_device_waiting_for_approval() {
    let app = App::new().await;
    let owner = app.owner().await;
    let peer = crm_p2p::Peer {
        id: "1a2b".repeat(16),
        direct: None,
    };
    crm_server::p2p::alert_pending(&app.state, &peer)
        .await
        .unwrap();
    crm_server::p2p::alert_pending(&app.state, &peer)
        .await
        .unwrap();
    let items = bell(&app, &owner).await;
    let pending = kinds(&items, "device_pending");
    assert_eq!(
        pending.len(),
        1,
        "повтор того же устройства не дублируется: {items:?}"
    );
    assert_eq!(pending[0]["code"], "1A2B-1A2B");
}
