//! Веб-интерфейс глазами браузера: вход через cookie, админ-панель владельца, раздача страниц.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, OFFICE_PC};
use serde_json::json;

const LOGIN: &str = r#"{"username":"owner","password":"очень-надёжный-пароль","device_id":"browser-1","device_name":"Chrome"}"#;

fn login_body() -> serde_json::Value {
    serde_json::from_str(LOGIN).unwrap()
}

fn session_cookie(headers: &axum::http::HeaderMap) -> String {
    let set = headers
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    set.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn browser_login_sets_protected_cookie_that_authenticates() {
    let app = App::new().await;
    app.owner().await;
    let (s, h, _) = app
        .raw("POST", "/api/auth/login", &[], Some(login_body()))
        .await;
    assert_eq!(s, StatusCode::OK);
    let set = h.get("set-cookie").unwrap().to_str().unwrap().to_owned();
    for part in [
        "crm_session=",
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/",
    ] {
        assert!(set.contains(part), "в cookie нет {part}: {set}");
    }
    let cookie = session_cookie(&h);
    let (s, _, me) = app
        .raw("GET", "/api/me", &[("cookie", &cookie)], None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(me.contains("\"role\":\"host\""), "{me}");
}

#[tokio::test]
async fn browser_logout_clears_cookie_and_ends_session() {
    let app = App::new().await;
    app.owner().await;
    let (_, h, _) = app
        .raw("POST", "/api/auth/login", &[], Some(login_body()))
        .await;
    let cookie = session_cookie(&h);
    let (s, h, _) = app
        .raw("POST", "/api/auth/logout", &[("cookie", &cookie)], None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(h
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    let (s, _, _) = app
        .raw("GET", "/api/me", &[("cookie", &cookie)], None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn owner_sees_staff_list_with_invite_state_employee_does_not() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    app.invite(&owner, "maria", "manager").await;

    let (s, list) = app
        .call("GET", "/api/admin/users", common::LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 3);
    let ivan = list.iter().find(|u| u["username"] == "ivan").unwrap();
    assert_eq!(ivan["state"], "invited", "пароль ещё не задан");
    assert_eq!(
        list.iter().find(|u| u["username"] == "owner").unwrap()["state"],
        "active"
    );

    let (_, v) = app.accept(&invite, "пароль-сотрудника-1", OFFICE_PC).await;
    let (s, _) = app
        .call(
            "GET",
            "/api/admin/users",
            OFFICE_PC,
            v["token"].as_str(),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, list) = app
        .call("GET", "/api/admin/users", common::LOCAL, Some(&owner), None)
        .await;
    let ivan = list
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "ivan")
        .unwrap()
        .clone();
    assert_eq!(ivan["state"], "active");
    assert_eq!(ivan["display_name"], "Сотрудник");
    assert!(ivan.get("password_hash").is_none());
}

#[tokio::test]
async fn pages_are_served_with_security_headers_and_spa_fallback() {
    let app = App::new().await;
    for path in ["/", "/clients", "/admin/staff"] {
        let (s, h, body) = app.raw("GET", path, &[], None).await;
        assert_eq!(s, StatusCode::OK, "{path}");
        assert!(
            h["content-type"].to_str().unwrap().starts_with("text/html"),
            "{path}"
        );
        assert!(body.contains("<div id=\"root\""), "{path}: {body}");
        let csp = h["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"),
            "{csp}"
        );
    }
    let (s, h, _) = app.raw("GET", "/api/nothing-here", &[], None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "неизвестный API — не страница");
    assert_eq!(h["x-content-type-options"], "nosniff");
    let (_, h, _) = app.raw("GET", "/api/health", &[], None).await;
    assert_eq!(h["referrer-policy"], "no-referrer");
}

#[tokio::test]
async fn invite_accept_in_browser_also_sets_cookie() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    let body = json!({"token": invite, "password": "пароль-сотрудника-1", "device_id": "b", "device_name": "Edge"});
    let (s, h, _) = app
        .raw("POST", "/api/invites/accept", &[], Some(body))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(h
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("HttpOnly"));
}
