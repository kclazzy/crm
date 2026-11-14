//! Имена файлов бэкапов и правило ротации (ТЗ §5).

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

const PREFIX: &str = "backup_";
const SUFFIX: &str = ".db.zst";
const STAMP: &str = "%Y_%m_%d_%H_%M";

/// Как часто можно делать копии (часы) — значения, которые делят сутки ровно.
pub const ALLOWED_EVERY_HOURS: [u32; 8] = [1, 2, 3, 4, 6, 8, 12, 24];

/// Настройки резервного копирования, которые выбирает владелец. Создаётся только
/// через проверку — недопустимую политику (в том числе из config.toml) не записать.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawPolicy", into = "RawPolicy")]
pub struct Policy {
    enabled: bool,
    every_hours: u32,
    keep_all_hours: u32,
    keep_days: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicy {
    pub enabled: bool,
    pub every_hours: u32,
    pub keep_all_hours: u32,
    pub keep_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("копию можно делать каждые 1, 2, 3, 4, 6, 8, 12 или 24 часа")]
    Every,
    #[error("все копии можно хранить от 1 часа до 7 суток")]
    KeepAll,
    #[error("по одной копии в день можно хранить от 1 до 365 дней")]
    KeepDays,
    #[error("срок хранения ежедневных копий должен быть не меньше срока хранения всех копий")]
    Order,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            every_hours: 1,
            keep_all_hours: 48,
            keep_days: 30,
        }
    }
}

impl TryFrom<RawPolicy> for Policy {
    type Error = PolicyError;
    fn try_from(r: RawPolicy) -> Result<Self, PolicyError> {
        Policy::new(r.enabled, r.every_hours, r.keep_all_hours, r.keep_days)
    }
}

impl From<Policy> for RawPolicy {
    fn from(p: Policy) -> Self {
        RawPolicy {
            enabled: p.enabled,
            every_hours: p.every_hours,
            keep_all_hours: p.keep_all_hours,
            keep_days: p.keep_days,
        }
    }
}

