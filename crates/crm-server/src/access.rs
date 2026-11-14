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

impl Scope {
    pub async fn load(pool: &SqlitePool, user: &CurrentUser) -> Result<Self, ApiError> {
        Self::load_for(pool, &user.id, user.role).await
    }

    pub async fn load_for(
        pool: &SqlitePool,
        user_id: &str,
        role: crm_core::auth::Role,
    ) -> Result<Self, ApiError> {
        let groups: Vec<String> =
            sqlx::query_scalar("SELECT group_id FROM group_members WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(pool)
                .await?;
        let actor = Actor {
            id: user_id.to_owned(),
            role,
            groups: groups.into_iter().collect(),
        };
        let clients: Vec<(String, String)> =
            sqlx::query_as("SELECT id, group_id FROM clients WHERE deleted_at IS NULL")
                .fetch_all(pool)
                .await?;
        let rows: Vec<(String, Option<String>, String, String, bool)> = sqlx::query_as(
            "SELECT t.id, t.parent_task_id, t.client_id, c.group_id,
                    EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = t.id AND a.user_id = ?)
             FROM tasks t JOIN clients c ON c.id = t.client_id
             WHERE t.deleted_at IS NULL AND c.deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
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
pub async fn audience(pool: &SqlitePool, task_id: &str) -> Result<Vec<String>, ApiError> {
    let users: Vec<(String, String)> =
        sqlx::query_as("SELECT id, role FROM users WHERE is_active = 1")
            .fetch_all(pool)
            .await?;
    let mut out = Vec::new();
    for (id, role) in users {
        let role = role
            .parse()
            .map_err(|_| ApiError::Corrupt(format!("роль «{role}»")))?;
        if Scope::load_for(pool, &id, role)
            .await?
            .task(task_id)
            .can_read()
        {
            out.push(id);
        }
    }
    Ok(out)
}
