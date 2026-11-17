//! Задачи: дерево, этапы, исполнители, статусы и дедлайны (ТЗ §4.2–4.3).

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, TimeZone, Utc};
use crm_core::access::{ClientAccess, TaskAccess};
use crm_core::auth::Role;
use crm_core::task::{
    check_archive, check_can_complete, check_edit, check_reassign, check_reopen,
    check_status_change, check_subtask_deadline, check_transition, completed_at_after,
    deadline_state, parse_title, DeadlineState, Involvement, Priority, TaskStatus,
};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::access::Scope;
use crate::auth::{new_id, now_ms, CurrentUser};
use crate::chat::{system, unread_by_task};
use crate::events::task_changed;
use crate::http::{ApiError, AppState};

impl From<crm_core::task::RuleError> for ApiError {
    fn from(e: crm_core::task::RuleError) -> Self {
        Self::BadRequest(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub done: i64,
    pub total: i64,
}

/// Подробности, которые видит только участник задачи (не «только путь»).
#[derive(Debug, Clone, Serialize)]
pub struct Details {
    pub status: TaskStatus,
    pub priority: Priority,
    pub deadline_at: i64,
    pub deadline_state: DeadlineState,
    pub completed_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub created_at: i64,
    /// Первым в `assignees`; ведёт задачу. Остальные — соисполнители.
    pub assignees: Vec<PersonRef>,
    pub lead_id: Option<String>,
    pub progress: Progress,
    pub unread: i64,
    pub children: usize,
    pub last_message_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub access: TaskAccess,
    #[serde(flatten)]
    pub details: Option<Details>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    client_id: String,
    client_name: String,
    parent_task_id: Option<String>,
    title: String,
    priority: i64,
    status: String,
    deadline_at: i64,
    completed_at: Option<i64>,
    archived_at: Option<i64>,
    created_at: i64,
    stages_total: i64,
    stages_done: i64,
    last_message_at: Option<i64>,
}

fn ms(t: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(t).single().unwrap_or_default()
}

/// Все задачи, которые пользователь видит хотя бы как путь, — со сводкой.
pub async fn summaries(
    app: &AppState,
    scope: &Scope,
    me: &str,
) -> Result<Vec<TaskSummary>, ApiError> {
    let org = crate::org::load(&app.pool).await?;
    let warn = Duration::hours(org.warn_hours());
    let now = Utc::now();
    let only = scope.visible_json();
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT t.id, t.client_id, c.name AS client_name, t.parent_task_id, t.title, t.priority,
                t.status, t.deadline_at, t.completed_at, t.archived_at, t.created_at,
                (SELECT count(*) FROM task_stages s WHERE s.task_id = t.id) AS stages_total,
                (SELECT count(*) FROM task_stages s WHERE s.task_id = t.id AND s.completed_at IS NOT NULL) AS stages_done,
                (SELECT max(m.created_at) FROM messages m WHERE m.task_id = t.id) AS last_message_at
         FROM tasks t JOIN clients c ON c.id = t.client_id
         WHERE t.deleted_at IS NULL AND c.deleted_at IS NULL
           AND (?1 IS NULL OR t.id IN (SELECT value FROM json_each(?1)))
         ORDER BY t.deadline_at",
    )
    .bind(&only)
    .fetch_all(&app.pool)
    .await?;
    let mut assignees: HashMap<String, Vec<PersonRef>> = HashMap::new();
    let mut leads: HashMap<String, String> = HashMap::new();
    let pairs: Vec<(String, String, String, bool)> = sqlx::query_as(
        "SELECT a.task_id, u.id, u.display_name, a.is_lead FROM task_assignees a JOIN users u ON u.id = a.user_id
         WHERE ?1 IS NULL OR a.task_id IN (SELECT value FROM json_each(?1))
         ORDER BY a.is_lead DESC, u.display_name COLLATE NOCASE",
    )
    .bind(&only)
    .fetch_all(&app.pool)
    .await?;
    for (t, id, name, lead) in pairs {
        if lead {
            leads.insert(t.clone(), id.clone());
        }
        assignees.entry(t).or_default().push(PersonRef { id, name });
    }
    let mut children: HashMap<String, usize> = HashMap::new();
    for r in &rows {
        if let Some(p) = &r.parent_task_id {
            if scope.task(&r.id) >= TaskAccess::PathOnly {
                *children.entry(p.clone()).or_default() += 1;
            }
        }
    }
    let unread = unread_by_task(&app.pool, me, only.as_deref()).await?;
    let mut out = Vec::new();
    for r in rows {
        let access = scope.task(&r.id);
        if access == TaskAccess::None {
            continue;
        }
        let details = if access.can_read() {
            let status: TaskStatus = r
                .status
                .parse()
                .map_err(|_| ApiError::Corrupt(format!("статус «{}»", r.status)))?;
            Some(Details {
                status,
                priority: Priority::try_from(r.priority)
                    .map_err(|_| ApiError::Corrupt(format!("приоритет {}", r.priority)))?,
                deadline_at: r.deadline_at,
                deadline_state: deadline_state(status, ms(r.deadline_at), now, warn),
                completed_at: r.completed_at,
                archived_at: r.archived_at,
                created_at: r.created_at,
                assignees: assignees.remove(&r.id).unwrap_or_default(),
                lead_id: leads.remove(&r.id),
                progress: Progress {
                    done: r.stages_done,
                    total: r.stages_total,
                },
                unread: unread.get(&r.id).copied().unwrap_or(0),
                children: children.get(&r.id).copied().unwrap_or(0),
                last_message_at: r.last_message_at,
            })
        } else {
            None
        };
        out.push(TaskSummary {
            id: r.id,
            client_id: r.client_id,
            client_name: r.client_name,
            parent_id: r.parent_task_id,
            title: r.title,
            access,
            details,
        });
    }
    Ok(out)
}

// ---------- Список ----------

#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// `mine` — где я исполнитель; иначе все доступные.
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    status: Option<TaskStatus>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    priority: Option<i64>,
    /// `assignee` — я исполнитель, `creator` — я постановщик, `involved` — то или другое.
    #[serde(default)]
    role: Option<String>,
    /// Поиск по названию задачи и клиента.
    #[serde(default)]
    q: Option<String>,
    /// Только просроченные.
    #[serde(default)]
    overdue: bool,
}

