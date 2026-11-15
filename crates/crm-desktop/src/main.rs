//! CRM в трее (ТЗ §2.5.1): иконка с цветом состояния сервера, меню, окно,
//! которое при закрытии прячется в трей. Сервер работает отдельно — службой.

// В релизной сборке без окна консоли.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod connections;
mod selftest;
mod server_ctl;
mod status;
mod updates;

use std::sync::Mutex;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use status::{Light, Status};

const TRAY_ID: &str = "main";
const POLL_EVERY: Duration = Duration::from_secs(5);

/// Строка состояния в меню трея — меняется при каждом опросе.
struct StatusLine(MenuItem<Wry>);
/// Последнее известное состояние — чтобы не дёргать трей без изменений.
struct LastStatus(Mutex<Option<Status>>);

fn tray_image(light: Light) -> Option<Image<'static>> {
    let bytes: &'static [u8] = match light {
        Light::Ok => include_bytes!("../icons/tray-ok.png"),
        Light::Warn => include_bytes!("../icons/tray-warn.png"),
        Light::Error => include_bytes!("../icons/tray-error.png"),
        Light::Offline => include_bytes!("../icons/tray-offline.png"),
    };
    Image::from_bytes(bytes).ok()
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Пункты трея. Окно может показывать саму CRM (чужую страницу) — поэтому всё, что
/// требует ответа на экране, сначала возвращает окно на панель программы.
pub fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "panel" => connections::to_panel(app, None),
        "open" => {
            // У владельца — CRM этого компьютера, у сотрудника — окно как есть.
            if server_ctl::service_state() == status::Service::NotInstalled {
                show_main(app);
            } else {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = connections::open_own(app.clone()).await {
                        connections::to_panel(&app, Some(("toast", &e)));
                    }
                });
            }
        }
        "servers" => connections::back_to_list(app),
        "update" => {
            if let Err(e) = server_ctl::open_browser(updates::RELEASES) {
                eprintln!("CRM: {e}");
            }
        }
        "browser" => {
            let url = format!("https://localhost:{}", server_ctl::PORT);
            if let Err(e) = server_ctl::open_browser(&url) {
                eprintln!("CRM: {e}");
            }
        }
        "vpn-apply" | "restart" => {
            let (app, action) = (app.clone(), id.to_owned());
            connections::to_panel(
                &app,
                Some(("toast", "Выполняю — подтвердите запрос Windows…")),
            );
            tauri::async_runtime::spawn(async move {
                let text = match server_action(app.clone(), action.clone()).await {
                    Ok(_) if action == "restart" => "Сервер перезапущен".to_owned(),
                    Ok(_) => "VPN включён. Не забудьте перенаправить порт на роутере.".to_owned(),
                    Err(e) => e,
                };
                connections::to_panel(&app, Some(("toast", &text)));
            });
        }
        // Остановка затронет всех сотрудников — подтверждение на панели.
        "stop" => connections::to_panel(app, Some(("confirm", "stop"))),
        "reset-owner" => {
            std::thread::spawn(|| {
                if let Err(e) = server_ctl::reset_owner_password_console() {
                    eprintln!("CRM: {e}");
                }
            });
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

async fn current_status() -> Status {
    let service = tauri::async_runtime::spawn_blocking(server_ctl::service_state)
        .await
        .unwrap_or(status::Service::Unknown);
    let health = server_ctl::health().await;
    status::summarize(service, &health)
}

/// Обновить трей и окно, если состояние изменилось.
fn apply(app: &AppHandle, st: Status) {
    let last = app.state::<LastStatus>();
    let Ok(mut guard) = last.0.lock() else {
        return;
    };
    if guard.as_ref() == Some(&st) {
        return;
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(tray_image(st.light));
        let _ = tray.set_tooltip(Some(format!("CRM · {}", st.text)));
    }
    let _ = app.state::<StatusLine>().0.set_text(&st.text);
    let _ = app.emit("status", &st);
    *guard = Some(st);
}

async fn refresh(app: &AppHandle) -> Status {
    let st = current_status().await;
    apply(app, st.clone());
    st
}

// ---------- Команды для окна ----------

#[tauri::command]
async fn get_status(app: AppHandle) -> Status {
    refresh(&app).await
}

#[tauri::command]
fn addresses() -> Vec<String> {
    server_ctl::lan_addresses()
}

#[tauri::command]
async fn server_action(app: AppHandle, action: String) -> Result<Status, String> {
    if !server_ctl::is_allowed_action(&action) {
        return Err("неизвестное действие".into());
    }
    tauri::async_runtime::spawn_blocking(move || server_ctl::elevated(&action))
        .await
        .map_err(|e| e.to_string())??;
    // Даём серверу подняться и отвечаем свежим состоянием.
    for _ in 0..20 {
        let st = refresh(&app).await;
        if st.reachable || st.service == status::Service::Stopped {
            return Ok(st);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(refresh(&app).await)
}

#[derive(serde::Serialize)]
struct Autostart {
    /// Служба сервера стартует с Windows (`None` — сервера на этом компьютере нет).
    server: Option<bool>,
    /// Значок CRM появляется в трее при входе в Windows.
    app: bool,
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> Autostart {
    Autostart {
        server: server_ctl::service_autostart(),
        app: app.autolaunch().is_enabled().unwrap_or(false),
    }
}

#[tauri::command]
fn set_app_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    let a = app.autolaunch();
    if on { a.enable() } else { a.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_server_autostart(on: bool) -> Result<(), String> {
    let action = if on { "autostart-on" } else { "autostart-off" };
    tauri::async_runtime::spawn_blocking(move || server_ctl::elevated(action))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn create_owner(
    app: AppHandle,
    username: String,
    display_name: String,
    password: String,
) -> Result<Status, String> {
    server_ctl::create_owner(username, display_name, password).await?;
    Ok(refresh(&app).await)
}

// ---------- Трей ----------

fn build_tray(app: &tauri::App) -> tauri::Result<TrayIcon> {
    let panel = MenuItem::with_id(app, "panel", "Панель управления", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Открыть CRM", true, None::<&str>)?;
    let servers = MenuItem::with_id(app, "servers", "Мои серверы", true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Открыть CRM в браузере", true, None::<&str>)?;
    let vpn = MenuItem::with_id(
        app,
        "vpn-apply",
        "Применить настройки VPN…",
        true,
        None::<&str>,
    )?;
    let line = MenuItem::with_id(app, "status", "Проверяю сервер…", false, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Обновления…", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "Перезапустить сервер", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Остановить сервер…", true, None::<&str>)?;
    let reset = MenuItem::with_id(
        app,
        "reset-owner",
        "Сбросить пароль владельца…",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        "Выйти из приложения (сервер продолжит работать)",
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &panel,
            &open,
            &servers,
            &browser,
            &PredefinedMenuItem::separator(app)?,
            &line,
            &PredefinedMenuItem::separator(app)?,
            &restart,
            &stop,
            &reset,
            &vpn,
            &PredefinedMenuItem::separator(app)?,
            &update,
            &quit,
        ],
    )?;
    app.manage(StatusLine(line));
    app.manage(updates::UpdateItem(update));

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("CRM")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(img) = tray_image(Light::Offline) {
        builder = builder.icon(img);
    }
    builder.build(app)
}

fn main() {
    let minimized = std::env::args().any(|a| a == "--minimized");
    let result = tauri::Builder::default()
        // Второй запуск не создаёт вторую иконку, а показывает окно первого.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(LastStatus(Mutex::new(None)))
        .manage(connections::Tunnels::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            addresses,
            server_action,
            create_owner,
            get_autostart,
            set_app_autostart,
            set_server_autostart,
            connections::servers,
            connections::key_needs_pin,
            connections::server_add,
            connections::server_remove,
            connections::server_rename,
            connections::server_open,
            connections::open_own
        ])
        .setup(move |app| {
            build_tray(app)?;
            // Запуск вместе с Windows — свёрнутым в трей (по умолчанию включено).
            // Значок в трее при входе в Windows включается один раз, при первом запуске;
            // дальше — как выбрал пользователь (переключатель на панели).
            let autostart = app.autolaunch();
            let marker = app
                .path()
                .app_data_dir()
                .map(|d| d.join("autostart-chosen"))
                .ok();
            if marker.as_ref().is_some_and(|m| !m.exists()) {
                let _ = autostart.enable();
                if let Some(m) = &marker {
                    let _ = m.parent().map(std::fs::create_dir_all);
                    let _ = std::fs::write(m, b"1");
                }
            }
            // Главное окно создаётся здесь, чтобы встроить в страницы CRM
            // кнопку «Мои серверы» и объяснение вместо белого экрана.
            if let Some(cfg) = app.config().app.windows.first().cloned() {
                let handle = app.handle().clone();
                tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
                    .initialization_script(connections::guard_script())
                    .on_navigation(move |url| {
                        !selftest::enabled() || selftest::on_navigation(&handle, url)
                    })
                    .on_page_load(|_, payload| {
                        if selftest::enabled() {
                            selftest::log(&format!("LOAD {:?} {}", payload.event(), payload.url()));
                        }
                    })
                    .build()?;
            }
            if !minimized {
                show_main(app.handle());
            }
            tauri::async_runtime::spawn(connections::keep_tunnels_fresh(app.handle().clone()));
            tauri::async_runtime::spawn(updates::watch(app.handle().clone()));
            if selftest::enabled() {
                tauri::async_runtime::spawn(selftest::run(app.handle().clone()));
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    refresh(&handle).await;
                    tokio::time::sleep(POLL_EVERY).await;
                }
            });
            Ok(())
        })
        // Крестик прячет окно в трей, а не закрывает приложение.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("CRM: не удалось запустить приложение: {e}");
        std::process::exit(1);
    }
}
