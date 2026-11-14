//! Права доступа (ТЗ §3.3): единственное место, где решается, кто что видит и меняет.
//!
//! Сервер собирает факты (роль, группы, дерево задач, назначения) и спрашивает здесь.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::auth::Role;

/// Кто спрашивает.
#[derive(Debug, Clone)]
pub struct Actor {
    pub id: String,
    pub role: Role,
    /// Группы, где состоит пользователь.
    pub groups: HashSet<String>,
}

/// Что пользователь может с задачей. Упорядочено по возрастанию прав.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAccess {
    /// Не видит вовсе.
    None,
    /// Видит только название — как звено пути к своей подзадаче.
    PathOnly,
    /// Исполнитель: видит, меняет статус, отмечает этапы, пишет в чат.
    Participant,
    /// Владелец или менеджер группы: всё, включая дедлайны, исполнителей, подзадачи.
    Manage,
}

impl TaskAccess {
    pub fn can_read(self) -> bool {
        self >= Self::Participant
    }
    pub fn can_work(self) -> bool {
        self >= Self::Participant
    }
    pub fn can_manage(self) -> bool {
        self == Self::Manage
    }
}

/// Задача глазами правил доступа.
#[derive(Debug, Clone)]
pub struct TaskNode {
    pub id: String,
    pub parent: Option<String>,
    /// Группа клиента задачи.
    pub group: String,
    /// Спрашивающий — исполнитель этой задачи.
    pub assigned_to_actor: bool,
}

/// Права на каждую задачу из `nodes`.
///
/// Исполнитель задачи работает и со всеми её подзадачами; над своей задачей видит
/// только названия родителей. Менеджер управляет задачами клиентов своих групп.
pub fn task_access(actor: &Actor, nodes: &[TaskNode]) -> HashMap<String, TaskAccess> {
    let by_id: HashMap<&str, &TaskNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let ancestors = |n: &TaskNode| {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = n.parent.as_deref();
        while let Some(id) = cur {
            // Защита от цикла в повреждённых данных.
            if !seen.insert(id) {
                break;
            }
            match by_id.get(id) {
                Some(p) => {
                    out.push(*p);
                    cur = p.parent.as_deref();
                }
                None => break,
            }
        }
        out
    };

    let mut access: HashMap<String, TaskAccess> = HashMap::new();
    for n in nodes {
        let base = if actor.role == Role::Host
            || (actor.role == Role::Manager && actor.groups.contains(&n.group))
        {
            TaskAccess::Manage
        } else if n.assigned_to_actor || ancestors(n).iter().any(|a| a.assigned_to_actor) {
            TaskAccess::Participant
        } else {
            TaskAccess::None
        };
        raise(&mut access, &n.id, base);
        if base >= TaskAccess::Participant {
            for a in ancestors(n) {
                raise(&mut access, &a.id, TaskAccess::PathOnly);
            }
        }
    }
    access
}

fn raise(map: &mut HashMap<String, TaskAccess>, id: &str, to: TaskAccess) {
    let e = map.entry(id.to_owned()).or_insert(TaskAccess::None);
    if to > *e {
        *e = to;
    }
}

/// Что пользователь может с клиентом.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAccess {
    None,
    /// Видит карточку (только чтение) — потому что работает над его задачей.
    View,
    /// Редактирует карточку, заводит задачи.
    Manage,
}

/// `has_visible_task` — у пользователя есть доступная (не только путь) задача этого клиента.
pub fn client_access(actor: &Actor, group: &str, has_visible_task: bool) -> ClientAccess {
    if actor.role == Role::Host || (actor.role == Role::Manager && actor.groups.contains(group)) {
        ClientAccess::Manage
    } else if has_visible_task {
        ClientAccess::View
    } else {
        ClientAccess::None
    }
}

/// Может ли заводить клиентов в этой группе.
pub fn can_create_client(actor: &Actor, group: &str) -> bool {
    client_access(actor, group, false) == ClientAccess::Manage
}

/// Удаление в корзину: владелец — всё, менеджер — созданное им в своих группах.
pub fn can_trash(actor: &Actor, access: ClientAccess, created_by: &str) -> bool {
    match actor.role {
        Role::Host => true,
        Role::Manager => access == ClientAccess::Manage && created_by == actor.id,
        Role::Employee => false,
    }
}

