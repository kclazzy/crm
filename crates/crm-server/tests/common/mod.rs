//! Общие помощники интеграционных тестов: сервер на временной папке и запросы к нему.
#![allow(clippy::unwrap_used, dead_code)]

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use crm_server::{config::Config, db, http};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tower::ServiceExt;

pub const LOCAL: &str = "127.0.0.1";
pub const OFFICE_PC: &str = "192.168.1.20";

pub fn config_in(dir: &std::path::Path) -> Config {
    Config::from_toml(&format!(
        "listen = \"127.0.0.1:0\"\n[storage]\ndb_dir = {:?}\nattachments_dir = {:?}\nbackup_dirs = [{:?}]\n",
        dir.join("data"),
        dir.join("files"),
        dir.join("backups"),
    ))
    .unwrap()
}

pub struct App {
    pub pool: SqlitePool,
    router: axum::Router,
    _dir: tempfile::TempDir,
}

impl App {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_in(dir.path());
        let pool = db::open(&cfg.storage).await.unwrap();
        let state = http::AppState::new(pool.clone(), cfg, Some(dir.path().join("config.toml")));
        Self {
            router: http::router(state),
            pool,
            _dir: dir,
        }
    }

    /// Временная папка этого теста (там же config.toml и data/).
    pub fn dir(&self) -> &std::path::Path {
        self._dir.path()
    }

    pub async fn call(
        &self,
        method: &str,
        path: &str,
        ip: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let mut req = req.body(body).unwrap();
        let addr: SocketAddr = format!("{ip}:50000").parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));

        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }

    /// Запрос с произвольными заголовками; ответ целиком: статус, заголовки, тело.
    pub async fn raw(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut req = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let mut req = req.body(body).unwrap();
        let addr: SocketAddr = format!("{LOCAL}:50000").parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// Владелец создан с этого компьютера и вошёл; возвращает токен сессии.
    pub async fn owner(&self) -> String {
        let (s, _) = self
            .call("POST", "/api/setup", LOCAL, None, Some(json!({
                "username": "owner", "display_name": "Владелец", "password": "очень-надёжный-пароль"
            })))
            .await;
        assert_eq!(s, StatusCode::CREATED);
        self.login("owner", "очень-надёжный-пароль", LOCAL).await
    }

    pub async fn try_login(&self, username: &str, password: &str, ip: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            "/api/auth/login",
            ip,
            None,
            Some(json!({
                "username": username, "password": password,
                "device_id": format!("dev-{ip}"), "device_name": "Тестовый ПК"
            })),
        )
        .await
    }

    pub async fn login(&self, username: &str, password: &str, ip: &str) -> String {
        let (s, v) = self.try_login(username, password, ip).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["token"].as_str().unwrap().to_owned()
    }

    /// Владелец заводит сотрудника; возвращает (id, токен приглашения).
    pub async fn invite(&self, owner: &str, username: &str, role: &str) -> (String, String) {
        let (s, v) = self
            .call(
                "POST",
                "/api/admin/users",
                LOCAL,
                Some(owner),
                Some(json!({
                    "username": username, "display_name": "Сотрудник", "role": role
                })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        (
            v["user"]["id"].as_str().unwrap().to_owned(),
            v["invite_token"].as_str().unwrap().to_owned(),
        )
    }

    pub async fn accept(&self, invite: &str, password: &str, ip: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            "/api/invites/accept",
            ip,
            None,
            Some(json!({
                "token": invite, "password": password,
                "device_id": format!("dev-{ip}"), "device_name": "Ноутбук сотрудника"
            })),
        )
        .await
    }
}
