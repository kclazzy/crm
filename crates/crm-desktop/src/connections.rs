//! «Мои серверы» в окне программы: добавление по ключу, статус, подключение
//! через туннель с закреплённым сертификатом (crm-connect).

use std::collections::HashMap;

use crm_connect::probe::State as ProbeState;
use crm_connect::store::Added;
use crm_connect::tunnel::{Target, Tunnel};
use crm_connect::{probe, Probe, Server, Store};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

/// Открытые туннели: сервер → туннель. Живут, пока работает программа.
#[derive(Default)]
pub struct Tunnels(pub Mutex<HashMap<String, Tunnel>>);

/// Сервер, открытый последним, — его и показывает «Открыть CRM в браузере».
#[derive(Default)]
pub struct LastOpened(pub std::sync::Mutex<Option<String>>);

/// Куда вести браузер: в CRM, открытую в программе (через её подключение — сертификат
/// ставить не нужно), иначе в сервер на этом компьютере. `localhost` не годится: браузер
/// пробует IPv6 (::1), а сервер слушает IPv4 — «соединение недоступно».
pub fn browser_target(
    tunnel_url: Option<String>,
    own: crate::status::Health,
    service: crate::status::Service,
) -> Result<String, &'static str> {
    use crate::status::{Health, Service};
    if let Some(u) = tunnel_url {
        return Ok(format!("{u}/"));
    }
    match (own, service) {
        (Health::Ok { .. }, _) => Ok(format!("https://127.0.0.1:{}/", crate::server_ctl::PORT)),
        (Health::Unreachable, Service::Stopped) => {
            Err("Сервер на этом компьютере остановлен — запустите его в панели управления")
        }
        (Health::Unreachable, Service::Running) => {
            Err("Сервер запускается — попробуйте через минуту")
        }
        _ => Err("Сначала откройте CRM: «Мои серверы» → «Подключиться»"),
    }
}

/// «Открыть CRM в браузере» из трея.
pub async fn open_in_browser(app: AppHandle) {
    let last = app
        .state::<LastOpened>()
        .0
        .lock()
        .ok()
        .and_then(|l| l.clone());
    let tunnel_url = match last {
        Some(id) => app
            .state::<Tunnels>()
            .0
            .lock()
            .await
            .get(&id)
            .map(Tunnel::url),
        None => None,
    };
    let own = if tunnel_url.is_some() {
        crate::status::Health::Unreachable
    } else {
        crate::server_ctl::health().await
    };
    match browser_target(tunnel_url, own, crate::server_ctl::service_state()) {
        Ok(url) => {
            if let Err(e) = crate::server_ctl::open_browser(&url) {
                to_panel(
                    &app,
                    Some(("toast", &format!("Не удалось открыть браузер: {e}"))),
                );
            }
        }
        Err(why) => to_panel(&app, Some(("toast", why))),
    }
}

#[cfg(test)]
mod browser_tests {
    use super::browser_target;
    use crate::status::{Health, Service};

    fn up() -> Health {
        Health::Ok {
            needs_owner: false,
            ca_fingerprint: None,
        }
    }

    #[test]
    fn browser_goes_where_crm_actually_is() {
        // Сотрудник: открыта CRM в программе — браузер идёт через то же подключение.
        let t = Some("http://127.0.0.1:51234".to_owned());
        assert_eq!(
            browser_target(t, Health::Unreachable, Service::NotInstalled).unwrap(),
            "http://127.0.0.1:51234/"
        );
        // Сервер на этом компьютере — по 127.0.0.1, не localhost.
        assert_eq!(
            browser_target(None, up(), Service::Running).unwrap(),
            "https://127.0.0.1:8443/"
        );
        // Нет ни подключения, ни своего сервера — понятная подсказка вместо «соединение недоступно».
        assert!(
            browser_target(None, Health::Unreachable, Service::NotInstalled)
                .unwrap_err()
                .contains("Мои серверы")
        );
        assert!(browser_target(None, Health::Unreachable, Service::Stopped)
            .unwrap_err()
            .contains("остановлен"));
    }
}

fn store(app: &AppHandle) -> Result<Store, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(Store::new(dir.join("servers.json")))
}

