//! Расписание напоминаний о дедлайне с учётом тихих часов (ТЗ §4.5).

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::schedule::WorkSchedule;
use crate::task::Priority;

/// Вид напоминания. Порядок вариантов — по возрастанию срочности.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    H24,
    H4,
    H1,
    Overdue,
}

impl ReminderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::H24 => "h24",
            Self::H4 => "h4",
            Self::H1 => "h1",
            Self::Overdue => "overdue",
        }
    }

    /// За сколько до дедлайна напомнить.
    fn lead(self) -> Duration {
        match self {
            Self::H24 => Duration::hours(24),
            Self::H4 => Duration::hours(4),
            Self::H1 => Duration::hours(1),
            Self::Overdue => Duration::zero(),
        }
    }
}

/// Какие напоминания положены задаче с данным приоритетом.
pub fn kinds_for(p: Priority) -> &'static [ReminderKind] {
    use ReminderKind::*;
    match p {
        Priority::High => &[H24, H4, H1, Overdue],
        Priority::Medium | Priority::Low => &[H24, Overdue],
    }
}

/// За сколько до конца рабочего окна присылать напоминание, которое не может ждать утра.
const LAST_WORKING_HOUR: Duration = Duration::hours(1);

/// Когда реально доставить напоминание, запланированное на `planned`.
fn delivery_time(
    kind: ReminderKind,
    planned: DateTime<Utc>,
    deadline: DateTime<Utc>,
    s: &WorkSchedule,
) -> DateTime<Utc> {
    let Some(next) = s.next_work_start(planned) else {
        return planned;
    };
    if kind == ReminderKind::Overdue || next < deadline {
        return next;
    }
    // Утро наступит уже после дедлайна — предупреждаем в последний рабочий час перед ним.
    match s.prev_work_end(planned) {
        Some((start, end)) => (end - LAST_WORKING_HOUR).max(start),
        None => planned,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedReminder {
    pub kind: ReminderKind,
    pub at: DateTime<Utc>,
}

/// План напоминаний по задаче. `schedule = None` — тихие часы не учитываются
/// (пользователь включил «уведомлять всегда»).
pub fn plan(
    priority: Priority,
    deadline: DateTime<Utc>,
    schedule: Option<&WorkSchedule>,
) -> Vec<PlannedReminder> {
    let mut out: Vec<PlannedReminder> = kinds_for(priority)
        .iter()
        .map(|&kind| {
            let planned = deadline - kind.lead();
            let at = schedule.map_or(planned, |s| delivery_time(kind, planned, deadline, s));
            PlannedReminder { kind, at }
        })
        .collect();
    // Совпавшие по времени — оставляем самое срочное.
    out.sort_by(|a, b| a.at.cmp(&b.at).then(b.kind.cmp(&a.kind)));
    out.dedup_by_key(|r| r.at);
    out
}

/// Что отправить сейчас: одно, самое срочное напоминание; остальные наступившие
/// помечаются отправленными, чтобы не прийти пачкой.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub send: ReminderKind,
    pub mark_sent: Vec<ReminderKind>,
}

pub fn due(
    plan: &[PlannedReminder],
    now: DateTime<Utc>,
    already_sent: &[ReminderKind],
) -> Option<Due> {
    let ready: Vec<ReminderKind> = plan
        .iter()
        .filter(|r| r.at <= now && !already_sent.contains(&r.kind))
        .map(|r| r.kind)
        .collect();
    let send = ready.iter().copied().max()?;
    Some(Due {
        send,
        mark_sent: ready,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::tests::{msk, office};
    use ReminderKind::*;

    fn p(kind: ReminderKind, at: DateTime<Utc>) -> PlannedReminder {
        PlannedReminder { kind, at }
    }

    #[test]
    fn high_priority_gets_three_reminders_others_one() {
        assert_eq!(kinds_for(Priority::High), &[H24, H4, H1, Overdue]);
        assert_eq!(kinds_for(Priority::Medium), &[H24, Overdue]);
        assert_eq!(kinds_for(Priority::Low), &[H24, Overdue]);
    }

    #[test]
    fn without_quiet_hours_reminders_follow_deadline() {
        let d = msk(16, 10, 0);
        assert_eq!(
            plan(Priority::High, d, None),
            vec![
                p(H24, msk(15, 10, 0)),
                p(H4, msk(16, 6, 0)),
                p(H1, msk(16, 9, 0)),
                p(Overdue, d)
            ]
        );
    }

    #[test]
    fn night_and_weekend_reminders_move_to_morning_and_merge() {
        // Дедлайн пн 16.11 10:00: вс 10:00 и пн 06:00 переносятся на пн 09:00,
        // где уже стоит H1 — остаётся одно, самое срочное.
        let s = office();
        assert_eq!(
            plan(Priority::High, msk(16, 10, 0), Some(&s)),
            vec![p(H1, msk(16, 9, 0)), p(Overdue, msk(16, 10, 0))]
        );
    }

    #[test]
    fn deadline_on_weekend_reminds_in_last_working_hour_before() {
        // Дедлайн сб 14.11 10:00: H4 (сб 06:00) и H1 (сб 09:00) не могут ждать до понедельника —
        // приходят в пятницу в последний рабочий час (18:00). Просрочка — в пн утром.
        let s = office();
        assert_eq!(
            plan(Priority::High, msk(14, 10, 0), Some(&s)),
            vec![
                p(H24, msk(13, 10, 0)),
                p(H1, msk(13, 18, 0)),
                p(Overdue, msk(16, 9, 0))
            ]
        );
    }

    #[test]
    fn due_sends_only_most_urgent_of_missed() {
        // Задачу создали за 2 ч до дедлайна: H24 и H4 уже наступили — приходит одно H4.
        let d = msk(13, 15, 0);
        let pl = plan(Priority::High, d, None);
        assert_eq!(
            due(&pl, msk(12, 14, 0), &[]),
            None,
            "раньше чем за 24 ч — рано"
        );
        assert_eq!(
            due(&pl, msk(13, 13, 0), &[]),
            Some(Due {
                send: H4,
                mark_sent: vec![H24, H4]
            })
        );
        assert_eq!(due(&pl, msk(13, 13, 30), &[H24, H4]), None, "повтора нет");
        assert_eq!(
            due(&pl, msk(13, 14, 0), &[H24, H4]),
            Some(Due {
                send: H1,
                mark_sent: vec![H1]
            })
        );
        assert_eq!(
            due(&pl, msk(13, 18, 0), &[H24, H4, H1]),
            Some(Due {
                send: Overdue,
                mark_sent: vec![Overdue]
            })
        );
    }
}
