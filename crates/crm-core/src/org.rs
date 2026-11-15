//! Настройки организации: рабочее время и порог «скоро дедлайн» (ТЗ §4.3, §4.5).

use chrono::{NaiveDate, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

use crate::schedule::{ScheduleError, WorkSchedule};

/// Проверенные настройки; расписание построено при разборе.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawOrg", into = "RawOrg")]
pub struct OrgSettings {
    raw: RawOrg,
    schedule: WorkSchedule,
}

impl PartialEq for OrgSettings {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawOrg {
    /// Часовой пояс IANA, например `Europe/Moscow`.
    pub timezone: String,
    /// Рабочие дни: 1 — понедельник … 7 — воскресенье.
    pub work_days: Vec<u8>,
    /// `ЧЧ:ММ`.
    pub work_start: String,
    pub work_end: String,
    pub holidays: Vec<NaiveDate>,
    /// За сколько часов до дедлайна задача становится «скоро».
    pub warn_hours: u32,
}

impl Default for RawOrg {
    fn default() -> Self {
        Self {
            timezone: "Europe/Moscow".into(),
            work_days: vec![1, 2, 3, 4, 5],
            work_start: "09:00".into(),
            work_end: "19:00".into(),
            holidays: Vec::new(),
            warn_hours: 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrgError {
    #[error("неизвестный часовой пояс «{0}»")]
    Timezone(String),
    #[error("время «{0}» — ожидается ЧЧ:ММ")]
    Time(String),
    #[error("день недели {0} — ожидается 1–7")]
    Day(u8),
    #[error("порог «скоро дедлайн»: от 1 до 168 часов")]
    WarnHours,
    #[error("{0}")]
    Schedule(#[from] ScheduleError),
}

impl TryFrom<RawOrg> for OrgSettings {
    type Error = OrgError;
    fn try_from(raw: RawOrg) -> Result<Self, OrgError> {
        if !(1..=168).contains(&raw.warn_hours) {
            return Err(OrgError::WarnHours);
        }
        let schedule = build_schedule(&raw)?;
        Ok(Self { raw, schedule })
    }
}

impl From<OrgSettings> for RawOrg {
    fn from(s: OrgSettings) -> Self {
        s.raw
    }
}

fn time(s: &str) -> Result<NaiveTime, OrgError> {
    NaiveTime::parse_from_str(s, "%H:%M").map_err(|_| OrgError::Time(s.to_owned()))
}

fn weekday(d: u8) -> Result<Weekday, OrgError> {
    match d {
        1..=7 => Weekday::try_from(d - 1).map_err(|_| OrgError::Day(d)),
        _ => Err(OrgError::Day(d)),
    }
}

fn build_schedule(raw: &RawOrg) -> Result<WorkSchedule, OrgError> {
    let tz = raw
        .timezone
        .parse()
        .map_err(|_| OrgError::Timezone(raw.timezone.clone()))?;
    let days = raw
        .work_days
        .iter()
        .map(|d| weekday(*d))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkSchedule::new(
        tz,
        days,
        time(&raw.work_start)?,
        time(&raw.work_end)?,
        raw.holidays.iter().copied(),
    )?)
}

impl OrgSettings {
    pub fn schedule(&self) -> &WorkSchedule {
        &self.schedule
    }

    pub fn tz(&self) -> chrono_tz::Tz {
        self.schedule.tz()
    }

    pub fn warn_hours(&self) -> i64 {
        i64::from(self.raw.warn_hours)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<OrgSettings, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    #[test]
    fn defaults_are_valid_office_hours() {
        let s = parse("{}").unwrap();
        assert_eq!(s.warn_hours(), 24);
        assert_eq!(s.tz(), chrono_tz::Europe::Moscow);
    }

    #[test]
    fn rejects_bad_values_with_readable_reason() {
        assert!(parse(r#"{"timezone":"Mars/Base"}"#)
            .unwrap_err()
            .contains("Mars/Base"));
        assert!(parse(r#"{"work_start":"9"}"#)
            .unwrap_err()
            .contains("ЧЧ:ММ"));
        assert!(parse(r#"{"work_days":[8]}"#).is_err());
        assert!(parse(r#"{"work_days":[]}"#).is_err());
        assert!(parse(r#"{"work_start":"19:00","work_end":"09:00"}"#).is_err());
        assert!(parse(r#"{"warn_hours":0}"#).is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let s = parse(r#"{"work_days":[1,2,3,4,5,6],"holidays":["2026-12-31"]}"#).unwrap();
        let back = parse(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
