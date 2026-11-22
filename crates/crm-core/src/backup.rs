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
    keep: KeepMode,
    max_gb: Option<u32>,
}

/// Как хранить копии.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeepMode {
    /// Все копии за `keep_all_hours`, дальше по одной в сутки `keep_days` дней.
    #[default]
    Window,
    /// Ровно 3 файла: последняя, моложе 2 суток и от 2 суток до ~2 недель.
    Ladder,
}

/// Меньше стольких копий уборка не оставляет никогда (если они вообще есть).
pub const MIN_KEEP: usize = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicy {
    pub enabled: bool,
    pub every_hours: u32,
    pub keep_all_hours: u32,
    pub keep_days: u32,
    #[serde(default)]
    pub keep: KeepMode,
    /// Потолок места под копии в одной папке, ГБ; `None` — без потолка.
    #[serde(default)]
    pub max_gb: Option<u32>,
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
    #[error("потолок места для копий — от 1 до 10000 ГБ")]
    MaxGb,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            every_hours: 1,
            keep_all_hours: 48,
            keep_days: 30,
            keep: KeepMode::Window,
            max_gb: None,
        }
    }
}

impl TryFrom<RawPolicy> for Policy {
    type Error = PolicyError;
    fn try_from(r: RawPolicy) -> Result<Self, PolicyError> {
        Policy::from_raw(r)
    }
}

