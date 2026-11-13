//! Журналы: в окно (ручной запуск) или в файлы с ротацией (служба).

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

/// Сколько дней хранить файлы журнала (ТЗ §2.5.1).
const KEEP_DAYS: usize = 14;

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

pub fn to_console() {
    tracing_subscriber::fmt().with_env_filter(filter()).init();
}

/// Ежедневные файлы `crm-server.ГГГГ-ММ-ДД.log`, старше 14 дней удаляются.
/// Возвращённый guard нужно держать до конца работы — он дописывает буфер.
pub fn to_files(dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(dir).ok()?;
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("crm-server")
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_ansi(false)
        .with_writer(writer)
        .init();
    Some(guard)
}
