//! Ключ подключения и VPN из админ-панели.
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
async fn vpn_wizard_issues_device_configs_and_revokes_them() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (s, v) = app
        .call(
            "PUT",
            "/api/admin/vpn",
            LOCAL,
            Some(&owner),
            Some(json!({"endpoint": "192.168.1.1"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("внешний"));
    let (s, v) = app
        .call(
            "POST",
            "/api/admin/vpn/peers",
            LOCAL,
            Some(&owner),
            Some(json!({"name": "Ноутбук"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "сначала адрес офиса: {v}");

    let (s, _) = app
        .call(
            "PUT",
            "/api/admin/vpn",
            LOCAL,
            Some(&owner),
            Some(json!({"endpoint": "office.keenetic.pro"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let conf_path = app.dir().join("data/wg/crm-vpn.conf");
    let server_key = std::fs::read_to_string(app.dir().join("data/wg/server.key")).unwrap();

    let (s, a) = app
        .call(
            "POST",
            "/api/admin/vpn/peers",
            LOCAL,
            Some(&owner),
            Some(json!({"name": "Ноутбук Ивана"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    assert_eq!(a["address"], "10.77.0.2");
    let conf = a["conf"].as_str().unwrap();
    assert!(conf.contains("Endpoint = office.keenetic.pro:51820"));
    assert!(conf.contains("AllowedIPs = 10.77.0.1/32"));
    assert!(
        a["qr_svg"].as_str().unwrap().starts_with("<?xml")
            || a["qr_svg"].as_str().unwrap().contains("<svg")
    );
    let (_, b) = app
        .call(
            "POST",
            "/api/admin/vpn/peers",
            LOCAL,
            Some(&owner),
            Some(json!({"name": "Телефон"})),
        )
        .await;
    assert_eq!(b["address"], "10.77.0.3");

    let server = std::fs::read_to_string(&conf_path).unwrap();
    assert!(server.contains(server_key.trim()));
    assert_eq!(server.matches("[Peer]").count(), 2);
    let device_private = conf
        .lines()
        .find_map(|l| l.strip_prefix("PrivateKey = "))
        .unwrap();
    assert!(
        !server.contains(device_private),
        "закрытый ключ устройства на сервере не хранится"
    );

    // Повторная настройка адреса не меняет ключи сервера — выданные файлы продолжают работать.
    app.call(
        "PUT",
        "/api/admin/vpn",
        LOCAL,
        Some(&owner),
        Some(json!({"endpoint": "85.12.34.56"})),
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(app.dir().join("data/wg/server.key")).unwrap(),
        server_key
    );

    let id = a["id"].as_str().unwrap();
    let (s, _) = app
        .call(
            "DELETE",
            &format!("/api/admin/vpn/peers/{id}"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let server = std::fs::read_to_string(&conf_path).unwrap();
    assert_eq!(server.matches("[Peer]").count(), 1);
    let (_, info) = app
        .call("GET", "/api/admin/vpn", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(info["configured"], true);
    assert_eq!(info["peers"].as_array().unwrap().len(), 1);

    // Запрос через VPN отмечает устройство.
    let (_, _) = app
        .call("GET", "/api/health", "10.77.0.3", None, None)
        .await;
    let (_, info) = app
        .call("GET", "/api/admin/vpn", LOCAL, Some(&owner), None)
        .await;
    assert!(info["peers"][0]["last_seen_at"].is_i64(), "{info}");
}

#[tokio::test]
async fn phone_links_give_qr_for_certificate_office_and_vpn() {
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
    assert!(!kinds.contains(&"vpn"), "VPN ещё не настроен: {v}");
    if let Some(cert) = v.as_array().unwrap().iter().find(|l| l["kind"] == "cert") {
        assert!(cert["url"].as_str().unwrap().ends_with("/api/tls/ca"));
        assert!(cert["qr_svg"].as_str().unwrap().contains("<svg"));
    }
    app.call(
        "PUT",
        "/api/admin/vpn",
        LOCAL,
        Some(&owner),
        Some(json!({"endpoint": "office.example.ru"})),
    )
    .await;
    let (_, v) = app
        .call("GET", "/api/admin/phone", LOCAL, Some(&owner), None)
        .await;
    let vpn = v
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["kind"] == "vpn")
        .unwrap();
    assert!(
        vpn["url"]
            .as_str()
            .unwrap()
            .starts_with("https://10.77.0.1:"),
        "{v}"
    );

    let (_, emp) = app.member(&owner, "olga", "employee", "10.0.0.3").await;
    let (s, _) = app
        .call("GET", "/api/admin/phone", LOCAL, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
