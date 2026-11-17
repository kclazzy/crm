//! Факты для правил доступа: сервер загружает их из БД, решает `crm_core::access`.

use std::collections::{HashMap, HashSet};

use crm_core::access::{client_access, task_access, Actor, ClientAccess, TaskAccess, TaskNode};
use sqlx::SqlitePool;

use crate::auth::CurrentUser;
use crate::http::ApiError;

/// Права одного пользователя на все клиенты и задачи — снимок на время запроса.
pub struct Scope {
    pub actor: Actor,
    tasks: HashMap<String, TaskAccess>,
    task_client: HashMap<String, String>,
    /// Клиенты, у которых есть хотя бы одна читаемая задача пользователя.
    working_on: HashSet<String>,
    client_group: HashMap<String, String>,
}

/// Задача: id, родитель, клиент, группа клиента, назначена ли пользователю.
type Row = (String, Option<String>, String, String, bool);

/// Кто спрашивает: роль и группы.
async fn actor(
    pool: &SqlitePool,
    user_id: &str,
    role: crm_core::auth::Role,
) -> Result<Actor, ApiError> {
    let groups: Vec<String> =
        sqlx::query_scalar("SELECT group_id FROM group_members WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(Actor {
        id: user_id.to_owned(),
        role,
        groups: groups.into_iter().collect(),
    })
}

impl Scope {
    pub async fn load(pool: &SqlitePool, user: &CurrentUser) -> Result<Self, ApiError> {
        Self::load_for(pool, &user.id, user.role).await
    }

    /// Права на все клиенты и задачи — для списков и поиска.
    pub async fn load_for(
        pool: &SqlitePool,
        user_id: &str,
        role: crm_core::auth::Role,
    ) -> Result<Self, ApiError> {
        let clients: Vec<(String, String)> =
            sqlx::query_as("SELECT id, group_id FROM clients WHERE deleted_at IS NULL")
                .fetch_all(pool)
                .await?;
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT t.id, t.parent_task_id, t.client_id, c.group_id,
                    EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = t.id AND a.user_id = ?)
             FROM tasks t JOIN clients c ON c.id = t.client_id
             WHERE t.deleted_at IS NULL AND c.deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
        Self::build(actor(pool, user_id, role).await?, clients, rows)
    }

    /// Права на одну задачу (карточка, чат, этапы, статус) — по цепочке «задача →
    /// родители»: читать и управлять задачей дают роль, группа клиента и назначение
    /// на неё или на родителя. Что задача у других — здесь неизвестно.
    pub async fn for_task(
        pool: &SqlitePool,
        user_id: &str,
        role: crm_core::auth::Role,
        task_id: &str,
    ) -> Result<Self, ApiError> {
        let rows: Vec<Row> = sqlx::query_as(
            "WITH RECURSIVE up(id, parent, client, grp, depth) AS (
                 SELECT t.id, t.parent_task_id, t.client_id, c.group_id, 0
                   FROM tasks t JOIN clients c ON c.id = t.client_id
                  WHERE t.id = ? AND t.deleted_at IS NULL AND c.deleted_at IS NULL
                 UNION ALL
                 SELECT t.id, t.parent_task_id, t.client_id, c.group_id, up.depth + 1
                   FROM up JOIN tasks t ON t.id = up.parent JOIN clients c ON c.id = t.client_id
                  WHERE t.deleted_at IS NULL AND c.deleted_at IS NULL AND up.depth < 64)
             SELECT id, parent, client, grp,
                    EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = up.id AND a.user_id = ?)
               FROM up",
        )
        .bind(task_id)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
        Self::build(actor(pool, user_id, role).await?, Vec::new(), rows)
    }

    /// Права на одного клиента и все его задачи (карточка клиента, новая задача).
    pub async fn for_client(
        pool: &SqlitePool,
        user_id: &str,
        role: crm_core::auth::Role,
        client_id: &str,
    ) -> Result<Self, ApiError> {
        let clients: Vec<(String, String)> =
            sqlx::query_as("SELECT id, group_id FROM clients WHERE id = ? AND deleted_at IS NULL")
                .bind(client_id)
                .fetch_all(pool)
                .await?;
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT t.id, t.parent_task_id, t.client_id, c.group_id,
                    EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = t.id AND a.user_id = ?)
             FROM tasks t JOIN clients c ON c.id = t.client_id
             WHERE t.client_id = ? AND t.deleted_at IS NULL AND c.deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(client_id)
        .fetch_all(pool)
        .await?;
        Self::build(actor(pool, user_id, role).await?, clients, rows)
    }

    /// Правила `crm_core::access` над загруженными фактами — одно место для всех видов.
    fn build(
        actor: Actor,
        clients: Vec<(String, String)>,
        rows: Vec<Row>,
    ) -> Result<Self, ApiError> {
        let mut task_client = HashMap::new();
        let nodes: Vec<TaskNode> = rows
            .into_iter()
            .map(|(id, parent, client, group, mine)| {
                task_client.insert(id.clone(), client);
                TaskNode {
                    id,
                    parent,
                    group,
                    assigned_to_actor: mine,
                }
            })
            .collect();
        let tasks = task_access(&actor, &nodes);
        let working_on = tasks
            .iter()
            .filter(|(_, a)| a.can_read())
            .filter_map(|(t, _)| task_client.get(t).cloned())
            .collect();
        Ok(Self {
            tasks,
            working_on,
            actor,
            task_client,
            client_group: clients.into_iter().collect(),
        })
    }

    /// Клиент задачи (для задач, о которых известно).
    pub fn client_of(&self, task_id: &str) -> Option<&str> {
        self.task_client.get(task_id).map(String::as_str)
    }

    pub fn task(&self, id: &str) -> TaskAccess {
        self.tasks.get(id).copied().unwrap_or(TaskAccess::None)
    }

    pub fn client(&self, id: &str) -> ClientAccess {
        let Some(group) = self.client_group.get(id) else {
            return ClientAccess::None;
        };
        client_access(&self.actor, group, self.working_on.contains(id))
    }

    /// Клиент существует и пользователь его хотя бы видит.
    pub fn client_or_404(&self, id: &str) -> Result<ClientAccess, ApiError> {
        match self.client(id) {
            ClientAccess::None => Err(ApiError::NotFound),
            a => Ok(a),
        }
    }

    /// Задача видна; иначе 404 — чтобы не подсказывать, что она существует.
    pub fn task_or_404(&self, id: &str) -> Result<TaskAccess, ApiError> {
        let a = self.task(id);
        if a.can_read() {
            Ok(a)
        } else {
            Err(ApiError::NotFound)
        }
    }

    pub fn require_manage_task(&self, id: &str) -> Result<(), ApiError> {
        if self.task_or_404(id)?.can_manage() {
            Ok(())
        } else {
            Err(ApiError::Forbidden)
        }
    }

    /// Видимые (не только путь) задачи.
    pub fn readable_tasks(&self) -> HashSet<&str> {
        self.tasks
            .iter()
            .filter(|(_, a)| a.can_read())
            .map(|(id, _)| id.as_str())
            .collect()
    }

    pub fn visible_clients(&self) -> Vec<(&str, ClientAccess)> {
        self.client_group
            .keys()
            .map(|id| (id.as_str(), self.client(id)))
            .filter(|(_, a)| *a != ClientAccess::None)
            .collect()
    }
}

