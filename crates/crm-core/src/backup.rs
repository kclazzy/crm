//! Имена файлов бэкапов и правило ротации (ТЗ §5).

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;

const PREFIX: &str = "backup_";
const SUFFIX: &str = ".db.zst";
const STAMP: &str = "%Y_%m_%d_%H_%M";

/// Сколько хранить все почасовые копии.
pub const KEEP_ALL: Duration = Duration::hours(48);
/// Сколько хранить по одной копии в сутки.
pub const KEEP_DAILY: Duration = Duration::days(30);

/// Имя файла бэкапа. Время в имени — UTC, чтобы переход на летнее время не давал дублей.
pub fn file_name(t: DateTime<Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", t.format(STAMP))
}

pub fn parse_file_name(name: &str) -> Option<DateTime<Utc>> {
    let stamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    NaiveDateTime::parse_from_str(stamp, STAMP)
        .ok()
        .map(|n| n.and_utc())
}

/// Какие бэкапы удалить. Хранится: всё моложе 48 ч; старше — последняя копия
/// за каждые сутки (по часовому поясу организации) в пределах 30 дней; остальное — удалить.
/// Повторный вызов на результате ничего не удаляет (идемпотентно).
pub fn to_delete(backups: &[DateTime<Utc>], now: DateTime<Utc>, tz: Tz) -> Vec<DateTime<Utc>> {
    use std::collections::BTreeMap;

    let mut delete = Vec::new();
    // Последняя копия каждых суток в зоне 48 ч … 30 дней.
    let mut newest_per_day: BTreeMap<NaiveDate, DateTime<Utc>> = BTreeMap::new();
    for &t in backups {
        let age = now - t;
        if age <= KEEP_ALL {
            continue;
        }
        if age > KEEP_DAILY {
            delete.push(t);
            continue;
        }
        let day = t.with_timezone(&tz).date_naive();
        match newest_per_day.get(&day) {
            Some(&kept) if kept >= t => delete.push(t),
            Some(&kept) => {
                delete.push(kept);
                newest_per_day.insert(day, t);
            }
            None => {
                newest_per_day.insert(day, t);
            }
        }
    }
    delete
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Europe::Moscow;

    fn utc(d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap()
    }

    #[test]
    fn file_name_round_trips() {
        let t = Utc.with_ymd_and_hms(2026, 11, 13, 7, 5, 0).unwrap();
        assert_eq!(file_name(t), "backup_2026_11_13_07_05.db.zst");
        assert_eq!(parse_file_name(&file_name(t)), Some(t));
        assert_eq!(parse_file_name("backup_2026_13_40_07_05.db.zst"), None);
        assert_eq!(parse_file_name("notes.txt"), None);
    }

    #[test]
    fn rotation_keeps_hourly_48h_then_one_per_day_for_30_days() {
        let now = utc(13, 12);
        // Каждый час за последние 40 суток.
        let all: Vec<_> = (0..40 * 24).map(|h| now - Duration::hours(h)).collect();
        let del = to_delete(&all, now, Moscow);
        let kept: Vec<_> = all.iter().filter(|t| !del.contains(t)).copied().collect();

        assert!(
            kept.iter().all(|t| now - *t <= KEEP_DAILY),
            "старше 30 дней — удалены"
        );
        let recent = kept.iter().filter(|t| now - **t <= KEEP_ALL).count();
        assert_eq!(recent, 49, "все почасовые за 48 ч (включая текущий)");
        let mut days: Vec<NaiveDate> = kept
            .iter()
            .filter(|t| now - **t > KEEP_ALL)
            .map(|t| t.with_timezone(&Moscow).date_naive())
            .collect();
        let n = days.len();
        days.dedup();
        assert_eq!(
            days.len(),
            n,
            "старше 48 ч — не больше одной копии за сутки"
        );
        assert!(
            (28..=29).contains(&n),
            "около 28 суточных копий, получено {n}"
        );
        // Из суток оставлена именно последняя копия (23:00 по Москве = 20:00 UTC).
        let five_days_ago = kept.iter().find(|t| {
            t.with_timezone(&Moscow).date_naive() == NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
        });
        assert_eq!(five_days_ago, Some(&utc(8, 20)));

        assert!(
            to_delete(&kept, now, Moscow).is_empty(),
            "повторная ротация ничего не удаляет"
        );
    }

    #[test]
    fn future_backups_are_kept() {
        let now = utc(13, 12);
        assert!(to_delete(&[now + Duration::hours(3)], now, Moscow).is_empty());
    }
}
