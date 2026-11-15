//! Самопроверка на настоящем Windows (CI): программа открывает CRM в своём окне,
//! записывает, что отрисовалось, затем останавливает сервер тем же путём, что и кнопка.
//!
//! Включается переменной `CRM_SELFTEST_OUT=путь\к\отчёту.txt`; в обычной работе выключена.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Url};

/// Адрес, по которому скрипт страницы «сообщает» результат: переход перехватывается и отменяется.
pub const REPORT_HOST: &str = "selftest.invalid";

pub fn out() -> Option<PathBuf> {
    std::env::var_os("CRM_SELFTEST_OUT").map(PathBuf::from)
}

pub fn enabled() -> bool {
    out().is_some()
}

pub fn log(line: &str) {
    let Some(p) = out() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Переход окна: отчёт страницы перехватываем, остальное записываем и пропускаем.
pub fn on_navigation(app: &AppHandle, url: &Url) -> bool {
    if url.host_str() == Some(REPORT_HOST) {
        let report = url
            .query_pairs()
            .find(|(k, _)| k == "d")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        log(&format!("PAGE {report}"));
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // Пункты трея, пока в окне открыта CRM: должны вернуть окно на панель.
            log("TRAY stop");
            crate::on_menu(&app, "stop");
            tokio::time::sleep(Duration::from_secs(3)).await;
            log("TRAY panel");
            crate::on_menu(&app, "panel");
            tokio::time::sleep(Duration::from_secs(3)).await;
            stop_server(app).await;
        });
        return false;
    }
    log(&format!("NAVIGATE {url}"));
    true
}

async fn stop_server(app: AppHandle) {
    let result = crate::server_action(app.clone(), "stop".into()).await;
    match result {
        Ok(st) => log(&format!(
            "STOP ok service={:?} text={}",
            st.service, st.text
        )),
        Err(e) => log(&format!("STOP error {e}")),
    }
    log("DONE");
    app.exit(0);
}

/// Дождаться сервера и открыть CRM в окне — как кнопка «Открыть CRM».
pub async fn run(app: AppHandle) {
    log("START");
    for _ in 0..60 {
        if matches!(
            crate::server_ctl::health().await,
            crate::status::Health::Ok { .. }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    match crate::connections::open_own(app.clone()).await {
        Ok(()) => log("OPEN ok"),
        Err(e) => {
            log(&format!("OPEN error {e}"));
            stop_server(app).await;
            return;
        }
    }
    // Страница не ответила совсем — отчёта не будет; не висим вечно.
    tokio::time::sleep(Duration::from_secs(40)).await;
    log("TIMEOUT страница не прислала отчёт");
    stop_server(app).await;
}
