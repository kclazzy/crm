//! Ключ подключения и доступ через интернет из админ-панели.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL};
use crm_core::connkey::decode;
use serde_json::json;

#[tokio::test]
async fn key_with_pin_carries_invite_that_works_once() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (id, _old_invite) = app.invite(&owner, "ivan", "employee").await;
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/connection-key",
            LOCAL,
            Some(&owner),
            Some(json!({"user_id": id, "pin": "4821"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let key = v["key"].as_str().unwrap();
    assert!(key.starts_with("CRM1-"));
    assert!(decode(key, None).is_err(), "без PIN не открыть");
    let k = decode(key, Some("4821")).unwrap();
    assert_eq!(k.fp, "AB:CD:EF:01");
    assert_eq!(k.user.as_deref(), Some("ivan"));
    assert!(!k.addrs.is_empty());

    let (s, v) = app
        .accept(
            k.invite.as_deref().unwrap(),
            "пароль-сотрудника-1",
            "10.0.0.2",
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Сотрудник с паролем — ключ с приглашением уже не нужен.
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/connection-key",
            LOCAL,
            Some(&owner),
            Some(json!({"user_id": id})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/connection-key",
            LOCAL,
            Some(&owner),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(decode(v["key"].as_str().unwrap(), None)
        .unwrap()
        .invite
        .is_none());

    let (_, emp) = app.member(&owner, "olga", "employee", "10.0.0.3").await;
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/connection-key",
            LOCAL,
            Some(&emp),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn internet_address_goes_to_key_phone_and_health() {
    let app = App::new().await;
    let owner = app.owner().await;
    let get = || app.call("GET", "/api/admin/remote", LOCAL, Some(&owner), None);
    let (s, v) = get().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["address"].is_null(), "по умолчанию выключено: {v}");
    let put = |a: &'static str| {
        app.call(
            "PUT",
            "/api/admin/remote",
            LOCAL,
            Some(&owner),
            Some(json!({"address": a})),
        )
    };
    let (s, v) = put("192.168.1.10").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "внутренний адрес: {v}");
    let (s, v) = put("https://Office.Example.ru/").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["address"], "office.example.ru");
    assert_eq!(get().await.1["address"], "office.example.ru");

    // Ключ: офисные адреса, затем внешний — последним.
    let (_, v) = app
        .call(
            "POST",
            "/api/admin/connection-key",
            LOCAL,
            Some(&owner),
            Some(json!({})),
        )
        .await;
    let k = decode(v["key"].as_str().unwrap(), None).unwrap();
    assert_eq!(
        k.addrs.last().map(String::as_str),
        Some("office.example.ru"),
        "{:?}",
        k.addrs
    );
    assert!(
        !k.addrs.iter().any(|a| a.starts_with("10.77.")),
        "VPN убран: {:?}",
        k.addrs
    );

    // Программы сотрудников узнают внешний адрес из ответа сервера.
    let (_, h) = app.call("GET", "/api/health", LOCAL, None, None).await;
    assert_eq!(h["public_addr"], "office.example.ru", "{h}");

    // Телефон: QR-код «из интернета».
    let (_, v) = app
        .call("GET", "/api/admin/phone", LOCAL, Some(&owner), None)
        .await;
    let net = v
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["kind"] == "internet")
        .unwrap();
    assert!(
        net["url"]
            .as_str()
            .unwrap()
            .starts_with("https://office.example.ru:"),
        "{v}"
    );

    // Выключить — пустой адрес.
    let (s, _) = put("").await;
    assert_eq!(s, StatusCode::OK);
    let (_, h) = app.call("GET", "/api/health", LOCAL, None, None).await;
    assert!(h["public_addr"].is_null());

    let (_, emp) = app.member(&owner, "olga", "employee", "10.0.0.3").await;
    let (s, _) = app
        .call("GET", "/api/admin/remote", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app
        .call("GET", "/api/admin/vpn", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "WireGuard убран");
}

#[tokio::test]
async fn phone_links_give_qr_for_certificate_and_office() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call("GET", "/api/admin/phone", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let kinds: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["kind"].as_str().unwrap())
        .collect();
    assert!(
        !kinds.contains(&"internet"),
        "внешний адрес ещё не задан: {v}"
    );
    if let Some(cert) = v.as_array().unwrap().iter().find(|l| l["kind"] == "cert") {
        assert!(cert["url"].as_str().unwrap().ends_with("/api/tls/ca"));
        assert!(cert["qr_svg"].as_str().unwrap().contains("<svg"));
    }
    let (_, emp) = app.member(&owner, "olga", "employee", "10.0.0.3").await;
    let (s, _) = app
        .call("GET", "/api/admin/phone", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
