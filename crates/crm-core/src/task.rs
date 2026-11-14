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

/// Нарушение правил дерева задач (ТЗ §4.2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuleError {
    #[error("дедлайн подзадачи не может быть позже дедлайна родительской задачи")]
    DeadlineAfterParent,
    #[error("нельзя завершить задачу: открыто подзадач — {0}")]
    OpenSubtasks(usize),
    #[error("название не может быть пустым и длиннее 300 символов")]
    BadTitle,
}

/// Дедлайн подзадачи ≤ дедлайна родителя.
pub fn check_subtask_deadline(child: i64, parent: Option<i64>) -> Result<(), RuleError> {
    match parent {
        Some(p) if child > p => Err(RuleError::DeadlineAfterParent),
        _ => Ok(()),
    }
}

/// Завершить можно, только когда все подзадачи завершены (этапы не блокируют).
pub fn check_can_complete(to: TaskStatus, open_subtasks: usize) -> Result<(), RuleError> {
    if to == TaskStatus::Completed && open_subtasks > 0 {
        Err(RuleError::OpenSubtasks(open_subtasks))
    } else {
        Ok(())
    }
}

/// Название задачи, этапа или клиента: без пробелов по краям, 1–300 символов.
pub fn parse_title(s: &str) -> Result<String, RuleError> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > 300 {
        Err(RuleError::BadTitle)
    } else {
        Ok(t.to_owned())
    }
}

/// `completed_at` при смене статуса: завершение ставит время, переоткрытие — сбрасывает.
pub fn completed_at_after(to: TaskStatus, before: Option<i64>, now: i64) -> Option<i64> {
    match to {
        TaskStatus::Completed => Some(before.unwrap_or(now)),
        _ => None,
    }
}

/// Задача глазами автоархива.
#[derive(Debug, Clone)]
pub struct ArchiveNode {
    pub id: String,
    pub parent: Option<String>,
    pub completed_at: Option<i64>,
}

/// Что отправить в архив (ТЗ §4.4): завершённая давнее `older_than` задача, у которой
/// всё поддерево завершено, уходит в архив целиком — вместе с поддеревом.
pub fn archive_candidates(nodes: &[ArchiveNode], older_than: i64) -> Vec<String> {
    use std::collections::{HashMap, HashSet};
    let mut children: HashMap<&str, Vec<&ArchiveNode>> = HashMap::new();
    for n in nodes {
        if let Some(p) = &n.parent {
            children.entry(p.as_str()).or_default().push(n);
        }
    }
    fn subtree<'a>(
        n: &'a ArchiveNode,
        children: &HashMap<&str, Vec<&'a ArchiveNode>>,
        seen: &mut HashSet<&'a str>,
        out: &mut Vec<&'a ArchiveNode>,
    ) {
        if !seen.insert(n.id.as_str()) {
            return;
        }
        out.push(n);
        for c in children.get(n.id.as_str()).into_iter().flatten() {
            subtree(c, children, seen, out);
        }
    }
    let mut picked: HashSet<&str> = HashSet::new();
    for n in nodes {
        if !n.completed_at.is_some_and(|at| at < older_than) {
            continue;
        }
        let mut all = Vec::new();
        subtree(n, &children, &mut HashSet::new(), &mut all);
        if all.iter().all(|t| t.completed_at.is_some()) {
            picked.extend(all.iter().map(|t| t.id.as_str()));
        }
    }
    let mut out: Vec<String> = picked.into_iter().map(str::to_owned).collect();
    out.sort();
    out
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

    #[test]
    fn subtask_deadline_not_after_parent() {
        assert_eq!(check_subtask_deadline(10, Some(10)), Ok(()));
        assert_eq!(check_subtask_deadline(5, None), Ok(()));
        assert_eq!(
            check_subtask_deadline(11, Some(10)),
            Err(RuleError::DeadlineAfterParent)
        );
    }

    #[test]
    fn open_subtasks_block_completion_only() {
        assert_eq!(
            check_can_complete(TaskStatus::Completed, 2),
            Err(RuleError::OpenSubtasks(2))
        );
        assert_eq!(check_can_complete(TaskStatus::Completed, 0), Ok(()));
        assert_eq!(check_can_complete(TaskStatus::InProgress, 3), Ok(()));
    }

    #[test]
    fn reopening_resets_completed_at() {
        assert_eq!(completed_at_after(TaskStatus::Completed, None, 7), Some(7));
        assert_eq!(
            completed_at_after(TaskStatus::Completed, Some(3), 7),
            Some(3)
        );
        assert_eq!(completed_at_after(TaskStatus::InProgress, Some(3), 7), None);
    }

    #[test]
    fn titles_are_trimmed_and_bounded() {
        assert_eq!(parse_title("  Договор "), Ok("Договор".into()));
        assert_eq!(parse_title("   "), Err(RuleError::BadTitle));
        assert_eq!(parse_title(&"я".repeat(301)), Err(RuleError::BadTitle));
    }

    fn an(id: &str, parent: Option<&str>, done: Option<i64>) -> ArchiveNode {
        ArchiveNode {
            id: id.into(),
            parent: parent.map(Into::into),
            completed_at: done,
        }
    }

    #[test]
    fn archive_takes_old_completed_subtrees_only() {
        let nodes = vec![
            an("old", None, Some(1)),
            an("old.child", Some("old"), Some(50)), // свежий, но родитель забирает
            an("blocked", None, Some(1)),
            an("blocked.open", Some("blocked"), None),
            an("fresh", None, Some(90)),
            an("open", None, None),
        ];
        assert_eq!(archive_candidates(&nodes, 10), ["old", "old.child"]);
    }

    #[test]
    fn archive_survives_cycles() {
        let nodes = vec![an("a", Some("b"), Some(1)), an("b", Some("a"), Some(1))];
        assert_eq!(archive_candidates(&nodes, 10), ["a", "b"]);
    }
}