impl From<Policy> for RawPolicy {
    fn from(p: Policy) -> Self {
        RawPolicy {
            enabled: p.enabled,
            every_hours: p.every_hours,
            keep_all_hours: p.keep_all_hours,
            keep_days: p.keep_days,
            keep: p.keep,
            max_gb: p.max_gb,
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
            keep: KeepMode::Window,
            max_gb: None,
        })
    }

    /// Полная проверка, включая способ хранения и потолок места.
    pub fn from_raw(r: RawPolicy) -> Result<Self, PolicyError> {
        let mut p = Policy::new(r.enabled, r.every_hours, r.keep_all_hours, r.keep_days)?;
        if r.max_gb.is_some_and(|g| !(1..=10_000).contains(&g)) {
            return Err(PolicyError::MaxGb);
        }
        p.keep = r.keep;
        p.max_gb = r.max_gb;
        Ok(p)
    }

    pub fn keep_mode(&self) -> KeepMode {
        self.keep
    }

    pub fn max_gb(&self) -> Option<u32> {
        self.max_gb
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
        if self.keep == KeepMode::Ladder {
            return MIN_KEEP as u32;
        }
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

/// Какие копии удалить. `copies` — время и размер каждой копии в папке.
///
/// 1. Основное правило — по способу хранения ([`KeepMode`]).
/// 2. Самая новая копия не удаляется никогда, и меньше [`MIN_KEEP`] копий не остаётся.
/// 3. Потолок места: сверх него удаляются самые старые, но не ниже п. 2.
///
/// Сутки считаются по часовому поясу организации. Повторный вызов на результате
/// ничего не удаляет (идемпотентно).
pub fn to_delete(
    copies: &[(DateTime<Utc>, u64)],
    now: DateTime<Utc>,
    tz: Tz,
    policy: &Policy,
) -> Vec<DateTime<Utc>> {
    let mut all: Vec<(DateTime<Utc>, u64)> = copies.to_vec();
    all.sort_by_key(|c| std::cmp::Reverse(c.0)); // новые первыми
    all.dedup_by_key(|c| c.0);
    let times: Vec<DateTime<Utc>> = all.iter().map(|c| c.0).collect();
    let mut keep: Vec<bool> = match policy.keep {
        KeepMode::Window => window_keep(&times, now, tz, policy),
        KeepMode::Ladder => ladder_keep(&times, now),
    };
    // Самая новая и не меньше MIN_KEEP — добираем самыми новыми из удаляемых.
    for k in keep.iter_mut().take(1) {
        *k = true;
    }
    let mut kept = keep.iter().filter(|k| **k).count();
    for k in keep.iter_mut() {
        if kept >= MIN_KEEP.min(times.len()) {
            break;
        }
        if !*k {
            *k = true;
            kept += 1;
        }
    }
    if let Some(gb) = policy.max_gb {
        let cap = u64::from(gb) << 30;
        let mut total: u64 = all
            .iter()
            .zip(&keep)
            .filter(|(_, k)| **k)
            .map(|(c, _)| c.1)
            .sum();
        for i in (0..all.len()).rev() {
            if total <= cap || kept <= MIN_KEEP {
                break;
            }
            if keep[i] && i > 0 {
                keep[i] = false;
                kept -= 1;
                total -= all[i].1;
            }
        }
    }
    all.iter()
        .zip(&keep)
        .filter(|(_, k)| !**k)
        .map(|(c, _)| c.0)
        .collect()
}

/// Окно: всё моложе `keep_all`, дальше последняя копия каждых суток до `keep_days`.
fn window_keep(times: &[DateTime<Utc>], now: DateTime<Utc>, tz: Tz, p: &Policy) -> Vec<bool> {
    let mut seen_days: Vec<NaiveDate> = Vec::new();
    times
        .iter()
        .map(|&t| {
            let age = now - t;
            if age <= p.keep_all() {
                return true;
            }
            if age > p.keep_daily() {
                return false;
            }
            // Новые идут первыми — первая встреченная за сутки и есть последняя.
            let day = t.with_timezone(&tz).date_naive();
            if seen_days.contains(&day) {
                false
            } else {
                seen_days.push(day);
                true
            }
        })
        .collect()
}

/// Лесенка: последняя; самая старая из моложе 2 суток; самая старая из «от суток до
/// 2 недель» (а если такой нет — самая старая из оставшихся, чтобы не терять историю).
fn ladder_keep(times: &[DateTime<Utc>], now: DateTime<Utc>) -> Vec<bool> {
    let mut keep = vec![false; times.len()];
    if times.is_empty() {
        return keep;
    }
    keep[0] = true;
    let age = |i: usize| now - times[i];
    let oldest = |pred: &dyn Fn(usize) -> bool, keep: &[bool]| {
        (1..times.len()).rev().find(|&i| !keep[i] && pred(i))
    };
    if let Some(b) = oldest(&|i| age(i) < Duration::hours(48), &keep) {
        keep[b] = true;
    }
    let c = oldest(
        &|i| age(i) >= Duration::hours(24) && age(i) < Duration::days(14),
        &keep,
    )
    .or_else(|| oldest(&|_| true, &keep));
    if let Some(c) = c {
        keep[c] = true;
    }
    keep
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
        let del = to_delete(&sized(&all), now, Moscow, &Policy::default());
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
            to_delete(&sized(&kept), now, Moscow, &Policy::default()).is_empty(),
            "повторная ротация ничего не удаляет"
        );
    }

    #[test]
    fn future_backups_are_kept() {
        let now = utc(13, 12);
        assert!(to_delete(
            &sized(&[now + Duration::hours(3)]),
            now,
            Moscow,
            &Policy::default()
        )
        .is_empty());
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
        let (two, one) = (now - Duration::hours(2), now - Duration::hours(1));
        let del = to_delete(
            &sized(&[ten_days, five_days, six_hours, two, one]),
            now,
            Moscow,
            &p,
        );
        assert_eq!(
            del,
            vec![ten_days],
            "старше 7 дней — удалить, остальные оставить"
        );
    }

    fn sized(ts: &[DateTime<Utc>]) -> Vec<(DateTime<Utc>, u64)> {
        ts.iter().map(|t| (*t, 1)).collect()
    }

    #[test]
    fn newest_copy_and_at_least_three_survive_any_rule() {
        let now = utc(13, 12);
        // Копии перестали делаться два месяца назад — все «старые».
        let old: Vec<_> = (0..5).map(|d| now - Duration::days(60 + d)).collect();
        let del = to_delete(&sized(&old), now, Moscow, &Policy::default());
        assert_eq!(del.len(), 2, "из 5 старых остаются 3");
        assert!(!del.contains(&old[0]), "самая новая не удаляется никогда");
        assert!(!del.contains(&old[1]) && !del.contains(&old[2]));
    }

    #[test]
    fn ladder_keeps_three_files_latest_recent_and_older() {
        let p = Policy::from_raw(RawPolicy {
            keep: KeepMode::Ladder,
            ..Policy::default().into()
        })
        .unwrap();
        // Каждый час 60 дней: после каждой копии — уборка, как на сервере.
        let start = utc(1, 0);
        let mut files: Vec<DateTime<Utc>> = Vec::new();
        let mut ages = Vec::new();
        for h in 0..60 * 24 {
            let now = start + Duration::hours(h);
            files.push(now);
            let del = to_delete(&sized(&files), now, Moscow, &p);
            files.retain(|t| !del.contains(t));
            if h > 30 * 24 {
                assert_eq!(files.len(), 3, "лесенка — ровно 3 файла");
                let mut a: Vec<i64> = files.iter().map(|t| (now - *t).num_hours()).collect();
                a.sort_unstable();
                ages.push(a);
            }
        }
        assert!(ages.iter().all(|a| a[0] == 0), "первая — последняя копия");
        assert!(
            ages.iter().all(|a| (1..48).contains(&a[1])),
            "вторая — моложе 2 суток"
        );
        assert!(
            ages.iter().all(|a| (48..=16 * 24).contains(&a[2])),
            "третья — от 2 суток до ~2 недель"
        );
    }

    #[test]
    fn size_cap_removes_oldest_but_keeps_three() {
        let now = utc(13, 12);
        let p = Policy::from_raw(RawPolicy {
            max_gb: Some(1),
            ..Policy::default().into()
        })
        .unwrap();
        let gb = 1 << 30;
        let copies: Vec<_> = (0..5).map(|h| (now - Duration::hours(h), gb / 2)).collect();
        let mut del = to_delete(&copies, now, Moscow, &p);
        del.sort();
        assert_eq!(
            del,
            vec![now - Duration::hours(4), now - Duration::hours(3)],
            "старые — первыми"
        );
        let big: Vec<_> = (0..5).map(|h| (now - Duration::hours(h), gb * 2)).collect();
        assert_eq!(
            to_delete(&big, now, Moscow, &p).len(),
            2,
            "меньше 3 копий не оставляем"
        );
    }

    #[test]
    fn old_config_without_new_fields_still_reads() {
        let p: Policy = serde_json::from_str(
            r#"{"enabled":true,"every_hours":6,"keep_all_hours":48,"keep_days":30}"#,
        )
        .unwrap();
        assert_eq!(p.keep_mode(), KeepMode::Window);
        assert_eq!(p.max_gb(), None);
        assert!(Policy::from_raw(RawPolicy {
            max_gb: Some(0),
            ..p.into()
        })
        .is_err());
    }
}
