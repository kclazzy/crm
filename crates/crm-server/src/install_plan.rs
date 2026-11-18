//! Решения установщика, не зависящие от Windows (проверяются тестами везде).
//!
//! Роль компьютера — сервер или рабочее место — выбирает человек в программе.
//! Установщик сам возвращает службу только при обновлении: отметку «служба была»
//! оставляет деинсталлятор за минуты до установки новой версии. Старая отметка
//! (сервер удалили когда-то давно) службу не возвращает — программа покажет выбор.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Storage;

/// Отметка «служба была» действует столько: при обновлении старый деинсталлятор и
/// установка новой версии идут одна за другой, в пределах пары минут.
pub const MARKER_FRESH_FOR: Duration = Duration::from_secs(10 * 60);

/// Вернуть ли службу после установки. `marker` — содержимое отметки и её возраст.
/// `Some(true)` — вернуть с автозапуском, `Some(false)` — с ручным запуском.
pub fn restore_service(marker: Option<(&str, Duration)>) -> Option<bool> {
    let (text, age) = marker?;
    (age <= MARKER_FRESH_FOR).then(|| text.trim() != "demand")
}

/// Убрать прежние базу и вложения в `old-<stamp>` рядом с данными — для установки
/// с чистой базой. Сертификаты, настройки и ключ удалённого доступа остаются: для
/// программ сотрудников это тот же сервер (не нужно заново сверять отпечаток). Возвращает папку с прежними данными
/// или `None`, если переносить нечего.
pub fn archive_old_data(storage: &Storage, stamp: &str) -> std::io::Result<Option<PathBuf>> {
    let has_db = storage.db_dir.join(crate::db::DB_FILE).exists();
    let has_files = non_empty(&storage.attachments_dir);
    if !has_db && !has_files {
        return Ok(None);
    }
    let root = storage
        .db_dir
        .parent()
        .ok_or_else(|| std::io::Error::other("у папки базы нет родительской папки"))?;
    let archive = root.join(format!("old-{stamp}"));
    std::fs::create_dir(&archive)?;
    // Из папки базы — только файлы crm.db*: сама база, её журнал и отложенное
    // восстановление (crm.db.restore — иначе новый сервер подставил бы прежнюю копию).
    let data = archive.join("data");
    std::fs::create_dir(&data)?;
    for entry in std::fs::read_dir(&storage.db_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(crate::db::DB_FILE) {
            std::fs::rename(entry.path(), data.join(&name))?;
        }
    }
    if storage.attachments_dir.exists() {
        std::fs::rename(&storage.attachments_dir, archive.join("attachments"))?;
    }
    Ok(Some(archive))
}

fn non_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn service_comes_back_only_right_after_uninstall() {
        assert_eq!(restore_service(None), None, "сервера не было");
        assert_eq!(
            restore_service(Some(("auto", MIN))),
            Some(true),
            "обновление"
        );
        assert_eq!(restore_service(Some(("demand\n", MIN))), Some(false));
        assert_eq!(restore_service(Some(("auto", 9 * MIN))), Some(true));
        assert_eq!(
            restore_service(Some(("auto", 11 * MIN))),
            None,
            "сервер удалили давно — выбор за человеком"
        );
        assert_eq!(restore_service(Some(("auto", 24 * 60 * MIN))), None);
    }

    fn storage(root: &Path) -> Storage {
        Storage {
            db_dir: root.join("data"),
            attachments_dir: root.join("attachments"),
            backup_dirs: Vec::new(),
        }
    }

    #[test]
    fn old_data_is_set_aside_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let st = storage(dir.path());
        std::fs::create_dir_all(&st.db_dir).unwrap();
        std::fs::write(st.db_dir.join("crm.db"), b"old").unwrap();
        std::fs::write(st.db_dir.join("crm.db-wal"), b"wal").unwrap();
        std::fs::write(st.db_dir.join("crm.db.restore"), b"staged").unwrap();
        std::fs::create_dir_all(st.attachments_dir.join("ab")).unwrap();
        std::fs::write(st.attachments_dir.join("ab/file"), b"x").unwrap();
        std::fs::create_dir_all(st.db_dir.join("tls")).unwrap();
        std::fs::write(st.db_dir.join("tls/ca.crt"), b"ca").unwrap();
        std::fs::write(st.db_dir.join("p2p.key"), b"key").unwrap();

        let archive = archive_old_data(&st, "20261118-101500").unwrap().unwrap();
        assert_eq!(archive, dir.path().join("old-20261118-101500"));
        assert_eq!(std::fs::read(archive.join("data/crm.db")).unwrap(), b"old");
        assert!(archive.join("data/crm.db-wal").exists());
        assert!(archive.join("attachments/ab/file").exists());
        assert!(
            !st.db_dir.join("crm.db").exists(),
            "новая база начнётся с нуля"
        );
        assert!(!st.attachments_dir.exists());
        assert!(!st.db_dir.join("crm.db-wal").exists());
        assert!(
            archive.join("data/crm.db.restore").exists(),
            "отложенное восстановление прежней базы не подставится в новую"
        );
        assert!(
            st.db_dir.join("tls/ca.crt").exists(),
            "сертификаты остаются"
        );
        assert!(
            st.db_dir.join("p2p.key").exists(),
            "ключ удалённого доступа остаётся"
        );
    }

    #[test]
    fn nothing_to_set_aside_on_a_clean_computer() {
        let dir = tempfile::tempdir().unwrap();
        let st = storage(dir.path());
        std::fs::create_dir_all(&st.db_dir).unwrap();
        assert_eq!(archive_old_data(&st, "x").unwrap(), None);
        assert!(!dir.path().join("old-x").exists());
    }

    #[test]
    fn second_fresh_install_in_the_same_second_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let st = storage(dir.path());
        std::fs::create_dir_all(&st.db_dir).unwrap();
        std::fs::write(st.db_dir.join("crm.db"), b"1").unwrap();
        archive_old_data(&st, "same").unwrap();
        std::fs::create_dir_all(&st.db_dir).unwrap();
        std::fs::write(st.db_dir.join("crm.db"), b"2").unwrap();
        assert!(
            archive_old_data(&st, "same").is_err(),
            "прежний архив не затирается"
        );
        assert_eq!(std::fs::read(st.db_dir.join("crm.db")).unwrap(), b"2");
    }
}
