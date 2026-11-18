//! Служба Windows «CRM Server» (ТЗ §2.1.1): установка, удаление, управление и
//! работа под диспетчером служб — без окон и консоли.

use std::ffi::{OsStr, OsString};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::cli::ServiceCmd;
use crate::config::{self, Config};
use crate::server;

pub const SERVICE_NAME: &str = "CRMServer";
const DISPLAY_NAME: &str = "CRM Server";
const DESCRIPTION: &str = "Сервер CRM: клиенты, задачи, чаты. Данные: C:\\ProgramData\\CRM";
/// Виртуальная учётная запись службы — без прав администратора.
const ACCOUNT: &str = "NT SERVICE\\CRMServer";
const FIREWALL_RULE: &str = "CRM Server";
/// Не показывать окно консоли у вспомогательных программ (netsh, icacls).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("нужны права администратора: запустите от имени администратора")]
    AccessDenied,
    #[error("служба не установлена")]
    NotInstalled,
    #[error("{0}")]
    Refused(String),
    #[error("ошибка Windows: {0}")]
    Windows(windows_service::Error),
    #[error("не удалось выполнить {program}: {detail}")]
    Tool { program: String, detail: String },
    #[error("ошибка файла {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error("служба не перешла в состояние «{0}» за 30 секунд")]
    Timeout(&'static str),
    #[error(
        "служба остановилась при запуске (код {0}); подробности — в C:\\ProgramData\\CRM\\logs"
    )]
    StartFailed(String),
}

impl From<windows_service::Error> for ServiceError {
    fn from(e: windows_service::Error) -> Self {
        const ERROR_ACCESS_DENIED: i32 = 5;
        const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
        match &e {
            windows_service::Error::Winapi(io) => match io.raw_os_error() {
                Some(ERROR_ACCESS_DENIED) => Self::AccessDenied,
                Some(ERROR_SERVICE_DOES_NOT_EXIST) => Self::NotInstalled,
                _ => Self::Windows(e),
            },
            _ => Self::Windows(e),
        }
    }
}

/// Выполнить команду `crm-server service …`. Возвращает текст для пользователя.
pub fn handle(cmd: ServiceCmd) -> Result<String, ServiceError> {
    match cmd {
        ServiceCmd::Install => install(),
        ServiceCmd::InstallFresh => install_fresh(),
        ServiceCmd::Uninstall => uninstall(),
        ServiceCmd::Start => start().map(|()| "Служба запущена.".into()),
        ServiceCmd::Stop => stop().map(|()| "Служба остановлена.".into()),
        ServiceCmd::Restart => {
            stop()?;
            start()?;
            Ok("Служба перезапущена.".into())
        }
        ServiceCmd::Status => status(),
        ServiceCmd::Run => {
            service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
            Ok(String::new())
        }
        ServiceCmd::Park => park(),
        ServiceCmd::Unpark => unpark(),
        ServiceCmd::Autostart(on) => {
            open(ServiceAccess::QUERY_STATUS)?;
            run_tool(
                "sc",
                [
                    "config",
                    SERVICE_NAME,
                    "start=",
                    if on { "auto" } else { "demand" },
                ],
            )?;
            Ok(if on {
                "Сервер будет запускаться вместе с Windows.".into()
            } else {
                "Сервер больше не запускается сам — запускайте его из программы CRM.".into()
            })
        }
    }
}

// ---------- Старый VPN ----------

/// До 0.2.5 CRM могла включить свой туннель WireGuard «crm-vpn» и открыть для него порты.
/// Теперь доступ снаружи — через интернет, поэтому туннель и правила убираем.
/// Саму программу WireGuard не трогаем: ею может пользоваться VPN компании.
fn remove_old_vpn() {
    let base = std::env::var_os("ProgramFiles")
        .map_or_else(|| PathBuf::from("C:\\Program Files"), PathBuf::from);
    let wg = base.join("WireGuard").join("wireguard.exe");
    if wg.exists() {
        let _ = run_tool(
            &wg.display().to_string(),
            ["/uninstalltunnelservice", "crm-vpn"],
        );
    }
    for rule in ["CRM VPN", "CRM Server (VPN)"] {
        let _ = run_tool(
            "netsh",
            [
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                format!("name={rule}").as_str(),
            ],
        );
    }
}

// ---------- Установка ----------

