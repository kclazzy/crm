//! Вход в систему глазами пользователя: владелец, приглашения, сессии, защита от перебора.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::{App, LOCAL, OFFICE_PC};
use serde_json::json;

const EMP_PASSWORD: &str = "пароль-сотрудника-1";

#[tokio::test]
async fn owner_can_be_created_only_once_and_only_on_the_host_itself() {
    let app = App::new().await;
    let body = json!({ "username": "owner", "display_name": "Владелец", "password": "очень-надёжный-пароль" });

    let (s, _) = app
        .call("POST", "/api/setup", OFFICE_PC, None, Some(body.clone()))
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "с другого компьютера сервер не захватить"
    );

    let (s, _) = app
        .call("POST", "/api/setup", LOCAL, None, Some(body.clone()))
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = app
        .call("POST", "/api/setup", LOCAL, None, Some(body))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "второй раз — нельзя");
}

#[tokio::test]
async fn setup_rejects_weak_password() {
    let app = App::new().await;
    let (s, v) = app
        .call(
            "POST",
            "/api/setup",
            LOCAL,
            None,
            Some(json!({
                "username": "owner", "display_name": "Владелец", "password": "123"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("пароль"));
}

#[tokio::test]
async fn logged_in_user_sees_himself_others_get_401() {
    let app = App::new().await;
    let owner = app.owner().await;

    let (s, me) = app.call("GET", "/api/me", LOCAL, Some(&owner), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["username"], "owner");
    assert_eq!(me["role"], "host");

    assert_eq!(
        app.call("GET", "/api/me", LOCAL, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.call("GET", "/api/me", LOCAL, Some("подделка"), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn wrong_password_and_unknown_login_answer_identically() {
    let app = App::new().await;
    app.owner().await;
    let wrong_pw = app
        .try_login("owner", "неверный-пароль-123", OFFICE_PC)
        .await;
    let no_user = app
        .try_login("nobody", "неверный-пароль-123", OFFICE_PC)
        .await;
    assert_eq!(wrong_pw.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong_pw, no_user,
        "по ответу нельзя узнать, существует ли логин"
    );
}

#[tokio::test]
async fn five_failures_lock_this_login_from_this_ip_only() {
    let app = App::new().await;
    app.owner().await;
    for _ in 0..5 {
        assert_eq!(
            app.try_login("owner", "подбираю-пароль", OFFICE_PC).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (s, v) = app
        .try_login("owner", "очень-надёжный-пароль", OFFICE_PC)
        .await;
    assert_eq!(
        s,
        StatusCode::TOO_MANY_REQUESTS,
        "даже верный пароль не пускает: {v}"
    );

    assert_eq!(
        app.try_login("owner", "очень-надёжный-пароль", LOCAL)
            .await
            .0,
        StatusCode::OK,
        "с другого IP — можно"
    );

    let failures: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'login.failed'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(failures, 5, "попытки видны в журнале безопасности");
}

#[tokio::test]
async fn employee_joins_by_one_time_invite_and_sets_own_password() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;

    assert_eq!(
        app.try_login("ivan", EMP_PASSWORD, OFFICE_PC).await.0,
        StatusCode::UNAUTHORIZED,
        "пароля ещё нет"
    );
    assert_eq!(
        app.accept(&invite, "короткий", OFFICE_PC).await.0,
        StatusCode::BAD_REQUEST
    );

    let (s, v) = app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, me) = app
        .call("GET", "/api/me", OFFICE_PC, v["token"].as_str(), None)
        .await;
    assert_eq!(me["username"], "ivan");
    assert_eq!(me["role"], "employee");

    assert_eq!(
        app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await.0,
        StatusCode::UNAUTHORIZED,
        "приглашение одноразовое"
    );
    app.login("ivan", EMP_PASSWORD, OFFICE_PC).await;
}

#[tokio::test]
async fn expired_invite_does_not_work() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    sqlx::query("UPDATE invites SET expires_at = 0")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn only_owner_manages_users_and_there_is_one_owner() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "maria", "manager").await;
    let (_, v) = app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await;
    let manager = v["token"].as_str().unwrap().to_owned();

    let new_user = json!({ "username": "petr", "display_name": "Пётр", "role": "employee" });
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/users",
            OFFICE_PC,
            Some(&manager),
            Some(new_user),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(
        app.call(
            "GET",
            "/api/admin/sessions",
            OFFICE_PC,
            Some(&manager),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let second_owner = json!({ "username": "boss2", "display_name": "Второй", "role": "host" });
    let (s, _) = app
        .call(
            "POST",
            "/api/admin/users",
            LOCAL,
            Some(&owner),
            Some(second_owner),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let dup = json!({ "username": "MARIA", "display_name": "Дубль", "role": "employee" });
    assert_eq!(
        app.call("POST", "/api/admin/users", LOCAL, Some(&owner), Some(dup))
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn logout_and_owner_revocation_end_sessions_immediately() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    let (_, v) = app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await;
    let ivan = v["token"].as_str().unwrap().to_owned();

    let (s, sessions) = app
        .call("GET", "/api/admin/sessions", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let list = sessions.as_array().unwrap();
    let ivans = list.iter().find(|x| x["username"] == "ivan").unwrap();
    assert_eq!(ivans["device_name"], "Ноутбук сотрудника");
    assert_eq!(ivans["ip"], OFFICE_PC);

    let path = format!("/api/admin/sessions/{}", ivans["id"].as_str().unwrap());
    assert_eq!(
        app.call("DELETE", &path, LOCAL, Some(&owner), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.call("GET", "/api/me", OFFICE_PC, Some(&ivan), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        app.call("POST", "/api/auth/logout", LOCAL, Some(&owner), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.call("GET", "/api/me", LOCAL, Some(&owner), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn expired_session_and_deactivated_user_are_rejected() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;
    let (_, v) = app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await;
    let ivan = v["token"].as_str().unwrap().to_owned();

    sqlx::query("UPDATE users SET is_active = 0 WHERE username = 'ivan'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.call("GET", "/api/me", OFFICE_PC, Some(&ivan), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.try_login("ivan", EMP_PASSWORD, OFFICE_PC).await.0,
        StatusCode::UNAUTHORIZED
    );

    sqlx::query("UPDATE sessions SET expires_at = 0")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.call("GET", "/api/me", LOCAL, Some(&owner), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn reset_access_revokes_sessions_and_old_password() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (id, invite) = app.invite(&owner, "ivan", "employee").await;
    let (_, v) = app.accept(&invite, EMP_PASSWORD, OFFICE_PC).await;
    let ivan = v["token"].as_str().unwrap().to_owned();

    let (s, v) = app
        .call(
            "POST",
            &format!("/api/admin/users/{id}/reset"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let new_invite = v["invite_token"].as_str().unwrap().to_owned();

    assert_eq!(
        app.call("GET", "/api/me", OFFICE_PC, Some(&ivan), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.try_login("ivan", EMP_PASSWORD, OFFICE_PC).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.accept(&new_invite, "новый-пароль-ивана", OFFICE_PC)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn secrets_are_never_stored_in_plain_text() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, invite) = app.invite(&owner, "ivan", "employee").await;

    let leaked: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM sessions WHERE token_hash = ?1)
              + (SELECT count(*) FROM invites WHERE token_hash = ?2)
              + (SELECT count(*) FROM users WHERE password_hash LIKE '%надёжный%')",
    )
    .bind(&owner)
    .bind(&invite)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0);
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE username = 'owner'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(hash.starts_with("$argon2id$"));
}
