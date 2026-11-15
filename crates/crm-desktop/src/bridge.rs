//! Связь страницы CRM в окне программы с самой программой.
//!
//! Кнопка на странице переходит по адресу `http://crm-app.invalid/ДЕЙСТВИЕ`: программа
//! перехватывает переход, отменяет его и выполняет действие. Скачивания со страницы
//! сохраняются в «Загрузки» и показываются в проводнике — само окно WebView2 этого не делает.

use std::path::{Path, PathBuf};

use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager, Url};

use crate::selftest;

/// Адрес «вызова программы» со страницы (домен .invalid не существует в сети).
pub const APP_HOST: &str = "crm-app.invalid";

/// Переход на `APP_HOST` — это команда, а не страница. `true` — переход перехвачен.
pub fn intercept(app: &AppHandle, url: &Url) -> bool {
    if url.host_str() != Some(APP_HOST) {
        return false;
    }
    match url.path().trim_matches('/') {
        "ping" => selftest::log("BRIDGE ping"),
        "vpn-apply" => crate::on_menu(app, "vpn-apply"),
        other => selftest::log(&format!("BRIDGE unknown {other}")),
    }
    true
}

pub fn on_download(app: &AppHandle, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { destination, .. } => {
            if let Ok(dir) = app.path().download_dir() {
                let name = destination
                    .file_name()
                    .map_or_else(|| "файл CRM".into(), |n| n.to_os_string());
                *destination = free_name(&dir.join(name));
            }
            true
        }
        DownloadEvent::Finished { path, success, .. } => {
            let shown = path.as_deref().map(Path::display);
            selftest::log(&format!(
                "DOWNLOAD done ok={success} path={}",
                shown.map(|p| p.to_string()).unwrap_or_default()
            ));
            if success && !selftest::enabled() {
                if let Some(p) = path {
                    reveal(&p);
                }
            }
            true
        }
        _ => true,
    }
}

/// `конфиг.conf` → `конфиг (2).conf`, если такой файл уже есть: ничего не затираем.
fn free_name(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

/// Показать скачанный файл в проводнике.
fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_name_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("crm-free-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("ivan.conf");
        assert_eq!(free_name(&first), first);
        std::fs::write(&first, "1").unwrap();
        assert_eq!(free_name(&first), dir.join("ivan (2).conf"));
        std::fs::write(dir.join("ivan (2).conf"), "2").unwrap();
        assert_eq!(free_name(&first), dir.join("ivan (3).conf"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