fn install() -> Result<String, ServiceError> {
    let root = config::default_data_root();
    std::fs::create_dir_all(&root).map_err(|source| ServiceError::Io {
        path: root.clone(),
        source,
    })?;
    let cfg = Config::load(&config::default_config_path())?;
    let exe = std::env::current_exe().map_err(|source| ServiceError::Io {
        path: PathBuf::from("crm-server.exe"),
        source,
    })?;

    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.clone(),
        launch_arguments: vec!["service".into(), "run".into()],
        dependencies: vec![],
        account_name: Some(ACCOUNT.into()),
        account_password: None,
    };
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::QUERY_STATUS;
    // Повторная установка (обновление) — меняем настройки существующей службы.
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(s) => {
            s.change_config(&info)?;
            s
        }
        Err(_) => manager.create_service(&info, access)?,
    };
    service.set_description(DESCRIPTION)?;
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            };
            3
        ]),
    })?;
    service.set_failure_actions_on_non_crash_failures(true)?;

    // Учётная запись службы появляется вместе со службой — только теперь можно дать ей права.
    // Только на корень: вложенные файлы получают права по наследованию. С /T при повторной
    // установке icacls переписывал права уже созданных файлов БД, и служба теряла к ним доступ.
    run_tool(
        "icacls",
        [
            root.as_os_str(),
            OsStr::new("/inheritance:r"),
            OsStr::new("/grant:r"),
            OsStr::new("*S-1-5-18:(OI)(CI)F"),
            OsStr::new("*S-1-5-32-544:(OI)(CI)F"),
            OsStr::new("NT SERVICE\\CRMServer:(OI)(CI)M"),
            OsStr::new("/Q"),
        ],
    )?;
    firewall_allow(&exe, cfg.listen.port())?;

    start()?;
    Ok(format!(
        "Служба «{DISPLAY_NAME}» установлена и запущена. Порт {}. Данные: {}",
        cfg.listen.port(),
        root.display()
    ))
}

/// Новый сервер на компьютере, где остались данные прежнего: база и вложения
/// откладываются в `old-<дата>`, служба ставится с чистой базой.
fn install_fresh() -> Result<String, ServiceError> {
    match open(ServiceAccess::QUERY_STATUS) {
        Err(ServiceError::NotInstalled) => {}
        Ok(_) => {
            return Err(ServiceError::Refused(
                "сервер уже установлен — чистую базу можно поставить только вместо удалённого"
                    .into(),
            ))
        }
        Err(e) => return Err(e),
    }
    let cfg = Config::load(&config::default_config_path())?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let archive =
        crate::install_plan::archive_old_data(&cfg.storage, &stamp).map_err(|source| {
            ServiceError::Io {
                path: cfg.storage.db_dir.clone(),
                source,
            }
        })?;
    let done = install()?;
    Ok(match archive {
        Some(a) => format!("{done}\nПрежние данные сохранены в {}", a.display()),
        None => done,
    })
}

fn uninstall() -> Result<String, ServiceError> {
    match stop() {
        Ok(()) | Err(ServiceError::NotInstalled) => {}
        Err(e) => return Err(e),
    }
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    match manager.open_service(SERVICE_NAME, ServiceAccess::DELETE) {
        Ok(s) => s.delete()?,
        Err(e) => match ServiceError::from(e) {
            ServiceError::NotInstalled => {}
            other => return Err(other),
        },
    }
    // Правила может и не быть — это не ошибка.
    let _ = run_tool(
        "netsh",
        [
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            format!("name={FIREWALL_RULE}").as_str(),
        ],
    );
    Ok(format!(
        "Служба удалена. Данные сохранены в {}",
        config::default_data_root().display()
    ))
}

// ---------- Обновление программы ----------
//
// Установщик при обновлении может сначала запустить старый деинсталлятор как обычное
// удаление (вариант «Удалить перед установкой» выбран по умолчанию). Отличить это от
// настоящего удаления деинсталлятор не может, поэтому служба не забывается: рядом с данными
// остаётся отметка, и следующая установка возвращает службу с теми же данными и автозапуском.

/// Отметка «служба была»: `auto` или `demand` — как она запускалась.
fn parked_path() -> PathBuf {
    config::default_data_root().join("service.parked")
}

fn park() -> Result<String, ServiceError> {
    let auto = match open(ServiceAccess::QUERY_CONFIG) {
        Ok(s) => s.query_config()?.start_type == ServiceStartType::AutoStart,
        Err(ServiceError::NotInstalled) => return Ok("Службы нет — запоминать нечего.".into()),
        Err(e) => return Err(e),
    };
    let path = parked_path();
    std::fs::write(&path, if auto { "auto" } else { "demand" })
        .map_err(|source| ServiceError::Io { path, source })?;
    uninstall()
}

fn unpark() -> Result<String, ServiceError> {
    remove_old_vpn();
    let path = parked_path();
    let marker = match std::fs::read_to_string(&path) {
        Ok(text) => {
            let age = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .unwrap_or(Duration::MAX);
            Some((text, age))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(ServiceError::Io { path, source }),
    };
    let Some(auto) = crate::install_plan::restore_service(
        marker.as_ref().map(|(text, age)| (text.as_str(), *age)),
    ) else {
        // Сервер удалили не ради обновления: роль компьютера выберет человек.
        let _ = std::fs::remove_file(&path);
        return Ok("Службу возвращать не нужно — роль компьютера выбирается в программе.".into());
    };
    let done = install()?;
    if !auto {
        handle(ServiceCmd::Autostart(false))?;
    }
    // Отметку убираем последней: если установка оборвётся, следующий запуск повторит её.
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(ServiceError::Io { path, source }),
    }
    Ok(done)
}

