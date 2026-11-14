//! Защита от перегрузки (ТЗ §3.2.1): лимит запросов с одного IP («ведро токенов»),
//! отдельный строгий лимит на вход и восстановление, учёт заблокированных адресов.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::http::AppState;

/// Вход, приглашения, восстановление: не больше 10 попыток в минуту с одного IP.
const AUTH: Limit = Limit {
    burst: 10.0,
    per_sec: 10.0 / 60.0,
};
/// Остальное: до 60 запросов разом и 30 в секунду — с запасом для обычной работы.
const API: Limit = Limit {
    burst: 60.0,
    per_sec: 30.0,
};
/// Больше адресов не держим в памяти — старые «вёдра» выбрасываются.
const MAX_TRACKED: usize = 10_000;

#[derive(Clone, Copy)]
struct Limit {
    burst: f64,
    per_sec: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Class {
    Auth,
    Api,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

#[derive(Clone, Serialize)]
pub struct Blocked {
    pub ip: String,
    pub count: u64,
    /// Сколько секунд назад был последний отказ.
    pub seconds_ago: u64,
}

#[derive(Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<(IpAddr, Class), Bucket>>,
    blocked: Mutex<HashMap<IpAddr, (u64, Instant)>>,
}

impl RateLimiter {
    /// Взять токен; `Err` — сколько ждать до следующего.
    fn take(&self, ip: IpAddr, class: Class) -> Result<(), Duration> {
        let limit = match class {
            Class::Auth => AUTH,
            Class::Api => API,
        };
        let now = Instant::now();
        let Ok(mut map) = self.buckets.lock() else {
            return Ok(()); // при сбое блокировки не мешаем работе
        };
        if map.len() > MAX_TRACKED {
            map.retain(|_, b| now.duration_since(b.at) < Duration::from_secs(600));
        }
        let b = map.entry((ip, class)).or_insert(Bucket {
            tokens: limit.burst,
            at: now,
        });
        b.tokens =
            (b.tokens + now.duration_since(b.at).as_secs_f64() * limit.per_sec).min(limit.burst);
        b.at = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - b.tokens) / limit.per_sec))
        }
    }

    fn note_blocked(&self, ip: IpAddr) {
        if let Ok(mut m) = self.blocked.lock() {
            let e = m.entry(ip).or_insert((0, Instant::now()));
            e.0 += 1;
            if e.0 == 1 || e.1.elapsed() > Duration::from_secs(60) {
                tracing::warn!(%ip, "слишком много запросов — адрес временно ограничен");
            }
            e.1 = Instant::now();
        }
    }

    /// Адреса, получавшие отказ за последние сутки.
    pub fn blocked(&self) -> Vec<Blocked> {
        let Ok(mut m) = self.blocked.lock() else {
            return Vec::new();
        };
        m.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(86_400));
        let mut v: Vec<Blocked> = m
            .iter()
            .map(|(ip, (count, at))| Blocked {
                ip: ip.to_string(),
                count: *count,
                seconds_ago: at.elapsed().as_secs(),
            })
            .collect();
        v.sort_by_key(|b| b.seconds_ago);
        v
    }
}

fn class_of(path: &str) -> Class {
    let strict =
        path.starts_with("/api/auth/") || path == "/api/invites/accept" || path == "/api/setup";
    if strict {
        Class::Auth
    } else {
        Class::Api
    }
}

pub async fn rate_limit(State(app): State<AppState>, req: Request, next: Next) -> Response {
    let Some(ip) = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
    else {
        return next.run(req).await;
    };
    // Опрос состояния из трея на самом сервере не ограничиваем.
    let class = class_of(req.uri().path());
    if ip.is_loopback() && class == Class::Api {
        return next.run(req).await;
    }
    match app.rt.limiter.take(ip, class) {
        Ok(()) => next.run(req).await,
        Err(wait) => {
            app.rt.limiter.note_blocked(ip);
            let mut res = (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(
                    serde_json::json!({ "error": "слишком много запросов, подождите немного" }),
                ),
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&wait.as_secs().max(1).to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
            res
        }
    }
}