pub async fn list(
    State(app): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<TaskSummary>>, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let mine = q.view.as_deref() == Some("mine");
    let created: std::collections::HashSet<String> =
        if matches!(q.role.as_deref(), Some("creator" | "involved")) {
            sqlx::query_scalar("SELECT id FROM tasks WHERE created_by = ?")
                .bind(&user.id)
                .fetch_all(&app.pool)
                .await?
                .into_iter()
                .collect()
        } else {
            Default::default()
        };
    let needle =
        q.q.as_deref()
            .map(str::to_lowercase)
            .filter(|s| !s.trim().is_empty());
    let out = summaries(&app, &scope, &user.id)
        .await?
        .into_iter()
        .filter(|t| {
            let Some(d) = &t.details else { return false };
            let is_assignee = d.assignees.iter().any(|a| a.id == user.id);
            let role_ok = match q.role.as_deref() {
                Some("assignee") => is_assignee,
                Some("creator") => created.contains(&t.id),
                // «Мои задачи»: я исполнитель или постановщик.
                Some("involved") => is_assignee || created.contains(&t.id),
                _ => true,
            };
            (d.archived_at.is_some() == q.archived)
                && (!mine || is_assignee)
                && role_ok
                && (!q.overdue || d.deadline_state == DeadlineState::Overdue)
                && q.status.is_none_or(|s| d.status == s)
                && q.priority.is_none_or(|p| i64::from(d.priority) == p)
                && q.client_id.as_ref().is_none_or(|c| *c == t.client_id)
                && needle.as_ref().is_none_or(|n| {
                    t.title.to_lowercase().contains(n.as_str())
                        || t.client_name.to_lowercase().contains(n.as_str())
                })
        })
        .collect();
    Ok(Json(out))
}

// ---------- Карточка ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Stage {
    id: String,
    title: String,
    completed_at: Option<i64>,
    completed_by: Option<String>,
    completed_by_name: Option<String>,
    sort_order: i64,
}

#[derive(Serialize)]
pub struct PathItem {
    id: String,
    title: String,
    readable: bool,
}

#[derive(Serialize)]
pub struct TaskCard {
    #[serde(flatten)]
    summary: TaskSummary,
    description: String,
    created_by: PersonRef,
    stages: Vec<Stage>,
    path: Vec<PathItem>,
    subtasks: Vec<TaskSummary>,
    parent_deadline_at: Option<i64>,
    can_trash: bool,
    /// Можно нажимать «Начать / Завершить / Отложить».
    can_change_status: bool,
    /// Можно сменить исполнителя (не в архиве).
    can_reassign: bool,
    /// Можно поменять название, описание, срок, приоритет.
    can_edit: bool,
    /// Можно убрать в архив или вернуть из него.
    can_archive: bool,
    archived_by: Option<String>,
    /// Ответственный (ведёт задачу); остальные исполнители — соисполнители.
    lead_id: Option<String>,
    /// Сотрудники группы клиента — их предлагать первыми при смене исполнителя.
    team: Vec<String>,
}

async fn stages(pool: &SqlitePool, task: &str) -> Result<Vec<Stage>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT s.id, s.title, s.completed_at, s.completed_by, u.display_name AS completed_by_name, s.sort_order
         FROM task_stages s LEFT JOIN users u ON u.id = s.completed_by
         WHERE s.task_id = ? ORDER BY s.sort_order, s.id",
    )
    .bind(task)
    .fetch_all(pool)
    .await?)
}