/// Разрешить входящие подключения к серверу только в частных и доменных сетях.
fn firewall_allow(exe: &Path, port: u16) -> Result<(), ServiceError> {
    let _ = run_tool(
        "netsh",
        [
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            format!("name={FIREWALL_RULE}").as_str(),
        ],
    );
    run_tool(
        "netsh",
        [
            "advfirewall".to_owned(),
            "firewall".into(),
            "add".into(),
            "rule".into(),
            format!("name={FIREWALL_RULE}"),
            "dir=in".into(),
            "action=allow".into(),
            "protocol=TCP".into(),
            format!("localport={port}"),
            format!("program={}", exe.display()),
            "profile=private,domain".into(),
            "enable=yes".into(),
        ],
    )
}

fn run_tool<I, S>(program: &str, args: I) -> Result<(), ServiceError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let out = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| ServiceError::Tool {
            program: program.into(),
            detail: e.to_string(),
        })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(ServiceError::Tool {
            program: program.into(),
            detail: String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        })
    }
}

// ---------- Управление ----------

fn open(access: ServiceAccess) -> Result<windows_service::service::Service, ServiceError> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    Ok(manager.open_service(SERVICE_NAME, access)?)
}

fn wait_for(
    service: &windows_service::service::Service,
    state: ServiceState,
    label: &'static str,
) -> Result<(), ServiceError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let started = Instant::now();
    while Instant::now() < deadline {
        let st = service.query_status()?;
        if st.current_state == state {
            return Ok(());
        }
        // Ждём запуска, а служба уже снова остановлена — значит, упала при старте.
        if state == ServiceState::Running
            && st.current_state == ServiceState::Stopped
            && started.elapsed() > Duration::from_secs(1)
        {
            return Err(ServiceError::StartFailed(format!("{:?}", st.exit_code)));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(ServiceError::Timeout(label))
}

fn start() -> Result<(), ServiceError> {
    let s = open(ServiceAccess::START | ServiceAccess::QUERY_STATUS)?;
    if s.query_status()?.current_state != ServiceState::Running {
        s.start::<&OsStr>(&[])?;
    }
    wait_for(&s, ServiceState::Running, "работает")
}

fn stop() -> Result<(), ServiceError> {
    let s = open(ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)?;
    if s.query_status()?.current_state != ServiceState::Stopped {
        s.stop()?;
    }
    wait_for(&s, ServiceState::Stopped, "остановлена")
}

fn status() -> Result<String, ServiceError> {
    let state = match open(ServiceAccess::QUERY_STATUS) {
        Ok(s) => s.query_status()?.current_state,
        Err(ServiceError::NotInstalled) => return Ok("не установлена".into()),
        Err(e) => return Err(e),
    };
    Ok(match state {
        ServiceState::Running => "работает",
        ServiceState::Stopped => "остановлена",
        ServiceState::StartPending => "запускается",
        ServiceState::StopPending => "останавливается",
        _ => "приостановлена",
    }
    .into())
}

// ---------- Работа под диспетчером служб ----------

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_as_service() {
        tracing::error!(error = %e, "служба завершилась с ошибкой");
    }
}

fn status_of(state: ServiceState, exit: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: if exit == 0 {
            ServiceExitCode::Win32(0)
        } else {
            ServiceExitCode::ServiceSpecific(exit)
        },
        checkpoint: 0,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    }
}

fn run_as_service() -> Result<(), Box<dyn std::error::Error>> {
    let _log = crate::logging::to_files(&config::default_data_root().join("logs"));
    let stop = Arc::new(Notify::new());
    let on_stop = stop.clone();
    let handle = service_control_handler::register(SERVICE_NAME, move |c| match c {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            on_stop.notify_one();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    handle.set_service_status(status_of(ServiceState::StartPending, 0))?;

    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(async {
        let cfg = Config::load(&config::default_config_path())?;
        let running = server::start(&cfg).await?;
        tracing::info!(addr = %running.addr, ca = running.ca_fingerprint.as_deref().unwrap_or("—"), "служба CRM запущена");
        handle.set_service_status(status_of(ServiceState::Running, 0))?;
        running.run_until(stop.notified()).await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    });
    if let Err(e) = &result {
        tracing::error!(error = %e, "ошибка сервера");
        // Простой файл в обход журнала — на случай, если сам журнал не открылся.
        let _ = std::fs::write(
            config::default_data_root().join("last-error.txt"),
            format!("{e}\n{e:?}\n"),
        );
    }
    handle.set_service_status(status_of(ServiceState::Stopped, u32::from(result.is_err())))?;
    result
}
