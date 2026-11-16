//! CRM для Android: «Мои серверы» и окно CRM через локальный туннель.
//!
//! Вся логика подключения — в `crm-connect` (тот же код, что в программе на
//! компьютере): проверка адресов, подключение по ключу сервера, туннель
//! `127.0.0.1:ПОРТ`. Здесь только команды для экрана и переход окна на CRM.

use std::collections::HashMap;

use crm_connect::probe::{self, AddrCheck, Probe, State as ProbeState};
use crm_connect::store::Added;
use crm_connect::tunnel::{Target, Tunnel};
use crm_connect::{Server, Store};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

/// Собственные страницы приложения (экран серверов).
#[cfg(target_os = "android")]
const APP_URL: &str = "http://tauri.localhost/index.html";
#[cfg(not(target_os = "android"))]
const APP_URL: &str = "tauri://localhost/index.html";

#[derive(Default)]
struct Tunnels(Mutex<HashMap<String, Tunnel>>);

fn store(app: &AppHandle) -> Result<Store, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    // Секрет устройства — чтобы сервер узнавал этот телефон после одобрения владельцем.
    let _ = crm_connect::node::use_device_key(&dir);
    Ok(Store::new(dir.join("servers.json")))
}

#[derive(Serialize)]
struct ServerView {
    #[serde(flatten)]
    server: Server,
    probe: Probe,
}

async fn view(server: Server) -> ServerView {
    let probe = probe::probe(&server).await;
    ServerView { server, probe }
}

#[tauri::command]
async fn servers(app: AppHandle) -> Result<Vec<ServerView>, String> {
    let list = store(&app)?.list().map_err(|e| e.to_string())?;
    let tasks: Vec<_> = list
        .into_iter()
        .map(|s| tauri::async_runtime::spawn(view(s)))
        .collect();
    let mut out = Vec::new();
    for t in tasks {
        if let Ok(v) = t.await {
            out.push(v);
        }
    }
    Ok(out)
}

/// Ключ, как его ввёл человек: из мессенджера он приходит с переносами строк и
/// пробелами, а клавиатура телефона может вставить их сама.
fn clean_key(input: &str) -> String {
    input.split_whitespace().collect()
}

#[tauri::command]
fn key_needs_pin(key: String) -> Result<bool, String> {
    crm_core::connkey::needs_pin(&clean_key(&key)).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct AddedView {
    id: String,
    name: String,
    invite: Option<String>,
}

#[tauri::command]
fn server_add(app: AppHandle, key: String, pin: Option<String>) -> Result<AddedView, String> {
    let Added { server, invite, .. } = store(&app)?
        .add_key(&clean_key(&key), pin.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(AddedView {
        id: server.id,
        name: server.name,
        invite,
    })
}

#[tauri::command]
async fn server_remove(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Tunnels>().0.lock().await.remove(&id);
    store(&app)?.remove(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn server_check(app: AppHandle, id: String) -> Result<Vec<AddrCheck>, String> {
    let server = store(&app)?.get(&id).map_err(|e| e.to_string())?;
    Ok(probe::diagnose(&server).await)
}

#[tauri::command]
fn server_add_addr(app: AppHandle, id: String, addr: String) -> Result<(), String> {
    store(&app)?
        .add_addr(&id, &addr)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn server_remove_addr(app: AppHandle, id: String, addr: String) -> Result<(), String> {
    store(&app)?
        .remove_addr(&id, &addr)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Найти путь к серверу, открыть туннель и вернуть адрес CRM для окна.
/// Повторный вызов (сеть сменилась: Wi-Fi ↔ мобильный интернет) перенаправляет
/// тот же туннель — вход в CRM при этом не теряется.
#[tauri::command]
async fn server_open(app: AppHandle, id: String, invite: Option<String>) -> Result<String, String> {
    let st = store(&app)?;
    let server = st.get(&id).map_err(|e| e.to_string())?;
    let p = probe::probe(&server).await;
    if p.state == ProbeState::WrongServer {
        return Err("По этому адресу отвечает другой сервер — подключение запрещено. Попросите у владельца новый ключ.".into());
    }
    let Some(target) = Target::of(&server, &p).await else {
        return Err("Сервер не отвечает. Проверьте интернет и нажмите «Проверить».".into());
    };
    if let Some(addr) = &p.addr {
        let _ = st.remember_addr(&id, addr);
        let _ = st.learn_public_addr(&id, p.public_addr.as_deref());
        let _ = st.learn_node(&id, p.node_id.as_deref(), p.node_udp);
    }
    let tunnels = app.state::<Tunnels>();
    let mut map = tunnels.0.lock().await;
    let url = match map.get(&id) {
        Some(t) => {
            t.retarget(target).await;
            t.url()
        }
        None => {
            let t = Tunnel::open(server.local_port, target)
                .await
                .map_err(|e| format!("не удалось открыть подключение: {e}"))?;
            let u = t.url();
            map.insert(id, t);
            u
        }
    };
    Ok(match invite {
        Some(token) => format!("{url}/invite?code={}", urlencode(&token)),
        None => format!("{url}/"),
    })
}

/// Перевести окно на страницу CRM (с экрана серверов уйти обычной ссылкой нельзя:
/// политика безопасности экрана запрещает переходы наружу).
#[tauri::command]
fn open_window(app: AppHandle, url: String) -> Result<(), String> {
    let ok = url.starts_with("http://127.0.0.1:");
    if !ok {
        return Err("окно открывает только CRM через своё подключение".into());
    }
    let w = app
        .get_webview_window("main")
        .ok_or_else(|| "окно не найдено".to_owned())?;
    let u: tauri::Url = url.parse().map_err(|e| format!("адрес {url}: {e}"))?;
    w.navigate(u).map_err(|e| e.to_string())
}

fn guard_script() -> String {
    include_str!("guard.js").replace("__APP_URL__", APP_URL)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used)] // без окна приложению нечего показать — завершаемся с понятной ошибкой
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder
        .plugin(tauri_plugin_barcode_scanner::init())
        .plugin(tauri_plugin_biometric::init());
    builder
        .manage(Tunnels::default())
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .initialization_script(guard_script())
            .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            servers,
            key_needs_pin,
            server_add,
            server_remove,
            server_check,
            server_add_addr,
            server_remove_addr,
            server_open,
            open_window,
        ])
        .run(tauri::generate_context!())
        .expect("не удалось запустить приложение CRM");
}

#[cfg(test)]
mod tests {
    use super::clean_key;

    #[test]
    fn key_pasted_with_line_breaks_and_spaces_is_accepted() {
        assert_eq!(
            clean_key("  CRM1-AHsi\nbmFt ZSI6\r\n Imlw \t"),
            "CRM1-AHsibmFtZSI6Imlw"
        );
        assert_eq!(clean_key("CRM1-abc"), "CRM1-abc");
    }
}