pub async fn get(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<TaskCard>, ApiError> {
    let scope = Scope::for_card(&app.pool, &user.id, user.role, &id).await?;
    let access = scope.task_or_404(&id)?;
    let all = summaries(&app, &scope, &user.id).await?;
    let by_id: HashMap<&str, &TaskSummary> = all.iter().map(|t| (t.id.as_str(), t)).collect();
    let summary = by_id
        .get(id.as_str())
        .map(|t| (*t).clone())
        .ok_or(ApiError::NotFound)?;
    let mut path = Vec::new();
    let mut cur = summary.parent_id.clone();
    while let Some(p) = cur {
        let Some(t) = by_id.get(p.as_str()) else {
            break;
        };
        path.push(PathItem {
            id: t.id.clone(),
            title: t.title.clone(),
            readable: t.access.can_read(),
        });
        cur = t.parent_id.clone();
        if path.len() > 100 {
            break;
        }
    }
    path.reverse();
    let parent_deadline_at = summary
        .parent_id
        .as_deref()
        .and_then(|p| by_id.get(p))
        .and_then(|p| p.details.as_ref())
        .map(|d| d.deadline_at);
    let subtasks = all
        .iter()
        .filter(|t| t.parent_id.as_deref() == Some(id.as_str()))
        .cloned()
        .collect();
    let (description, by_id_user, by_name): (Option<String>, String, String) = sqlx::query_as(
        "SELECT t.description, u.id, u.display_name FROM tasks t JOIN users u ON u.id = t.created_by WHERE t.id = ?",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await?;
    let can_trash =
        access.can_manage() && (scope.actor.role == Role::Host || by_id_user == user.id);
    let who = involvement(&app.pool, &id, &user).await?;
    let can_change_status = check_status_change(who).is_ok();
    let can_reassign = check_reassign(who, access.can_manage(), 1).is_ok();
    let can_edit = check_edit(who, access.can_manage()).is_ok();
    let can_archive = who.is_owner || who.is_assignee;
    let lead_id = summary.details.as_ref().and_then(|d| d.lead_id.clone());
    let archived_by: Option<String> = sqlx::query_scalar(
        "SELECT u.display_name FROM tasks t JOIN users u ON u.id = t.archived_by WHERE t.id = ?",
    )
    .bind(&id)
    .fetch_optional(&app.pool)
    .await?;
    Ok(Json(TaskCard {
        summary,
        description: description.unwrap_or_default(),
        created_by: PersonRef {
            id: by_id_user,
            name: by_name,
        },
        stages: stages(&app.pool, &id).await?,
        path,
        subtasks,
        parent_deadline_at,
        can_trash,
        can_change_status,
        can_reassign,
        can_edit,
        can_archive,
        archived_by,
        lead_id,
        team: sqlx::query_scalar(
            "SELECT gm.user_id FROM tasks t JOIN clients c ON c.id = t.client_id
             JOIN group_members gm ON gm.group_id = c.group_id WHERE t.id = ?",
        )
        .bind(&id)
        .fetch_all(&app.pool)
        .await?,
    }))
}

// ---------- Создание ----------

#[derive(Deserialize)]
pub struct TaskIn {
    client_id: Option<String>,
    parent_id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    priority: Priority,
    deadline_at: i64,
    #[serde(default)]
    assignees: Vec<String>,
    #[serde(default)]
    stages: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct Parent {
    client_id: String,
    deadline_at: i64,
    status: String,
    archived_at: Option<i64>,
}

async fn parent_of(pool: &SqlitePool, id: &str) -> Result<Parent, ApiError> {
    sqlx::query_as("SELECT client_id, deadline_at, status, archived_at FROM tasks WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)
}

fn check_description(d: &str) -> Result<(), ApiError> {
    if d.chars().count() > 50_000 {
        Err(ApiError::BadRequest(
            "описание: не длиннее 50 000 символов".into(),
        ))
    } else {
        Ok(())
    }
}

async fn set_assignees(
    tx: &mut Transaction<'_, Sqlite>,
    task: &str,
    users: &[String],
) -> Result<(), ApiError> {
    if users.len() > 50 {
        return Err(ApiError::BadRequest("слишком много исполнителей".into()));
    }
    sqlx::query("DELETE FROM task_assignees WHERE task_id = ?")
        .bind(task)
        .execute(&mut **tx)
        .await?;
    for (i, u) in users.iter().enumerate() {
        let n = sqlx::query(
            "INSERT OR IGNORE INTO task_assignees(task_id, user_id, is_lead) SELECT ?, id, ? FROM users WHERE id = ? AND is_active = 1",
        )
        .bind(task)
        .bind(i == 0)
        .bind(u)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if n == 0 {
            let dup: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM task_assignees WHERE task_id = ? AND user_id = ?",
            )
            .bind(task)
            .bind(u)
            .fetch_one(&mut **tx)
            .await?;
            if dup == 0 {
                return Err(ApiError::BadRequest(
                    "исполнитель не найден или отключён".into(),
                ));
            }
        }
    }
    Ok(())
}

async fn names(pool: &SqlitePool, ids: &[String]) -> Result<String, ApiError> {
    let mut out = Vec::new();
    for id in ids {
        if let Some(n) =
            sqlx::query_scalar::<_, String>("SELECT display_name FROM users WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?
        {
            out.push(n);
        }
    }
    Ok(if out.is_empty() {
        "никого".into()
    } else {
        out.join(", ")
    })
}

pub async fn create(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(req): Json<TaskIn>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let title = parse_title(&req.title)?;
    check_description(&req.description)?;
    let client_id = match &req.parent_id {
        Some(p) => {
            Scope::for_task(&app.pool, &user.id, user.role, p)
                .await?
                .require_manage_task(p)?;
            let parent = parent_of(&app.pool, p).await?;
            if parent.status == TaskStatus::Completed.as_str() || parent.archived_at.is_some() {
                return Err(ApiError::Conflict(
                    "родительская задача завершена — сначала переоткройте её",
                ));
            }
            check_subtask_deadline(req.deadline_at, Some(parent.deadline_at))?;
            parent.client_id
        }
        None => {
            let c = req
                .client_id
                .clone()
                .ok_or_else(|| ApiError::BadRequest("не указан клиент".into()))?;
            let scope = Scope::for_client(&app.pool, &user.id, user.role, &c).await?;
            if scope.client_or_404(&c)? != ClientAccess::Manage {
                return Err(ApiError::Forbidden);
            }
            c
        }
    };
    let id = new_id();
    let now = now_ms();
    let mut tx = crate::db::write_tx(&app.pool).await?;
    sqlx::query(
        "INSERT INTO tasks(id, client_id, parent_task_id, title, description, priority, status,
                           created_by, created_at, deadline_at)
         VALUES (?, ?, ?, ?, ?, ?, 'waiting', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&client_id)
    .bind(&req.parent_id)
    .bind(&title)
    .bind(&req.description)
    .bind(i64::from(req.priority))
    .bind(&user.id)
    .bind(now)
    .bind(req.deadline_at)
    .execute(&mut *tx)
    .await?;
    set_assignees(&mut tx, &id, &req.assignees).await?;
    for (i, s) in req.stages.iter().enumerate() {
        let t = s.trim();
        if t.is_empty() {
            continue;
        }
        insert_stage(&mut tx, &id, &parse_title(t)?, i as i64).await?;
    }
    system(
        &mut tx,
        &id,
        &user.id,
        &format!("{} создал задачу", user.display_name),
    )
    .await?;
    if let Some(p) = &req.parent_id {
        system(
            &mut tx,
            p,
            &user.id,
            &format!("{} добавил подзадачу «{title}»", user.display_name),
        )
        .await?;
    }
    tx.commit().await?;
    task_changed(&app, &id, "created").await?;
    if let Some(p) = &req.parent_id {
        task_changed(&app, p, "subtask").await?;
    }
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

async fn insert_stage(
    tx: &mut Transaction<'_, Sqlite>,
    task: &str,
    title: &str,
    order: i64,
) -> Result<String, ApiError> {
    let id = new_id();
    sqlx::query("INSERT INTO task_stages(id, task_id, title, sort_order) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(task)
        .bind(title)
        .bind(order)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

// ---------- Изменение ----------

#[derive(Deserialize)]
pub struct TaskPatch {
    title: Option<String>,
    description: Option<String>,
    priority: Option<Priority>,
    deadline_at: Option<i64>,
    assignees: Option<Vec<String>>,
}

#[derive(sqlx::FromRow)]
struct Current {
    title: String,
    priority: i64,
    deadline_at: i64,
    parent_task_id: Option<String>,
    status: String,
    archived_at: Option<i64>,
    completed_at: Option<i64>,
}

async fn current(pool: &SqlitePool, id: &str) -> Result<Current, ApiError> {
    sqlx::query_as(
        "SELECT title, priority, deadline_at, parent_task_id, status, archived_at, completed_at
         FROM tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub(crate) fn fmt_time(tz: chrono_tz::Tz, t: i64) -> String {
    ms(t)
        .with_timezone(&tz)
        .format("%d.%m.%Y %H:%M")
        .to_string()
}

/// «5 ноября 2026, 07:00» — как в записях чата.
pub(crate) fn fmt_long(tz: chrono_tz::Tz, t: i64) -> String {
    const MONTHS: [&str; 12] = [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ];
    use chrono::Datelike;
    let d = ms(t).with_timezone(&tz);
    let month = MONTHS.get(d.month0() as usize).copied().unwrap_or("");
    format!("{} {month} {}, {}", d.day(), d.year(), d.format("%H:%M"))
}

fn priority_label(p: i64) -> &'static str {
    match p {
        1 => "P1 высокий",
        2 => "P2 средний",
        _ => "P3 низкий",
    }
}

pub async fn update(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<TaskPatch>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &id).await?;
    let access = scope.task_or_404(&id)?;
    let who = involvement(&app.pool, &id, &user).await?;
    check_edit(who, access.can_manage()).map_err(|_| ApiError::Forbidden)?;
    // Исполнителей через «Изменить» назначает руководитель; остальные — «Сменить исполнителя».
    if req.assignees.is_some() && !access.can_manage() {
        return Err(ApiError::Forbidden);
    }
    let cur = current(&app.pool, &id).await?;
    if cur.archived_at.is_some() {
        return Err(ApiError::Conflict("задача в архиве — сначала верните её"));
    }
    let tz = crate::org::load(&app.pool).await?.tz();
    let who = &user.display_name;
    let mut tx = crate::db::write_tx(&app.pool).await?;
    if let Some(t) = &req.title {
        let t = parse_title(t)?;
        if t != cur.title {
            sqlx::query("UPDATE tasks SET title = ? WHERE id = ?")
                .bind(&t)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            system(
                &mut tx,
                &id,
                &user.id,
                &format!("{who} переименовал(а) задачу: «{t}»"),
            )
            .await?;
        }
    }
    if let Some(d) = &req.description {
        check_description(d)?;
        let before: String = sqlx::query_scalar("SELECT description FROM tasks WHERE id = ?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
        if *d != before {
            sqlx::query("UPDATE tasks SET description = ? WHERE id = ?")
                .bind(d)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            system(
                &mut tx,
                &id,
                &user.id,
                &format!("{who} изменил(а) описание задачи"),
            )
            .await?;
        }
    }
    if let Some(p) = req.priority {
        let p = i64::from(p);
        if p != cur.priority {
            sqlx::query("UPDATE tasks SET priority = ? WHERE id = ?")
                .bind(p)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            system(
                &mut tx,
                &id,
                &user.id,
                &format!(
                    "{who} изменил(а) приоритет: {} → {}",
                    priority_label(cur.priority),
                    priority_label(p)
                ),
            )
            .await?;
        }
    }
    if let Some(d) = req.deadline_at {
        if d != cur.deadline_at {
            let parent_deadline = match &cur.parent_task_id {
                Some(p) => Some(parent_of(&app.pool, p).await?.deadline_at),
                None => None,
            };
            check_subtask_deadline(d, parent_deadline)?;
            let later_children: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM tasks WHERE parent_task_id = ? AND deleted_at IS NULL AND deadline_at > ?",
            )
            .bind(&id)
            .bind(d)
            .fetch_one(&mut *tx)
            .await?;
            if later_children > 0 {
                return Err(ApiError::BadRequest(format!(
                    "у {later_children} подзадач дедлайн позже — сначала сдвиньте их"
                )));
            }
            sqlx::query("UPDATE tasks SET deadline_at = ? WHERE id = ?")
                .bind(d)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            system(
                &mut tx,
                &id,
                &user.id,
                &format!(
                    "{who} изменил(а) крайний срок задачи на {}",
                    fmt_long(tz, d)
                ),
            )
            .await?;
        }
    }
    if let Some(a) = &req.assignees {
        set_assignees(&mut tx, &id, a).await?;
        let list = names(&app.pool, a).await?;
        system(
            &mut tx,
            &id,
            &user.id,
            &format!("{who} назначил исполнителей: {list}"),
        )
        .await?;
    }
    tx.commit().await?;
    task_changed(&app, &id, "updated").await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AssigneesIn {
    assignees: Vec<String>,
}

/// Сменить исполнителя — в том числе у задачи в работе: статус, этапы и чат сохраняются.
pub async fn reassign(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AssigneesIn>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &id).await?;
    let access = scope.task_or_404(&id)?;
    let mut ids: Vec<String> = Vec::new();
    for a in req.assignees {
        if !ids.contains(&a) {
            ids.push(a);
        }
    }
    let who = involvement(&app.pool, &id, &user).await?;
    check_reassign(who, access.can_manage(), ids.len())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if current(&app.pool, &id).await?.archived_at.is_some() {
        return Err(ApiError::Conflict("задача в архиве — сначала верните её"));
    }
    let before: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM task_assignees WHERE task_id = ? ORDER BY user_id")
            .bind(&id)
            .fetch_all(&app.pool)
            .await?;
    let same_set = {
        let (mut a, mut b) = (before.clone(), ids.clone());
        a.sort();
        b.sort();
        a == b
    };
    if same_set && before.first() == ids.first() {
        return Ok(StatusCode::NO_CONTENT);
    }
    let text = format!(
        "{} сменил(а) исполнителя: {} → {}",
        user.display_name,
        names(&app.pool, &before).await?,
        names(&app.pool, &ids).await?
    );
    let mut tx = crate::db::write_tx(&app.pool).await?;
    set_assignees(&mut tx, &id, &ids).await?;
    system(&mut tx, &id, &user.id, &text).await?;
    tx.commit().await?;
    task_changed(&app, &id, "updated").await?;
    // Новым исполнителям — «вам передали задачу»; постановщику — что исполнитель сменился.
    let (title, client, creator): (String, String, String) = sqlx::query_as(
        "SELECT t.title, c.name, t.created_by FROM tasks t JOIN clients c ON c.id = t.client_id WHERE t.id = ?",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await?;
    let payload = serde_json::json!({ "title": title, "client": client, "by": user.display_name });
    for new in ids.iter().filter(|u| **u != user.id && !before.contains(u)) {
        crate::notify::push(&app, new, Some(&id), "assigned", payload.clone()).await?;
    }
    if creator != user.id && !ids.contains(&creator) {
        crate::notify::push(&app, &creator, Some(&id), "reassigned", payload).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct StatusIn {
    status: TaskStatus,
}

#[derive(Serialize)]
pub struct StatusOut {
    /// Сколько этапов осталось неотмеченными — интерфейс предупреждает.
    open_stages: i64,
}

/// Кто пользователь для этой задачи: постановщик, исполнитель, владелец.
async fn involvement(
    pool: &SqlitePool,
    task: &str,
    user: &CurrentUser,
) -> Result<Involvement, ApiError> {
    let (creator, lead): (String, Option<bool>) = sqlx::query_as(
        "SELECT created_by, (SELECT is_lead FROM task_assignees WHERE task_id = t.id AND user_id = ?)
         FROM tasks t WHERE id = ?",
    )
    .bind(&user.id)
    .bind(task)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Involvement {
        is_owner: user.role == Role::Host,
        is_creator: creator == user.id,
        is_assignee: lead == Some(true),
        is_co_assignee: lead == Some(false),
    })
}

/// «Иван Петров начал выполнение» — запись в чат при смене статуса.
/// Одинаково для любого пути, в том числе для завершённой и возвращённой из архива задачи.
fn status_phrase(to: TaskStatus) -> &'static str {
    match to {
        TaskStatus::InProgress => "начал(а) выполнение задачи",
        TaskStatus::Completed => "завершил(а) задачу",
        TaskStatus::Postponed => "отложил(а) задачу",
        TaskStatus::Waiting => "вернул(а) задачу в ожидание",
    }
}

pub async fn set_status(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<StatusIn>,
) -> Result<Json<StatusOut>, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &id).await?;
    scope.task_or_404(&id)?;
    check_status_change(involvement(&app.pool, &id, &user).await?)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let cur = current(&app.pool, &id).await?;
    if cur.archived_at.is_some() {
        return Err(ApiError::Conflict("задача в архиве — сначала верните её"));
    }
    let from: TaskStatus = cur
        .status
        .parse()
        .map_err(|_| ApiError::Corrupt(format!("статус «{}»", cur.status)))?;
    check_transition(from, req.status)?;
    if let Some(p) = &cur.parent_task_id {
        let parent: TaskStatus = parent_of(&app.pool, p)
            .await?
            .status
            .parse()
            .map_err(|_| ApiError::Corrupt("статус родительской задачи".into()))?;
        check_reopen(req.status, Some(parent)).map_err(|_| {
            ApiError::Conflict(
                "родительская задача завершена — сначала верните в работу родительскую задачу",
            )
        })?;
    }
    let open_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tasks WHERE parent_task_id = ? AND deleted_at IS NULL AND status <> 'completed'",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await?;
    check_can_complete(
        req.status,
        usize::try_from(open_children).unwrap_or(usize::MAX),
    )?;
    let open_stages: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_stages WHERE task_id = ? AND completed_at IS NULL",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await?;
    if from != req.status {
        let completed = completed_at_after(req.status, cur.completed_at, now_ms());
        let mut tx = crate::db::write_tx(&app.pool).await?;
        sqlx::query("UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?")
            .bind(req.status.as_str())
            .bind(completed)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        system(
            &mut tx,
            &id,
            &user.id,
            &format!(
                "{} {}. Статус: «{}»",
                user.display_name,
                status_phrase(req.status),
                req.status.label()
            ),
        )
        .await?;
        tx.commit().await?;
        task_changed(&app, &id, "status").await?;
    }
    Ok(Json(StatusOut { open_stages }))
}

// ---------- Этапы ----------

#[derive(Deserialize)]
pub struct StageIn {
    title: String,
}

async fn stage_task(
    pool: &SqlitePool,
    stage: &str,
) -> Result<(String, String, Option<i64>), ApiError> {
    sqlx::query_as("SELECT task_id, title, completed_at FROM task_stages WHERE id = ?")
        .bind(stage)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)
}

pub async fn add_stage(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<StageIn>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &id).await?;
    scope.require_manage_task(&id)?;
    let title = parse_title(&req.title)?;
    let mut tx = crate::db::write_tx(&app.pool).await?;
    let order: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(sort_order), -1) + 1 FROM task_stages WHERE task_id = ?",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    let sid = insert_stage(&mut tx, &id, &title, order).await?;
    tx.commit().await?;
    task_changed(&app, &id, "stages").await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": sid }))))
}

#[derive(Deserialize)]
pub struct StagePatch {
    done: Option<bool>,
    title: Option<String>,
}

pub async fn update_stage(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(sid): Path<String>,
    Json(req): Json<StagePatch>,
) -> Result<StatusCode, ApiError> {
    let (task, title, done_at) = stage_task(&app.pool, &sid).await?;
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &task).await?;
    let access = scope.task_or_404(&task)?;
    let mut tx = crate::db::write_tx(&app.pool).await?;
    if let Some(t) = &req.title {
        if !access.can_manage() {
            return Err(ApiError::Forbidden);
        }
        sqlx::query("UPDATE task_stages SET title = ? WHERE id = ?")
            .bind(parse_title(t)?)
            .bind(&sid)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(done) = req.done {
        if !access.can_work() {
            return Err(ApiError::Forbidden);
        }
        if done != done_at.is_some() {
            let (at, by) = if done {
                (Some(now_ms()), Some(user.id.clone()))
            } else {
                (None, None)
            };
            sqlx::query("UPDATE task_stages SET completed_at = ?, completed_by = ? WHERE id = ?")
                .bind(at)
                .bind(by)
                .bind(&sid)
                .execute(&mut *tx)
                .await?;
            let verb = if done {
                "отметил этап"
            } else {
                "снял отметку с этапа"
            };
            system(
                &mut tx,
                &task,
                &user.id,
                &format!("{} {verb} «{title}»", user.display_name),
            )
            .await?;
        }
    }
    tx.commit().await?;
    task_changed(&app, &task, "stages").await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_stage(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(sid): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (task, _, _) = stage_task(&app.pool, &sid).await?;
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &task).await?;
    scope.require_manage_task(&task)?;
    sqlx::query("DELETE FROM task_stages WHERE id = ?")
        .bind(&sid)
        .execute(&app.pool)
        .await?;
    task_changed(&app, &task, "stages").await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Корзина и архив ----------

pub async fn trash(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &id).await?;
    scope.require_manage_task(&id)?;
    let created_by: String = sqlx::query_scalar("SELECT created_by FROM tasks WHERE id = ?")
        .bind(&id)
        .fetch_one(&app.pool)
        .await?;
    if scope.actor.role != Role::Host && created_by != user.id {
        return Err(ApiError::Forbidden);
    }
    // Аудитория — до удаления, пока задача видна.
    let to = crate::access::audience(&app.pool, &id).await?;
    sqlx::query("UPDATE tasks SET deleted_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&id)
        .execute(&app.pool)
        .await?;
    app.rt.hub.send(
        to,
        serde_json::json!({ "type": "task", "task_id": id, "what": "deleted" }),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn restore(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (group, created_by): (String, String) = sqlx::query_as(
        "SELECT c.group_id, t.created_by FROM tasks t JOIN clients c ON c.id = t.client_id
         WHERE t.id = ? AND t.deleted_at IS NOT NULL",
    )
    .bind(&id)
    .fetch_optional(&app.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    let scope = Scope::load(&app.pool, &user).await?;
    let allowed = match scope.actor.role {
        Role::Host => true,
        Role::Manager => scope.actor.groups.contains(&group) && created_by == user.id,
        Role::Employee => false,
    };
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    sqlx::query("UPDATE tasks SET deleted_at = NULL WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?;
    task_changed(&app, &id, "restored").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Проверка прав на архив: задача видна, завершена, пользователь — исполнитель или владелец.
async fn archive_check(app: &AppState, user: &CurrentUser, id: &str) -> Result<Current, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, id).await?;
    scope.task_or_404(id)?;
    let cur = current(&app.pool, id).await?;
    let status: TaskStatus = cur
        .status
        .parse()
        .map_err(|_| ApiError::Corrupt(format!("статус «{}»", cur.status)))?;
    check_archive(involvement(&app.pool, id, user).await?, status)?;
    Ok(cur)
}

/// В архив: завершённая задача вместе с подзадачами. Убирает исполнитель или владелец.
pub async fn archive(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    archive_one(&app, &user, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct ArchivedOut {
    archived: usize,
}

/// «В архив все завершённые» — те, что этот человек вправе убрать.
pub async fn archive_completed(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<ArchivedOut>, ApiError> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM tasks WHERE status = 'completed' AND archived_at IS NULL AND deleted_at IS NULL
         ORDER BY parent_task_id IS NOT NULL, created_at",
    )
    .fetch_all(&app.pool)
    .await?;
    let mut archived = 0;
    for id in ids {
        match archive_one(&app, &user, &id).await {
            Ok(true) => archived += 1,
            Ok(false) | Err(ApiError::NotFound | ApiError::BadRequest(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Json(ArchivedOut { archived }))
}

/// `true` — убрали сейчас; `false` — уже была в архиве (например, вместе с родительской).
async fn archive_one(app: &AppState, user: &CurrentUser, id: &str) -> Result<bool, ApiError> {
    let cur = archive_check(app, user, id).await?;
    if cur.archived_at.is_some() {
        return Ok(false);
    }
    let mut tx = crate::db::write_tx(&app.pool).await?;
    sqlx::query(
        "WITH RECURSIVE down(id) AS (SELECT ? UNION SELECT t.id FROM tasks t JOIN down ON t.parent_task_id = down.id)
         UPDATE tasks SET archived_at = ?, archived_by = ?
         WHERE id IN (SELECT id FROM down) AND status = 'completed' AND archived_at IS NULL",
    )
    .bind(id)
    .bind(now_ms())
    .bind(&user.id)
    .execute(&mut *tx)
    .await?;
    system(
        &mut tx,
        id,
        &user.id,
        &format!("{} убрал(а) задачу в архив", user.display_name),
    )
    .await?;
    tx.commit().await?;
    task_changed(app, id, "archived").await?;
    Ok(true)
}

/// Восстановить из архива: задача, её подзадачи и родители, если они тоже в архиве.
#[derive(Deserialize)]
pub struct UnarchiveQuery {
    /// Сразу вернуть в работу — одним действием: или всё, или ничего.
    #[serde(default)]
    reopen: Option<u8>,
}

pub async fn unarchive(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(q): Query<UnarchiveQuery>,
) -> Result<StatusCode, ApiError> {
    archive_check(&app, &user, &id).await?;
    let reopen = q.reopen.is_some_and(|r| r != 0);
    if reopen {
        // Подзадачу под завершённой задачей в работу не вернуть — проверяем до изменений.
        let cur = current(&app.pool, &id).await?;
        if let Some(p) = &cur.parent_task_id {
            let parent: TaskStatus = parent_of(&app.pool, p)
                .await?
                .status
                .parse()
                .map_err(|_| ApiError::Corrupt("статус родительской задачи".into()))?;
            check_reopen(TaskStatus::InProgress, Some(parent)).map_err(|_| {
                ApiError::Conflict(
                    "родительская задача завершена — сначала верните в работу родительскую задачу",
                )
            })?;
        }
    }
    let mut tx = crate::db::write_tx(&app.pool).await?;
    sqlx::query(
        "WITH RECURSIVE down(id) AS (SELECT ? UNION SELECT t.id FROM tasks t JOIN down ON t.parent_task_id = down.id),
              up(id, parent) AS (SELECT id, parent_task_id FROM tasks WHERE id = ?
                                 UNION SELECT t.id, t.parent_task_id FROM tasks t JOIN up ON t.id = up.parent)
         UPDATE tasks SET archived_at = NULL, archived_by = NULL
         WHERE id IN (SELECT id FROM down UNION SELECT id FROM up)",
    )
    .bind(&id)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    let text = if reopen {
        sqlx::query("UPDATE tasks SET status = 'in_progress', completed_at = NULL WHERE id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        format!(
            "{} вернул(а) задачу из архива в работу. Статус: «{}»",
            user.display_name,
            TaskStatus::InProgress.label()
        )
    } else {
        format!("{} восстановил(а) задачу из архива", user.display_name)
    };
    system(&mut tx, &id, &user.id, &text).await?;
    tx.commit().await?;
    task_changed(&app, &id, "unarchived").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Удалить из архива навсегда (с подзадачами, чатом и файлами) — только владелец.
pub async fn purge(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if user.role != Role::Host {
        return Err(ApiError::Forbidden);
    }
    let cur = current(&app.pool, &id).await?;
    if cur.archived_at.is_none() {
        return Err(ApiError::Conflict(
            "удалить навсегда можно только задачу из архива",
        ));
    }
    let to = crate::access::audience(&app.pool, &id).await?;
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?;
    crate::attachments::collect_garbage(&app).await?;
    crate::auth::audit(
        &app.pool,
        Some(&user.id),
        "task.purged",
        Some(&cur.title),
        "local",
    )
    .await?;
    app.rt.hub.send(
        to,
        serde_json::json!({ "type": "task", "task_id": id, "what": "deleted" }),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct TrashedTask {
    id: String,
    title: String,
    client_name: String,
    deleted_at: i64,
}

/// Корзина задач: владелец — все, менеджер — созданные им в своих группах.
pub async fn trashed(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<TrashedTask>>, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    if scope.actor.role == Role::Employee {
        return Err(ApiError::Forbidden);
    }
    let rows: Vec<(String, String, String, i64, String, String)> = sqlx::query_as(
        "SELECT t.id, t.title, c.name, t.deleted_at, c.group_id, t.created_by
         FROM tasks t JOIN clients c ON c.id = t.client_id
         WHERE t.deleted_at IS NOT NULL AND c.deleted_at IS NULL ORDER BY t.deleted_at DESC",
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .filter(|(_, _, _, _, g, by)| {
                scope.actor.role == Role::Host || (scope.actor.groups.contains(g) && *by == user.id)
            })
            .map(|(id, title, client_name, deleted_at, _, _)| TrashedTask {
                id,
                title,
                client_name,
                deleted_at,
            })
            .collect(),
    ))
}
