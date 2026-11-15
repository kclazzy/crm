//! Восстановление пароля: коды владельца, сброс на сервере, запрос сотрудника.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::json;

const NEW_PW: &str = "новый-пароль-владельца";

async fn codes(app: &App, owner: &str) -> Vec<String> {
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/recovery-codes",
            LOCAL,
            Some(owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect()
}

async fn recover(app: &App, ip: &str, code: &str) -> (StatusCode, serde_json::Value) {
    app.call(
        "POST",
        "/api/auth/recover",
        ip,
        None,
        Some(json!({ "username": "owner", "code": code, "new_password": NEW_PW })),
    )
    .await
}

#[tokio::test]
async fn owner_regains_access_with_one_time_code() {
    let app = App::new().await;
    let owner = app.owner().await;
    let list = codes(&app, &owner).await;
    assert_eq!(list.len(), 10);
    let (_, st) = app
        .call(
            "GET",
            "/api/admin/recovery-codes",
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(st["unused"], 10);

    let (s, v) = recover(&app, OFFICE_PC, &list[0]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        app.call("GET", "/api/me", LOCAL, Some(&owner), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "старые входы закрыты"
    );
    assert_eq!(
        app.try_login("owner", "очень-надёжный-пароль", OFFICE_PC)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    app.login("owner", NEW_PW, OFFICE_PC).await;
    assert_eq!(
        recover(&app, OFFICE_PC, &list[0]).await.0,
        StatusCode::UNAUTHORIZED,
        "код одноразовый"
    );
}

#[tokio::test]
async fn new_codes_replace_old_ones_and_only_owner_gets_them() {
    let app = App::new().await;
    let owner = app.owner().await;
    let old = codes(&app, &owner).await;
    let _new = codes(&app, &owner).await;
    assert_eq!(
        recover(&app, OFFICE_PC, &old[1]).await.0,
        StatusCode::UNAUTHORIZED,
        "старые коды больше не действуют"
    );

    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    let (_, v) = app.accept(&invite, "секретный-пароль-1", OFFICE_PC).await;
    let ivan = v["token"].as_str().unwrap();
    assert_eq!(
        app.call(
            "POST",
            "/api/admin/recovery-codes",
            OFFICE_PC,
            Some(ivan),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn guessing_codes_is_blocked() {
    let app = App::new().await;
    let owner = app.owner().await;
    let list = codes(&app, &owner).await;
    for _ in 0..5 {
        assert_eq!(
            recover(&app, OFFICE_PC, "AAAA-BBBB-CCCC").await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        recover(&app, OFFICE_PC, &list[2]).await.0,
        StatusCode::TOO_MANY_REQUESTS,
        "после 5 ошибок — пауза"
    );
}

#[tokio::test]
async fn reset_on_server_machine_sets_password_and_closes_sessions() {
    let app = App::new().await;
    let owner = app.owner().await;
    // Код из приложения включён и обязателен везде, а телефон потерян.
    let (_, v) = app
        .call("POST", "/api/me/2fa/start", LOCAL, Some(&owner), None)
        .await;
    let secret = crm_core::totp::base32_decode(v["secret"].as_str().unwrap()).unwrap();
    let step = u64::try_from(chrono::Utc::now().timestamp()).unwrap() / 30;
    let code = format!("{:06}", crm_core::totp::code_at(&secret, step));
    let (s, _) = app
        .call(
            "POST",
            "/api/me/2fa/confirm",
            LOCAL,
            Some(&owner),
            Some(json!({"code": code})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .call(
            "PUT",
            "/api/admin/2fa",
            LOCAL,
            Some(&owner),
            Some(json!({"policy": "all"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let storage = common::config_in(app.dir()).storage;
    assert!(
        crm_server::recovery::reset_owner_password(&storage, "короткий")
            .await
            .is_err(),
        "правила пароля те же"
    );
    let name = crm_server::recovery::reset_owner_password(&storage, NEW_PW)
        .await
        .unwrap();
    assert_eq!(name, "owner");
    assert_eq!(
        app.call("GET", "/api/me", LOCAL, Some(&owner), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // Сброс на сервере снимает и код: владелец входит паролем и настраивает код заново.
    let (s, v) = app.try_login("owner", NEW_PW, OFFICE_PC).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["must_setup_2fa"], true, "{v}");
}

#[tokio::test]
async fn employee_asks_for_reset_and_owner_sees_request() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (id, invite) = app.invite(&owner, "ivan", "employee").await;
    app.accept(&invite, "секретный-пароль-1", OFFICE_PC).await;

    let real = app
        .call(
            "POST",
            "/api/auth/forgot",
            OFFICE_PC,
            None,
            Some(json!({ "username": "ivan" })),
        )
        .await;
    let fake = app
        .call(
            "POST",
            "/api/auth/forgot",
            OFFICE_PC,
            None,
            Some(json!({ "username": "nobody" })),
        )
        .await;
    assert_eq!(real.0, StatusCode::OK);
    assert_eq!(real, fake, "по ответу не узнать, есть ли такой логин");

    let (_, staff) = app
        .call("GET", "/api/admin/users", LOCAL, Some(&owner), None)
        .await;
    let ivan = staff
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "ivan")
        .unwrap()
        .clone();
    assert_eq!(ivan["reset_requested"], true);

    app.call(
        "POST",
        &format!("/api/admin/users/{id}/reset"),
        LOCAL,
        Some(&owner),
        None,
    )
    .await;
    let (_, staff) = app
        .call("GET", "/api/admin/users", LOCAL, Some(&owner), None)
        .await;
    let ivan = staff
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "ivan")
        .unwrap()
        .clone();
    assert_eq!(
        ivan["reset_requested"], false,
        "после сброса пометка снимается"
    );
}