/// Кто из пользователей видит задачу — им уходят события чата и счётчики.
///
/// Читать задачу дают роль, группа её клиента и назначение на неё саму или на
/// родителя; подзадачи дают лишь «путь». Поэтому правила (`crm_core::access`)
/// проверяются на цепочке «задача → родители», а не на всей базе: рассылка не
/// замедляется с ростом числа задач и сотрудников.
pub async fn audience(pool: &SqlitePool, task_id: &str) -> Result<Vec<String>, ApiError> {
    // Задача и её родители — как их видит `Scope`: не в корзине, клиент не в корзине.
    let chain: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "WITH RECURSIVE up(id, parent, grp, depth) AS (
             SELECT t.id, t.parent_task_id, c.group_id, 0
               FROM tasks t JOIN clients c ON c.id = t.client_id
              WHERE t.id = ? AND t.deleted_at IS NULL AND c.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.parent_task_id, c.group_id, up.depth + 1
               FROM up JOIN tasks t ON t.id = up.parent JOIN clients c ON c.id = t.client_id
              WHERE t.deleted_at IS NULL AND c.deleted_at IS NULL AND up.depth < 64)
         SELECT id, parent, grp FROM up ORDER BY depth",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    if chain.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<&str> = chain.iter().map(|(id, _, _)| id.as_str()).collect();
    let in_chain = serde_json::to_string(&ids).map_err(|e| ApiError::Failed(e.to_string()))?;
    let assigned: HashSet<(String, String)> = sqlx::query_as(
        "SELECT task_id, user_id FROM task_assignees WHERE task_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&in_chain)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    let users: Vec<(String, String)> =
        sqlx::query_as("SELECT id, role FROM users WHERE is_active = 1")
            .fetch_all(pool)
            .await?;
    let mut groups: HashMap<String, HashSet<String>> = HashMap::new();
    for (user, group) in
        sqlx::query_as::<_, (String, String)>("SELECT user_id, group_id FROM group_members")
            .fetch_all(pool)
            .await?
    {
        groups.entry(user).or_default().insert(group);
    }

    let mut out = Vec::new();
    for (id, role) in users {
        let actor = Actor {
            role: role
                .parse()
                .map_err(|_| ApiError::Corrupt(format!("роль «{role}»")))?,
            groups: groups.remove(&id).unwrap_or_default(),
            id,
        };
        let nodes: Vec<TaskNode> = chain
            .iter()
            .map(|(t, parent, group)| TaskNode {
                id: t.clone(),
                parent: parent.clone(),
                group: group.clone(),
                assigned_to_actor: assigned.contains(&(t.clone(), actor.id.clone())),
            })
            .collect();
        if task_access(&actor, &nodes)
            .get(task_id)
            .is_some_and(|a| a.can_read())
        {
            out.push(actor.id);
        }
    }
    Ok(out)
}
