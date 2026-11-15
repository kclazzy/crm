//! Второй фактор: код из приложения-аутентификатора при входе из интернета.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use crm_core::totp::{base32_decode, code_at, STEP_SECS};
use serde_json::{json, Value};

const HOME: &str = "85.12.34.56";

fn now_step() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap() / STEP_SECS
}

fn code(secret: &str, step: u64) -> String {
    format!("{:06}", code_at(&base32_decode(secret).unwrap(), step))
}

/// Включить код себе; возвращает (секрет, резервные коды).
async fn enroll(app: &App, token: &str) -> (String, Vec<String>) {
    let (s, v) = app
        .call("POST", "/api/me/2fa/start", LOCAL, Some(token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let secret = v["secret"].as_str().unwrap().to_owned();
    assert!(v["qr_svg"].as_str().unwrap().contains("<svg"));
    assert!(v["url"].as_str().unwrap().starts_with("otpauth://totp/"));
    let (s, v) = app
        .call(
            "POST",
            "/api/me/2fa/confirm",
            LOCAL,
            Some(token),
            Some(json!({"code": "000000"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "неверный код не включает: {v}");
    let (s, v) = app
        .call(
            "POST",
            "/api/me/2fa/confirm",
            LOCAL,
            Some(token),
            Some(json!({"code": code(&secret, now_step())})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let backup: Vec<String> = v["backup_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(backup.len(), 10);
    (secret, backup)
}

async fn login(
    app: &App,
    user: &str,
    password: &str,
    ip: &str,
    code: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({
        "username": user, "password": password,
        "device_id": format!("dev-{ip}"), "device_name": "Тест"
    });
    if let Some(c) = code {
        body["code"] = json!(c);
    }
    app.call("POST", "/api/auth/login", ip, None, Some(body))
        .await
}

#[tokio::test]
async fn code_is_asked_from_internet_and_not_in_office() {
    let app = App::new().await;
    let owner = app.owner().await;
    let pass = "очень-надёжный-пароль";
    let (secret, backup) = enroll(&app, &owner).await;
    let (_, me) = app
        .call("GET", "/api/me/2fa", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(me["enabled"], true, "{me}");
    assert_eq!(me["backup_left"], 10);
    assert_eq!(
        me["policy"], "remote",
        "по умолчанию — код при входе из интернета"
    );

    // В офисе — как раньше, без кода.
    let (s, v) = login(&app, "owner", pass, OFFICE_PC, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Из интернета: без кода — просьба ввести код, сессии нет.
    let (s, v) = login(&app, "owner", pass, HOME, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["need_code"], true, "{v}");
    assert!(v["token"].is_null());
    let (s, v) = login(&app, "owner", "неверный-пароль-123", HOME, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(
        v["need_code"].is_null(),
        "при неверном пароле код не спрашиваем: {v}"
    );
    let (s, _) = login(&app, "owner", pass, HOME, Some("123456")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Код этого интервала уже потрачен при включении — следующий подходит, и только раз.
    let next = code(&secret, now_step() + 1);
    let (s, v) = login(&app, "owner", pass, HOME, Some(&next)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = login(&app, "owner", pass, HOME, Some(&next)).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "тот же код второй раз не принимается"
    );

    // Резервный код — один раз.
    let (s, v) = login(&app, "owner", pass, HOME, Some(&backup[0])).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = login(&app, "owner", pass, HOME, Some(&backup[0])).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (_, me) = app
        .call("GET", "/api/me/2fa", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(me["backup_left"], 9);
}

#[tokio::test]
async fn without_code_internet_login_is_refused_and_setup_happens_in_office() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (emp_id, _) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let pass = "пароль-сотрудника-1";

    // Кода нет — из интернета один пароль не пускает (иначе вор пароля привязал бы свой телефон).
    let (s, v) = login(&app, "ivan", pass, HOME, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"].as_str().unwrap().contains("в офисе"), "{v}");

    // В офисе входит и настраивает код — дальше из дома с кодом.
    let (s, v) = login(&app, "ivan", pass, OFFICE_PC, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token = v["token"].as_str().unwrap().to_owned();
    let (secret, _) = enroll(&app, &token).await;
    let (s, v) = login(
        &app,
        "ivan",
        pass,
        HOME,
        Some(&code(&secret, now_step() + 1)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Потерял телефон — владелец сбрасывает код.
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/admin/users/{emp_id}/2fa/reset"),
            LOCAL,
            Some(&token),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "сбрасывает только владелец");
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/admin/users/{emp_id}/2fa/reset"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = login(&app, "ivan", pass, HOME, None).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "после сброса — снова настройка в офисе"
    );
}

#[tokio::test]
async fn policy_all_makes_everyone_set_up_code_first() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, emp) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    let put = |p: &'static str, t: String| {
        let app = &app;
        async move {
            app.call(
                "PUT",
                "/api/admin/2fa",
                LOCAL,
                Some(&t),
                Some(json!({"policy": p})),
            )
            .await
        }
    };
    let (s, _) = put("all", emp.clone()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = put("all", owner.clone()).await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "сначала код себе — иначе владелец закроет дверь сам: {v}"
    );
    enroll(&app, &owner).await;
    let (s, v) = put("all", owner.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Сотрудник без кода: входит в офисе, но до настройки работать нельзя.
    let (s, v) = login(&app, "ivan", "пароль-сотрудника-1", OFFICE_PC, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["must_setup_2fa"], true, "{v}");
    let token = v["token"].as_str().unwrap().to_owned();
    let (s, v) = app
        .call("GET", "/api/clients", LOCAL, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["setup_2fa"], true, "{v}");
    let (s, _) = app.call("GET", "/api/me", LOCAL, Some(&token), None).await;
    assert_eq!(s, StatusCode::OK);
    enroll(&app, &token).await;
    let (s, _) = app
        .call("GET", "/api/clients", LOCAL, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "после настройки — работает");

    // Выключить свой код при обязательной политике нельзя.
    let (s, _) = app
        .call("DELETE", "/api/me/2fa", LOCAL, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = put("off", owner.clone()).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .call("DELETE", "/api/me/2fa", LOCAL, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, me) = app
        .call("GET", "/api/me/2fa", LOCAL, Some(&token), None)
        .await;
    assert_eq!(me["enabled"], false);
}

#[tokio::test]
async fn office_session_without_code_does_not_work_from_internet() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (secret, _) = enroll(&app, &owner).await;
    let (_, emp) = app.member(&owner, "ivan", "employee", OFFICE_PC).await;
    // Сессия, где код вводили при настройке, уже подтверждена; берём обычный вход в офисе.
    let (_, v) = login(&app, "owner", "очень-надёжный-пароль", OFFICE_PC, None).await;
    let owner = v["token"].as_str().unwrap().to_owned();

    // Вошли в офисе без кода, ноутбук унесли домой: та же сессия из интернета просит код.
    let (s, _) = app
        .call("GET", "/api/clients", OFFICE_PC, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = app
        .call("GET", "/api/clients", HOME, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["need_code"], true, "{v}");
    // Подтвердить кодом прямо в этой сессии — дальше работает и из дома.
    let (s, _) = app
        .call(
            "POST",
            "/api/auth/verify",
            HOME,
            Some(&owner),
            Some(json!({"code": "000000"})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = app
        .call(
            "POST",
            "/api/auth/verify",
            HOME,
            Some(&owner),
            Some(json!({"code": code(&secret, now_step() + 1)})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, _) = app
        .call("GET", "/api/clients", HOME, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);

    // Сотрудник без кода: офисная сессия из интернета не пускает совсем.
    let (s, v) = app.call("GET", "/api/me", HOME, Some(&emp), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"].as_str().unwrap().contains("в офисе"), "{v}");
    let (s, _) = app
        .call("GET", "/api/me", OFFICE_PC, Some(&emp), None)
        .await;
    assert_eq!(s, StatusCode::OK);
}
