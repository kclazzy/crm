//! Рабочее время организации и тихие часы (ТЗ §4.5).

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

/// Сколько дней максимум просматриваем вперёд/назад в поиске рабочего окна.
const SEARCH_DAYS: i64 = 366;

/// Рабочее время организации. Конструктор гарантирует `start < end` и хотя бы один рабочий день.
#[derive(Debug, Clone)]
pub struct WorkSchedule {
    tz: Tz,
    /// Рабочие дни, индекс — номер дня от понедельника.
    days: [bool; 7],
    start: NaiveTime,
    end: NaiveTime,
    holidays: BTreeSet<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("начало рабочего дня должно быть раньше конца")]
    EmptyDay,
    #[error("не выбрано ни одного рабочего дня")]
    NoWorkDays,
}

impl WorkSchedule {
    pub fn new(
        tz: Tz,
        days: impl IntoIterator<Item = Weekday>,
        start: NaiveTime,
        end: NaiveTime,
        holidays: impl IntoIterator<Item = NaiveDate>,
    ) -> Result<Self, ScheduleError> {
        let mut set = [false; 7];
        for d in days {
            set[d.num_days_from_monday() as usize] = true;
        }
        if start >= end {
            return Err(ScheduleError::EmptyDay);
        }
        if !set.contains(&true) {
            return Err(ScheduleError::NoWorkDays);
        }
        Ok(Self {
            tz,
            days: set,
            start,
            end,
            holidays: holidays.into_iter().collect(),
        })
    }

    pub fn tz(&self) -> Tz {
        self.tz
    }

    fn is_work_day(&self, d: NaiveDate) -> bool {
        self.days[d.weekday().num_days_from_monday() as usize] && !self.holidays.contains(&d)
    }

    /// Рабочее окно дня `[start, end)` в UTC, если день рабочий.
    fn window(&self, d: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        if !self.is_work_day(d) {
            return None;
        }
        let s = self.local(d, self.start)?;
        let e = self.local(d, self.end)?;
        Some((s, e))
    }

    /// Локальное время → UTC. При переходе на летнее время берётся более раннее значение;
    /// несуществующее время (пропуск часа) — рабочее окно этого дня пропускается.
    fn local(&self, d: NaiveDate, t: NaiveTime) -> Option<DateTime<Utc>> {
        self.tz
            .from_local_datetime(&d.and_time(t))
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn local_date(&self, t: DateTime<Utc>) -> NaiveDate {
        t.with_timezone(&self.tz).date_naive()
    }

    pub fn is_working(&self, t: DateTime<Utc>) -> bool {
        self.window(self.local_date(t))
            .is_some_and(|(s, e)| s <= t && t < e)
    }

    /// Ближайший момент `>= t`, когда идёт рабочее время.
    pub fn next_work_start(&self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let first = self.local_date(t);
        (0..SEARCH_DAYS)
            .filter_map(|i| first.checked_add_signed(Duration::days(i)))
            .filter_map(|d| self.window(d))
            .find_map(|(s, e)| {
                if t < s {
                    Some(s)
                } else if t < e {
                    Some(t)
                } else {
                    None
                }
            })
    }

    /// Конец последнего рабочего окна, закончившегося не позже `t` (или идущего в `t`).
    pub fn prev_work_end(&self, t: DateTime<Utc>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let first = self.local_date(t);
        (0..SEARCH_DAYS)
            .filter_map(|i| first.checked_sub_signed(Duration::days(i)))
            .filter_map(|d| self.window(d))
            .find(|(s, _)| *s < t)
            .map(|(s, e)| (s, e.min(t)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono_tz::Europe::Moscow;

    /// Пн–Пт 09:00–19:00 по Москве (UTC+3), 4 ноября — праздник.
    pub(crate) fn office() -> WorkSchedule {
        WorkSchedule::new(
            Moscow,
            [
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ],
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
            [NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()],
        )
        .unwrap()
    }

    /// Московское время → UTC. 13.11.2026 — пятница.
    pub(crate) fn msk(day: u32, h: u32, m: u32) -> DateTime<Utc> {
        Moscow
            .with_ymd_and_hms(2026, 11, day, h, m, 0)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn rejects_invalid_schedules() {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        assert_eq!(
            WorkSchedule::new(Moscow, [Weekday::Mon], t(19), t(9), []).err(),
            Some(ScheduleError::EmptyDay)
        );
        assert_eq!(
            WorkSchedule::new(Moscow, [], t(9), t(19), []).err(),
            Some(ScheduleError::NoWorkDays)
        );
    }

    #[test]
    fn working_time_respects_hours_weekends_and_holidays() {
        let s = office();
        assert!(s.is_working(msk(13, 9, 0)));
        assert!(s.is_working(msk(13, 18, 59)));
        assert!(!s.is_working(msk(13, 19, 0)));
        assert!(!s.is_working(msk(13, 8, 59)));
        assert!(!s.is_working(msk(14, 12, 0)), "суббота");
        assert!(!s.is_working(msk(4, 12, 0)), "праздник");
    }

    #[test]
    fn next_work_start_jumps_over_night_weekend_and_holiday() {
        let s = office();
        assert_eq!(s.next_work_start(msk(13, 12, 0)), Some(msk(13, 12, 0)));
        assert_eq!(s.next_work_start(msk(12, 7, 0)), Some(msk(12, 9, 0)));
        assert_eq!(
            s.next_work_start(msk(13, 20, 0)),
            Some(msk(16, 9, 0)),
            "пт вечер → пн"
        );
        assert_eq!(
            s.next_work_start(msk(3, 20, 0)),
            Some(msk(5, 9, 0)),
            "вт вечер → чт, ср праздник"
        );
    }

    #[test]
    fn prev_work_end_finds_last_window_before() {
        let s = office();
        assert_eq!(
            s.prev_work_end(msk(14, 6, 0)),
            Some((msk(13, 9, 0), msk(13, 19, 0)))
        );
        assert_eq!(
            s.prev_work_end(msk(13, 12, 0)),
            Some((msk(13, 9, 0), msk(13, 12, 0)))
        );
        assert_eq!(
            s.prev_work_end(msk(13, 8, 0)),
            Some((msk(12, 9, 0), msk(12, 19, 0)))
        );
    }
}
