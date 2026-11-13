//! Журнал службы пишется в файлы (отдельный тестовый бинарник: глобальный подписчик один).
#![allow(clippy::unwrap_used)]

#[test]
fn service_log_goes_to_daily_file() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    let guard = crm_server::logging::to_files(&logs).expect("журнал должен открыться");
    tracing::info!("проверка журнала");
    drop(guard);
    let files: Vec<_> = std::fs::read_dir(&logs)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("crm-server.") && name.ends_with(".log"),
        "{name}"
    );
    assert!(std::fs::read_to_string(&files[0])
        .unwrap()
        .contains("проверка журнала"));
}
