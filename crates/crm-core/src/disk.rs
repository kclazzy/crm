//! Оценка заполненности диска и прогноз (ТЗ §2.7).

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

const GB: u64 = 1024 * 1024 * 1024;
/// Меньше этого свободного места — новые вложения не принимаются.
pub const MIN_FREE_FOR_UPLOADS: u64 = 2 * GB;
/// Меньше этого — база переходит в режим «только чтение».
pub const MIN_FREE_FOR_WRITES: u64 = GB;
/// Окно, по которому считается скорость роста.
pub const TREND_WINDOW: Duration = Duration::days(30);

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub warn_percent: u8,
    pub critical_percent: u8,
    pub warn_days: i64,
    pub critical_days: i64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            warn_percent: 80,
            critical_percent: 90,
            warn_days: 60,
            critical_days: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sample {
    pub at: DateTime<Utc>,
    pub free_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskLevel {
    Ok,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DiskStatus {
    pub level: DiskLevel,
    /// Через сколько дней диск заполнится при текущем темпе; `None` — место не убывает
    /// или истории меньше суток.
    pub days_left: Option<i64>,
    pub uploads_allowed: bool,
    pub writes_allowed: bool,
}

pub fn assess(
    total: u64,
    free: u64,
    history: &[Sample],
    now: DateTime<Utc>,
    th: Thresholds,
) -> DiskStatus {
    let days_left = forecast_days(free, history, now);
    let used = u128::from(total.saturating_sub(free)) * 100;
    let pct_at_least = |p: u8| used >= u128::from(total) * u128::from(p);
    let soon = |d: i64| days_left.is_some_and(|left| left < d);
    let writes_allowed = free >= MIN_FREE_FOR_WRITES;

    let level = if !writes_allowed || pct_at_least(th.critical_percent) || soon(th.critical_days) {
        DiskLevel::Critical
    } else if pct_at_least(th.warn_percent) || soon(th.warn_days) {
        DiskLevel::Warning
    } else {
        DiskLevel::Ok
    };
    DiskStatus {
        level,
        days_left,
        uploads_allowed: free >= MIN_FREE_FOR_UPLOADS,
        writes_allowed,
    }
}

/// Прогноз по самому старому замеру внутри окна: (сколько убыло) / (за сколько времени).
fn forecast_days(free: u64, history: &[Sample], now: DateTime<Utc>) -> Option<i64> {
    let oldest = history
        .iter()
        .filter(|s| s.at >= now - TREND_WINDOW && s.at < now)
        .min_by_key(|s| s.at)?;
    let span = now - oldest.at;
    if span < Duration::days(1) || oldest.free_bytes <= free {
        return None;
    }
    let dropped = u128::from(oldest.free_bytes - free);
    let span_secs = u128::try_from(span.num_seconds()).ok()?;
    let days = u128::from(free) * span_secs / (dropped * 86_400);
    i64::try_from(days).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0).unwrap()
    }
    fn days_ago(d: i64, free_gb: u64) -> Sample {
        Sample {
            at: now() - Duration::days(d),
            free_bytes: free_gb * GB,
        }
    }
    fn run(total_gb: u64, free_gb: u64, h: &[Sample]) -> DiskStatus {
        assess(total_gb * GB, free_gb * GB, h, now(), Thresholds::default())
    }

    #[test]
    fn level_by_fill_percent() {
        assert_eq!(run(100, 21, &[]).level, DiskLevel::Ok);
        assert_eq!(run(100, 20, &[]).level, DiskLevel::Warning);
        assert_eq!(run(100, 10, &[]).level, DiskLevel::Critical);
    }

    #[test]
    fn forecast_from_growth_over_window() {
        // За 10 дней свободное место упало со 110 до 100 ГБ — 1 ГБ/день → 100 дней.
        let s = run(
            500,
            100,
            &[days_ago(40, 200), days_ago(10, 110), days_ago(1, 101)],
        );
        assert_eq!(s.days_left, Some(100));
        assert_eq!(s.level, DiskLevel::Warning, "80 % заполнено");
    }

    #[test]
    fn forecast_raises_level_when_disk_fills_soon() {
        let s = run(1000, 500, &[days_ago(20, 700)]); // 10 ГБ/день → 50 дней
        assert_eq!(s.days_left, Some(50));
        assert_eq!(s.level, DiskLevel::Warning);
        let s = run(1000, 200, &[days_ago(10, 400)]); // 20 ГБ/день → 10 дней
        assert_eq!(s.level, DiskLevel::Critical);
    }

    #[test]
    fn no_forecast_when_space_not_shrinking_or_history_short() {
        assert_eq!(run(500, 300, &[days_ago(10, 290)]).days_left, None);
        assert_eq!(
            run(
                500,
                300,
                &[Sample {
                    at: now() - Duration::hours(5),
                    free_bytes: 310 * GB
                }]
            )
            .days_left,
            None
        );
        assert_eq!(run(500, 300, &[]).days_left, None);
    }

    #[test]
    fn low_space_blocks_uploads_then_writes() {
        let s = run(100, 3, &[]);
        assert!(s.uploads_allowed && s.writes_allowed);
        let s = assess(100 * GB, GB + GB / 2, &[], now(), Thresholds::default());
        assert!(!s.uploads_allowed && s.writes_allowed);
        let s = assess(100 * GB, GB / 2, &[], now(), Thresholds::default());
        assert!(!s.uploads_allowed && !s.writes_allowed);
        assert_eq!(s.level, DiskLevel::Critical);
    }
}
