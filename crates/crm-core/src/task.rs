//! Задача: приоритет, статус и вычисляемый индикатор дедлайна (ТЗ §4.2–4.3).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Приоритет задачи: 1 — высокий, 2 — средний, 3 — низкий.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub enum Priority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("недопустимый приоритет {0}, ожидается 1, 2 или 3")]
    Priority(i64),
    #[error("недопустимый статус задачи «{0}»")]
    Status(String),
}

impl TryFrom<i64> for Priority {
    type Error = ParseError;
    fn try_from(v: i64) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(Self::High),
            2 => Ok(Self::Medium),
            3 => Ok(Self::Low),
            other => Err(ParseError::Priority(other)),
        }
    }
}

impl From<Priority> for i64 {
    fn from(p: Priority) -> Self {
        match p {
            Priority::High => 1,
            Priority::Medium => 2,
            Priority::Low => 3,
        }
    }
}

/// Бизнес-статус задачи. Архив и просрочка статусами не являются.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    New,
    InProgress,
    Completed,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "new" => Ok(Self::New),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            other => Err(ParseError::Status(other.to_owned())),
        }
    }
}

/// Индикатор дедлайна — вычисляется, в БД не хранится.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadlineState {
    Ok,
    DueSoon,
    Overdue,
    Done,
}

pub fn deadline_state(
    status: TaskStatus,
    deadline: DateTime<Utc>,
    now: DateTime<Utc>,
    warn_before: Duration,
) -> DeadlineState {
    if status == TaskStatus::Completed {
        DeadlineState::Done
    } else if now > deadline {
        DeadlineState::Overdue
    } else if deadline - now < warn_before {
        DeadlineState::DueSoon
    } else {
        DeadlineState::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, h, 0, 0).unwrap()
    }

    #[test]
    fn priority_parses_only_1_to_3() {
        assert_eq!(Priority::try_from(1), Ok(Priority::High));
        assert_eq!(Priority::try_from(3), Ok(Priority::Low));
        assert_eq!(Priority::try_from(0), Err(ParseError::Priority(0)));
        assert_eq!(Priority::try_from(4), Err(ParseError::Priority(4)));
        assert_eq!(i64::from(Priority::Medium), 2);
    }

    #[test]
    fn status_round_trips_through_text() {
        for s in [
            TaskStatus::New,
            TaskStatus::InProgress,
            TaskStatus::Completed,
        ] {
            assert_eq!(s.as_str().parse::<TaskStatus>(), Ok(s));
        }
        assert!("archived".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn deadline_state_by_time_left() {
        let warn = Duration::hours(24);
        let deadline = at(12);
        let st = TaskStatus::InProgress;
        assert_eq!(
            deadline_state(st, deadline, at(12) - Duration::hours(25), warn),
            DeadlineState::Ok
        );
        assert_eq!(
            deadline_state(st, deadline, at(12) - Duration::hours(24), warn),
            DeadlineState::Ok
        );
        assert_eq!(
            deadline_state(st, deadline, at(11), warn),
            DeadlineState::DueSoon
        );
        assert_eq!(
            deadline_state(st, deadline, at(12), warn),
            DeadlineState::DueSoon
        );
        assert_eq!(
            deadline_state(st, deadline, at(13), warn),
            DeadlineState::Overdue
        );
    }

    #[test]
    fn completed_task_is_done_even_after_deadline() {
        let s = deadline_state(TaskStatus::Completed, at(10), at(20), Duration::hours(24));
        assert_eq!(s, DeadlineState::Done);
    }
}