impl Policy {
    pub fn new(
        enabled: bool,
        every_hours: u32,
        keep_all_hours: u32,
        keep_days: u32,
    ) -> Result<Self, PolicyError> {
        if !ALLOWED_EVERY_HOURS.contains(&every_hours) {
            return Err(PolicyError::Every);
        }
        if !(1..=168).contains(&keep_all_hours) {
            return Err(PolicyError::KeepAll);
        }
        if !(1..=365).contains(&keep_days) {
            return Err(PolicyError::KeepDays);
        }
        if keep_days * 24 < keep_all_hours {
            return Err(PolicyError::Order);
        }
        Ok(Self {
            enabled,
            every_hours,
            keep_all_hours,
            keep_days,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Пора ли делать копию: с последней удачной прошло не меньше интервала.
    pub fn is_due(&self, last_success: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
        self.enabled
            && last_success.is_none_or(|t| now - t >= Duration::hours(i64::from(self.every_hours)))
    }

    /// Сколько копий будет лежать в папке в установившемся режиме (для подсказки владельцу).
    pub fn expected_copies(&self) -> u32 {
        let recent = (self.keep_all_hours / self.every_hours).max(1);
        let daily = self
            .keep_days
            .saturating_sub(self.keep_all_hours.div_ceil(24));
        recent + daily
    }

    pub fn every_hours(&self) -> u32 {
        self.every_hours
    }

    fn keep_all(&self) -> Duration {
        Duration::hours(i64::from(self.keep_all_hours))
    }

    fn keep_daily(&self) -> Duration {
        Duration::days(i64::from(self.keep_days))
    }
}

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

/// Какие бэкапы удалить. Хранится: всё моложе `keep_all`; старше — последняя копия
/// за каждые сутки (по часовому поясу организации) в пределах `keep_days`; остальное — удалить.
/// Повторный вызов на результате ничего не удаляет (идемпотентно).
pub fn to_delete(
    backups: &[DateTime<Utc>],
    now: DateTime<Utc>,
    tz: Tz,
    policy: &Policy,
) -> Vec<DateTime<Utc>> {
    use std::collections::BTreeMap;

    let mut delete = Vec::new();
    // Последняя копия каждых суток в зоне 48 ч … 30 дней.
    let mut newest_per_day: BTreeMap<NaiveDate, DateTime<Utc>> = BTreeMap::new();
    for &t in backups {
        let age = now - t;
        if age <= policy.keep_all() {
            continue;
        }
        if age > policy.keep_daily() {
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
        let del = to_delete(&all, now, Moscow, &Policy::default());
        let kept: Vec<_> = all.iter().filter(|t| !del.contains(t)).copied().collect();

        assert!(
            kept.iter().all(|t| now - *t <= Duration::days(30)),
            "старше 30 дней — удалены"
        );
        let recent = kept
            .iter()
            .filter(|t| now - **t <= Duration::hours(48))
            .count();
        assert_eq!(recent, 49, "все почасовые за 48 ч (включая текущий)");
        let mut days: Vec<NaiveDate> = kept
            .iter()
            .filter(|t| now - **t > Duration::hours(48))
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
            to_delete(&kept, now, Moscow, &Policy::default()).is_empty(),
            "повторная ротация ничего не удаляет"
        );
    }

    #[test]
    fn future_backups_are_kept() {
        let now = utc(13, 12);
        assert!(to_delete(&[now + Duration::hours(3)], now, Moscow, &Policy::default()).is_empty());
    }

    #[test]
    fn policy_accepts_only_sensible_values() {
        assert!(Policy::new(true, 6, 24, 14).is_ok());
        assert_eq!(
            Policy::new(true, 5, 24, 14),
            Err(PolicyError::Every),
            "5 ч не делит сутки"
        );
        assert_eq!(Policy::new(true, 1, 0, 14), Err(PolicyError::KeepAll));
        assert_eq!(Policy::new(true, 1, 200, 30), Err(PolicyError::KeepAll));
        assert_eq!(Policy::new(true, 1, 48, 0), Err(PolicyError::KeepDays));
        assert_eq!(
            Policy::new(true, 1, 168, 3),
            Err(PolicyError::Order),
            "7 суток всех копий, а ежедневных — 3 дня"
        );
        let bad: Result<Policy, _> = serde_json::from_str(
            r#"{"enabled":true,"every_hours":7,"keep_all_hours":48,"keep_days":30}"#,
        );
        assert!(bad.is_err(), "недопустимая политика не читается и из файла");
    }

    #[test]
    fn copy_is_due_after_interval() {
        let p = Policy::new(true, 6, 48, 30).unwrap();
        let now = utc(13, 12);
        assert!(p.is_due(None, now), "копий ещё не было");
        assert!(!p.is_due(Some(now - Duration::hours(5)), now));
        assert!(p.is_due(Some(now - Duration::hours(6)), now));
        let off = Policy::new(false, 1, 48, 30).unwrap();
        assert!(!off.is_due(None, now), "выключено — никогда");
    }

    #[test]
    fn expected_copies_hint() {
        assert_eq!(
            Policy::default().expected_copies(),
            48 + 28,
            "48 почасовых + 28 суточных"
        );
        assert_eq!(
            Policy::new(true, 24, 24, 7).unwrap().expected_copies(),
            1 + 6
        );
    }

    #[test]
    fn custom_retention_is_respected() {
        let now = utc(13, 12);
        let p = Policy::new(true, 1, 12, 7).unwrap();
        let ten_days = now - Duration::days(10);
        let five_days = now - Duration::days(5);
        let six_hours = now - Duration::hours(6);
        let del = to_delete(&[ten_days, five_days, six_hours], now, Moscow, &p);
        assert_eq!(
            del,
            vec![ten_days],
            "старше 7 дней — удалить, остальные оставить"
        );
    }
}