#[derive(Serialize)]
pub struct ServerView {
    #[serde(flatten)]
    server: Server,
    probe: Probe,
}

async fn view(s: Server) -> ServerView {
    let p = probe(&s).await;
    ServerView {
        server: s,
        probe: p,
    }
}

#[tauri::command]
pub async fn servers(app: AppHandle) -> Result<Vec<ServerView>, String> {
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

#[tauri::command]
pub fn key_needs_pin(key: String) -> Result<bool, String> {
    crm_core::connkey::needs_pin(&key).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct AddedView {
    #[serde(flatten)]
    view: ServerView,
    invite: Option<String>,
    updated: bool,
}

#[tauri::command]
pub async fn server_add(
    app: AppHandle,
    key: String,
    pin: Option<String>,
) -> Result<AddedView, String> {
    let Added {
        server,
        invite,
        updated,
    } = store(&app)?
        .add_key(&key, pin.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(AddedView {
        view: view(server).await,
        invite,
        updated,
    })
}

#[tauri::command]
pub async fn server_remove(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Tunnels>().0.lock().await.remove(&id);
    store(&app)?.remove(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn server_rename(app: AppHandle, id: String, name: String) -> Result<(), String> {
    store(&app)?.rename(&id, &name).map_err(|e| e.to_string())
}

/// «Проверить соединение»: каждый адрес сервера и почему он (не) отвечает.
#[tauri::command]
pub async fn server_check(
    app: AppHandle,
    id: String,
) -> Result<Vec<crm_connect::probe::AddrCheck>, String> {
    let server = store(&app)?.get(&id).map_err(|e| e.to_string())?;
    Ok(crm_connect::probe::diagnose(&server).await)
}

/// Сотрудник ввёл адрес сервера сам. Подлинность всё равно проверит сертификат.
#[tauri::command]
pub fn server_add_addr(app: AppHandle, id: String, addr: String) -> Result<(), String> {
    store(&app)?
        .add_addr(&id, &addr)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn server_remove_addr(app: AppHandle, id: String, addr: String) -> Result<(), String> {
    store(&app)?
        .remove_addr(&id, &addr)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Подключиться: найти отвечающий адрес, открыть туннель, показать CRM в окне.
/// `invite` — ключ был для нового сотрудника: сразу экран «Задайте пароль».
#[tauri::command]
pub async fn server_open(app: AppHandle, id: String, invite: Option<String>) -> Result<(), String> {
    let st = store(&app)?;
    let server = st.get(&id).map_err(|e| e.to_string())?;
    let p = probe(&server).await;
    if p.state == ProbeState::Online {
        let _ = st.learn_public_addr(&id, p.public_addr.as_deref());
        if p.addr.is_some() {
            let _ = st.learn_node(&id, p.node_id.as_deref(), p.node_udp);
        }
    }
    let target = Target::of(&server, &p).await;
    let target = match (p.state, target) {
        (ProbeState::Online, Some(t)) => t,
        (ProbeState::WrongServer, _) => {
            return Err(
                "Сервер отвечает с чужим сертификатом — подключение остановлено. \
                        Попросите у владельца новый ключ."
                    .into(),
            )
        }
        _ => {
            return Err(format!(
                "Сервер не отвечает — ниже проверка по каждому адресу.{}",
                p.error.map(|e| format!(" ({e})")).unwrap_or_default()
            ))
        }
    };
    if let Some(addr) = &p.addr {
        let _ = st.remember_addr(&id, addr);
    }
    let url = {
        let tunnels = app.state::<Tunnels>();
        let mut map = tunnels.0.lock().await;
        match map.get(&id) {
            Some(t) => {
                t.retarget(target).await;
                t.url()
            }
            None => {
                let t = Tunnel::open(server.local_port, target)
                    .await
                    .map_err(|e| format!("не удалось открыть подключение: {e}"))?;
                let u = t.url();
                map.insert(id.clone(), t);
                u
            }
        }
    };
    if let Ok(mut last) = app.state::<LastOpened>().0.lock() {
        *last = Some(id.clone());
    }
    let path = match invite {
        Some(token) => format!("/invite?code={}", urlencode(&token)),
        None => "/".into(),
    };
    navigate(&app, &format!("{url}{path}"))
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

pub fn navigate(app: &AppHandle, url: &str) -> Result<(), String> {
    let w = app
        .get_webview_window("main")
        .ok_or_else(|| "окно не найдено".to_owned())?;
    let u: tauri::Url = url.parse().map_err(|e| format!("адрес {url}: {e}"))?;
    w.navigate(u).map_err(|e| e.to_string())?;
    let _ = w.show();
    let _ = w.set_focus();
    Ok(())
}

/// Адрес собственных страниц программы: на Windows — http://tauri.localhost, иначе tauri://localhost.
pub fn app_index_url() -> &'static str {
    if cfg!(windows) {
        "http://tauri.localhost/index.html"
    } else {
        "tauri://localhost/index.html"
    }
}

/// Скрипт для страниц CRM в окне: «← Мои серверы» и объяснение вместо белого экрана.
pub fn guard_script() -> String {
    include_str!("crm_guard.js")
        .replace("__APP_URL__", app_index_url())
        .replace("__REPORT_HOST__", crate::selftest::REPORT_HOST)
        .replace("__APP_HOST__", crate::bridge::APP_HOST)
        .replace(
            "__SELFTEST__",
            if crate::selftest::enabled() {
                "true"
            } else {
                "false"
            },
        )
}

/// Вернуться на стартовую панель программы; `param` — что показать: подтверждение или сообщение.
pub fn to_panel(app: &AppHandle, param: Option<(&str, &str)>) {
    // Адрес панели фиксированный: запомненный при запуске был about:blank — окно
    // ещё не загрузилось, и переход туда давал белый экран.
    let Ok(mut u) = app_index_url().parse::<tauri::Url>() else {
        return;
    };
    if let Some((k, v)) = param {
        u.query_pairs_mut().append_pair(k, v);
    }
    let _ = navigate(app, u.as_str());
}

/// Вернуться из CRM к списку «Мои серверы».
pub fn back_to_list(app: &AppHandle) {
    to_panel(app, Some(("screen", "servers")));
}

/// Владелец на сервере: «Этот компьютер» в списке и сразу вход в CRM.
#[tauri::command]
pub async fn open_own(app: AppHandle) -> Result<(), String> {
    let fp = match crate::server_ctl::health().await {
        crate::status::Health::Ok {
            ca_fingerprint: Some(fp),
            ..
        } => fp,
        _ => return Err("сервер на этом компьютере не отвечает".into()),
    };
    let (server, _) = store(&app)?
        .add(Server {
            id: crm_core::connkey::normalize_fp(&fp),
            name: "Этот компьютер".into(),
            addrs: vec!["127.0.0.1".into()],
            port: crate::server_ctl::PORT,
            fp,
            node: None,
            udp: None,
            manual: Vec::new(),
            last_addr: None,
            local_port: 0,
            user: None,
        })
        .map_err(|e| e.to_string())?;
    server_open(app, server.id, None).await
}

/// Раз в 15 секунд: если офисный адрес пропал (ушли домой), туннель переключается на внешний адрес.
pub async fn keep_tunnels_fresh(app: AppHandle) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        let Ok(st) = store(&app) else { continue };
        let ids: Vec<String> = app
            .state::<Tunnels>()
            .0
            .lock()
            .await
            .keys()
            .cloned()
            .collect();
        for id in ids {
            let Ok(server) = st.get(&id) else { continue };
            let p = probe(&server).await;
            if p.state == ProbeState::Online {
                let _ = st.learn_public_addr(&id, p.public_addr.as_deref());
                if p.addr.is_some() {
                    let _ = st.learn_node(&id, p.node_id.as_deref(), p.node_udp);
                }
            }
            // Туннель всегда ведём по тому пути, который ответил сейчас
            // (офис → дом → ключ узла и обратно); повтор того же пути ничего не рвёт.
            if p.state == ProbeState::Online {
                if let Some(addr) = p
                    .addr
                    .as_ref()
                    .filter(|a| server.last_addr.as_ref() != Some(*a))
                {
                    let _ = st.remember_addr(&id, addr);
                }
                if let Some(target) = Target::of(&server, &p).await {
                    if let Some(t) = app.state::<Tunnels>().0.lock().await.get(&id) {
                        t.retarget(target).await;
                    }
                }
            }
        }
    }
}