/// Окончательное удаление и очистка корзины — только владелец.
pub fn can_purge(actor: &Actor) -> bool {
    actor.role == Role::Host
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(role: Role, groups: &[&str]) -> Actor {
        Actor {
            id: "me".into(),
            role,
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
        }
    }

    fn node(id: &str, parent: Option<&str>, group: &str, mine: bool) -> TaskNode {
        TaskNode {
            id: id.into(),
            parent: parent.map(Into::into),
            group: group.into(),
            assigned_to_actor: mine,
        }
    }

    /// root → mid → leaf; другой корень other.
    fn tree(mine: &str) -> Vec<TaskNode> {
        vec![
            node("root", None, "g1", mine == "root"),
            node("mid", Some("root"), "g1", mine == "mid"),
            node("leaf", Some("mid"), "g1", mine == "leaf"),
            node("other", None, "g1", false),
        ]
    }

    fn get(m: &HashMap<String, TaskAccess>, id: &str) -> TaskAccess {
        m.get(id).copied().unwrap_or(TaskAccess::None)
    }

    #[test]
    fn host_manages_everything() {
        let m = task_access(&actor(Role::Host, &[]), &tree(""));
        for id in ["root", "mid", "leaf", "other"] {
            assert_eq!(get(&m, id), TaskAccess::Manage);
        }
    }

    #[test]
    fn manager_manages_only_own_groups() {
        let m = task_access(&actor(Role::Manager, &["g1"]), &tree(""));
        assert_eq!(get(&m, "leaf"), TaskAccess::Manage);
        let m = task_access(&actor(Role::Manager, &["g2"]), &tree(""));
        assert_eq!(get(&m, "leaf"), TaskAccess::None);
    }

    #[test]
    fn manager_outside_group_still_works_on_assigned_task() {
        let m = task_access(&actor(Role::Manager, &["g2"]), &tree("mid"));
        assert_eq!(get(&m, "mid"), TaskAccess::Participant);
        assert_eq!(get(&m, "other"), TaskAccess::None);
    }

    #[test]
    fn employee_sees_assigned_subtree_and_only_path_above() {
        let m = task_access(&actor(Role::Employee, &["g1"]), &tree("mid"));
        assert_eq!(get(&m, "root"), TaskAccess::PathOnly);
        assert_eq!(get(&m, "mid"), TaskAccess::Participant);
        assert_eq!(get(&m, "leaf"), TaskAccess::Participant);
        assert_eq!(get(&m, "other"), TaskAccess::None);
        // группа сотрудника прав на чужие задачи не даёт
        assert!(!get(&m, "root").can_read());
    }

    #[test]
    fn unassigned_employee_sees_nothing() {
        let m = task_access(&actor(Role::Employee, &["g1"]), &tree(""));
        assert!(m.values().all(|a| *a == TaskAccess::None));
    }

    #[test]
    fn cycle_in_data_does_not_hang() {
        let nodes = vec![
            node("a", Some("b"), "g", true),
            node("b", Some("a"), "g", false),
        ];
        let m = task_access(&actor(Role::Employee, &[]), &nodes);
        assert_eq!(get(&m, "a"), TaskAccess::Participant);
        assert_eq!(get(&m, "b"), TaskAccess::Participant);
    }

    #[test]
    fn client_access_rules() {
        let emp = actor(Role::Employee, &["g1"]);
        assert_eq!(client_access(&emp, "g1", false), ClientAccess::None);
        assert_eq!(client_access(&emp, "g1", true), ClientAccess::View);
        let mgr = actor(Role::Manager, &["g1"]);
        assert_eq!(client_access(&mgr, "g1", false), ClientAccess::Manage);
        assert_eq!(client_access(&mgr, "g2", true), ClientAccess::View);
        assert!(can_create_client(&mgr, "g1"));
        assert!(!can_create_client(&mgr, "g2"));
        assert!(!can_create_client(&emp, "g1"));
    }

    #[test]
    fn trash_and_purge_rules() {
        let mgr = actor(Role::Manager, &["g1"]);
        assert!(can_trash(&mgr, ClientAccess::Manage, "me"));
        assert!(!can_trash(&mgr, ClientAccess::Manage, "someone"));
        assert!(!can_trash(&mgr, ClientAccess::View, "me"));
        let host = actor(Role::Host, &[]);
        assert!(can_trash(&host, ClientAccess::Manage, "someone"));
        assert!(can_purge(&host));
        assert!(!can_purge(&mgr));
    }
}
