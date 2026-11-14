//! Проверка обновлений: раз в 6 часов спрашиваем GitHub о последнем выпуске.
//! Установка — тем же установщиком поверх (данные сохраняются), поэтому здесь
//! только подсказка в трее и ссылка на страницу выпуска.

use std::time::Duration;

use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager, Wry};

pub const RELEASES: &str = "https://github.com/kclazzy/crm/releases/latest";
const API: &str = "https://api.github.com/repos/kclazzy/crm/releases/latest";

/// Пункт трея «Обновления» — текст меняется, когда вышла новая версия.
pub struct UpdateItem(pub MenuItem<Wry>);

/// «v0.3.1» → (0, 3, 1). Хвост вроде «-beta» отбрасывается.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        it.next()??,
        it.next().flatten().unwrap_or(0),
        it.next().flatten().unwrap_or(0),
    ))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

async fn latest() -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("crm-desktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let v: serde_json::Value = client
        .get(API)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    v["tag_name"].as_str().map(str::to_owned)
}

pub async fn watch(app: AppHandle) {
    loop {
        if let Some(tag) = latest().await {
            if is_newer(&tag, env!("CARGO_PKG_VERSION")) {
                if let Some(item) = app.try_state::<UpdateItem>() {
                    let _ = item
                        .0
                        .set_text(format!("Доступно обновление {tag} — скачать"));
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0-beta", "0.9.0"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("nightly", "0.2.0"));
        assert_eq!(parse("v2"), Some((2, 0, 0)));
    }
}
