//! Сколько живёт вход (выбирает владелец): в офисе и из интернета по-разному.

use serde::{Deserialize, Serialize};

const HOUR_MS: i64 = 3600 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// Проверенные сроки. Недопустимые значения (в том числе из config.toml) не записать.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawLimits", into = "RawLimits")]
pub struct Limits {
    office_days: u32,
    remote_idle_hours: u32,
    remote_code_days: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLimits {
    /// В офисе вход живёт столько дней с последнего действия.
    pub office_days: u32,
    /// Из интернета: столько часов без действий — и выход.
    pub remote_idle_hours: u32,
    /// Из интернета: код из приложения заново раз в столько дней; 0 — не спрашивать.
    pub remote_code_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitsError {
    #[error("в офисе вход может жить от 1 до 90 дней")]
    Office,
    #[error("из интернета без действий — от 1 часа до 30 дней")]
    RemoteIdle,
    #[error("код заново — раз в 1–30 дней или не спрашивать (0)")]
    RemoteCode,
}

impl Default for Limits {
    /// Как было до настройки: 30 дней с последнего действия везде, код — только при входе.
    fn default() -> Self {
        Self {
            office_days: 30,
            remote_idle_hours: 30 * 24,
            remote_code_days: 0,
        }
    }
}

impl TryFrom<RawLimits> for Limits {
    type Error = LimitsError;
    fn try_from(r: RawLimits) -> Result<Self, LimitsError> {
        if !(1..=90).contains(&r.office_days) {
            return Err(LimitsError::Office);
        }
        if !(1..=30 * 24).contains(&r.remote_idle_hours) {
            return Err(LimitsError::RemoteIdle);
        }
        if r.remote_code_days > 30 {
            return Err(LimitsError::RemoteCode);
        }
        Ok(Self {
            office_days: r.office_days,
            remote_idle_hours: r.remote_idle_hours,
            remote_code_days: r.remote_code_days,
        })
    }
}

impl From<Limits> for RawLimits {
    fn from(l: Limits) -> Self {
        RawLimits {
            office_days: l.office_days,
            remote_idle_hours: l.remote_idle_hours,
            remote_code_days: l.remote_code_days,
        }
    }
}

impl Limits {
    /// Срок входа (мс) от последнего действия — по самому длинному, офисному.
    pub fn ttl_ms(&self) -> i64 {
        i64::from(self.office_days) * DAY_MS
    }

    /// Из интернета: вход закончился из-за простоя.
    pub fn remote_idle_expired(&self, last_seen: i64, now: i64) -> bool {
        now - last_seen > i64::from(self.remote_idle_hours) * HOUR_MS
    }

    /// Из интернета: пора снова спросить код (`mfa_at` — когда вводили).
    pub fn remote_code_stale(&self, mfa_at: Option<i64>, now: i64) -> bool {
        self.remote_code_days > 0
            && mfa_at.is_some_and(|t| now - t > i64::from(self.remote_code_days) * DAY_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(o: u32, i: u32, c: u32) -> Result<Limits, LimitsError> {
        Limits::try_from(RawLimits {
            office_days: o,
            remote_idle_hours: i,
            remote_code_days: c,
        })
    }

    #[test]
    fn only_sensible_limits_are_accepted() {
        assert!(l(30, 12, 7).is_ok());
        assert_eq!(l(0, 12, 7), Err(LimitsError::Office));
        assert_eq!(l(91, 12, 7), Err(LimitsError::Office));
        assert_eq!(l(30, 0, 7), Err(LimitsError::RemoteIdle));
        assert_eq!(l(30, 721, 7), Err(LimitsError::RemoteIdle));
        assert_eq!(l(30, 12, 31), Err(LimitsError::RemoteCode));
        assert_eq!(Limits::default().ttl_ms(), 30 * DAY_MS);
    }

    #[test]
    fn idle_and_code_age_are_counted_from_the_right_moment() {
        let x = l(30, 12, 7).unwrap();
        assert!(!x.remote_idle_expired(0, 12 * HOUR_MS));
        assert!(x.remote_idle_expired(0, 12 * HOUR_MS + 1));
        assert!(!x.remote_code_stale(Some(0), 7 * DAY_MS));
        assert!(x.remote_code_stale(Some(0), 7 * DAY_MS + 1));
        assert!(
            !x.remote_code_stale(None, 100 * DAY_MS),
            "кода не вводили — решает правило входа"
        );
        assert!(
            !l(30, 12, 0)
                .unwrap()
                .remote_code_stale(Some(0), 100 * DAY_MS),
            "0 — не спрашивать"
        );
    }
}
