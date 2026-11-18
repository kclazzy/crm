//! Первый вход из дома через программу CRM по ключу (с PIN): устройство одобрено
//! владельцем — сотрудник входит и сразу настраивает код из приложения, а не
//! получает отказ «настройте в офисе». Одного пароля по-прежнему мало: без
//! одобренного устройства из интернета без кода не войти.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use crm_core::totp::{base32_decode, code_at, STEP_SECS};
use crm_server::p2p::{admit, origin, Admit};
use serde_json::json;

const PW: &str = "пароль-сотрудника-1";

fn device(n: u16) -> crm_p2p::Peer {
    crm_p2p::Peer {
        id: format!("{n:04x}").repeat(16),
        direct: None,
    }
}

fn addr(peer: &crm_p2p::Peer) -> String {
    format!("[{}]", origin(peer).ip())
}

/// Владелец, сотрудник ivan и устройство ivan, одобренное владельцем.
async fn office_with_approved_laptop() -> (App, String, String) {
    let app = App::new().await;
    let owner = app.owner().await;
    app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let laptop = device(7);
    assert!(matches!(
        admit(&app.state.pool, &laptop).await.unwrap(),
        Admit::Pending { .. }
    ));
    let (s, v) = app
        .call(
            "PUT",
            &format!("/api/admin/remote/devices/{}", laptop.id),
            LOCAL,
            Some(&owner),
            Some(json!({"approved": true, "name": "Ноутбук Ивана"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    (app, owner, addr(&laptop))
}

#[tokio::test]
async fn first_login_from_home_on_approved_laptop_sets_up_the_code() {
    let (app, _, home) = office_with_approved_laptop().await;
    let (s, v) = app.try_login("ivan", PW, &home).await;
    assert_eq!(s, StatusCode::OK, "вход не отклонён: {v}");
    assert_eq!(v["must_setup_2fa"], true, "сначала — код из приложения");
    let token = v["token"].as_str().unwrap().to_owned();

    let (s, v) = app
        .call("GET", "/api/tasks", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(
        v["setup_2fa"], true,
        "до настройки кода — только настройка: {v}"
    );

    let (s, v) = app
        .call("POST", "/api/me/2fa/start", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "настроить можно прямо из дома: {v}");
    let secret = base32_decode(v["secret"].as_str().unwrap()).unwrap();
    let step = u64::try_from(chrono::Utc::now().timestamp()).unwrap() / STEP_SECS;
    let (s, v) = app
        .call(
            "POST",
            "/api/me/2fa/confirm",
            &home,
            Some(&token),
            Some(json!({"code": format!("{:06}", code_at(&secret, step))})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = app
        .call("GET", "/api/tasks", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "после настройки — работа");

    // Следующий вход из дома — уже с кодом.
    let (s, v) = app.try_login("ivan", PW, &home).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["need_code"], true, "{v}");
}

#[tokio::test]
async fn password_alone_from_unknown_place_is_still_not_enough() {
    let (app, _, _) = office_with_approved_laptop().await;
    let stranger = device(9);
    assert!(matches!(
        admit(&app.state.pool, &stranger).await.unwrap(),
        Admit::Pending { .. }
    ));
    for from in [addr(&stranger), "85.12.34.56".to_owned()] {
        let (s, v) = app.try_login("ivan", PW, &from).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{from}: {v}");
        let msg = v["error"].as_str().unwrap();
        assert!(msg.contains("одобрен"), "подсказка, как войти: {msg}");
    }
}

/// Вошёл в офисе (без кода), открыл ту же сессию дома на одобренном ноутбуке —
/// не тупик, а настройка кода; с чужого адреса — отказ.
#[tokio::test]
async fn office_session_used_at_home_leads_to_code_setup() {
    let (app, _, home) = office_with_approved_laptop().await;
    let token = app.login("ivan", PW, OFFICE_PC).await;
    let (s, v) = app
        .call("GET", "/api/tasks", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(v["setup_2fa"], true, "{v}");
    let (s, _) = app.call("GET", "/api/me", &home, Some(&token), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .call("POST", "/api/me/2fa/start", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = app
        .call("GET", "/api/tasks", "85.12.34.56", Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_ne!(
        v["setup_2fa"], true,
        "из интернета без устройства настройка не открывается"
    );
}

/// Ровно случай из жалобы: новый сотрудник дома задал пароль по приглашению
/// из ключа — и упирался в «настройте код в офисе», задач не видно. Теперь сразу
/// после пароля открывается настройка кода, после неё — задачи.
#[tokio::test]
async fn new_employee_at_home_goes_from_password_to_code_to_tasks() {
    let (app, owner, home) = office_with_approved_laptop().await;
    let (_, invite) = app.invite(&owner, "petr", "employee").await;
    let (s, v) = app.accept(&invite, PW, &home).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["must_setup_2fa"], true);
    let token = v["token"].as_str().unwrap().to_owned();

    let (s, v) = app
        .call("GET", "/api/tasks", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(v["setup_2fa"], true, "не тупик, а настройка кода: {v}");
    let (s, v) = app
        .call("POST", "/api/me/2fa/start", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let secret = base32_decode(v["secret"].as_str().unwrap()).unwrap();
    let step = u64::try_from(chrono::Utc::now().timestamp()).unwrap() / STEP_SECS;
    let (s, _) = app
        .call(
            "POST",
            "/api/me/2fa/confirm",
            &home,
            Some(&token),
            Some(json!({"code": format!("{:06}", code_at(&secret, step))})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .call("GET", "/api/tasks", &home, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "задачи видны");
}
