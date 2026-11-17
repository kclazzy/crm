//! Подключение программы в офисе по короткому коду: владелец диктует 8 символов,
//! программа сама получает ключ подключения — без длинного ключа и интернета.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::{App, LOCAL, OFFICE_PC};
use crm_core::connkey::decode;
use crm_core::enroll::{program_start, Answer, Code, EnrollError};
use serde_json::{json, Value};

const FP: &str = "AB:CD:EF:01";

async fn new_code(app: &App, owner: &str, user_id: &str) -> (StatusCode, Value) {
    app.call(
        "POST",
        &format!("/api/admin/users/{user_id}/enroll-code"),
        LOCAL,
        Some(owner),
        None,
    )
    .await
}

/// То, что делает программа: обмен по коду, ответ сервера, расшифровка ключа.
async fn enroll(app: &App, code: &str, fp: &str) -> Result<String, String> {
    let code = Code::parse(code).map_err(|e| e.to_string())?;
    let (program, msg) = program_start(&code, fp);
    let (s, v) = app
        .call(
            "POST",
            "/api/enroll",
            OFFICE_PC,
            None,
            Some(json!({ "id": code.id(), "msg": B64.encode(msg) })),
        )
        .await;
    if s != StatusCode::OK {
        return Err(format!("{s}: {v}"));
    }
    let b = |k: &str| B64.decode(v[k].as_str().unwrap()).unwrap();
    let answer = Answer {
        msg: b("msg"),
        nonce: b("nonce").try_into().unwrap(),
        sealed: b("sealed"),
    };
    program
        .finish(&answer)
        .map_err(|e: EnrollError| e.to_string())
}

#[tokio::test]
async fn new_employee_joins_with_a_short_code_and_sets_a_password() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (id, _) = app.invite(&owner, "ivan", "employee").await;

    let (s, v) = new_code(&app, &owner, &id).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = v["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 9, "{v}");
    assert!(v["expires_at"].as_i64().unwrap() > 0);

    // Программа вводит код как придётся — строчными и без дефиса.
    let key = enroll(&app, &code.to_lowercase().replace('-', ""), FP)
        .await
        .unwrap();
    let k = decode(&key, None).unwrap();
    assert_eq!(k.fp, FP);
    assert_eq!(k.user.as_deref(), Some("ivan"));
    // В ключе — приглашение: программа сразу откроет «Задайте пароль».
    let (s, v) = app
        .accept(
            k.invite.as_deref().unwrap(),
            "пароль-сотрудника-1",
            OFFICE_PC,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Тот же код на новом компьютере уже без приглашения: пароль задан.
    let k = decode(&enroll(&app, &code, FP).await.unwrap(), None).unwrap();
    assert_eq!((k.invite, k.user.as_deref()), (None, Some("ivan")));
}

#[tokio::test]
async fn code_is_for_owner_to_give_and_has_limited_attempts() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (id, _) = app.invite(&owner, "ivan", "employee").await;
    let (_, emp) = app.member(&owner, "petr", "employee", OFFICE_PC).await;
    let (s, _) = new_code(&app, &emp, &id).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "код выдаёт только владелец");

    let (_, v) = new_code(&app, &owner, &id).await;
    let code = v["code"].as_str().unwrap().to_owned();
    // Не тот секрет — ключ не открывается.
    let typo = format!("{}-0000", &code[..4]);
    let typo = if typo == code {
        format!("{}-1111", &code[..4])
    } else {
        typo
    };
    assert!(enroll(&app, &typo, FP).await.is_err());
    // Подмена сервера посередине (другой отпечаток) — тоже.
    assert!(enroll(&app, &code, "00:00:00:00").await.is_err());
    // Перебор: после 10 попыток код сгорает, даже верный больше не работает.
    for _ in 0..8 {
        let _ = enroll(&app, &typo, FP).await;
    }
    let err = enroll(&app, &code, FP).await.unwrap_err();
    assert!(err.starts_with("404"), "{err}");
    // Неизвестный код — тот же ответ, без подсказок.
    assert!(enroll(&app, "ZZZZ-ZZZZ", FP)
        .await
        .unwrap_err()
        .starts_with("404"));

    // Новый код заменяет старый.
    let (_, v1) = new_code(&app, &owner, &id).await;
    let (_, v2) = new_code(&app, &owner, &id).await;
    let (c1, c2) = (v1["code"].as_str().unwrap(), v2["code"].as_str().unwrap());
    assert!(enroll(&app, c1, FP).await.is_err());
    assert!(enroll(&app, c2, FP).await.is_ok());
}
