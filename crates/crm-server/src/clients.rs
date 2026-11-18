//! Клиенты — учётные записи для группировки задач (ТЗ §4.1). Без доступа к серверу.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use crm_core::access::{can_create_client, can_purge, can_trash, ClientAccess};
use crm_core::task::parse_title;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

use crate::access::Scope;
use crate::auth::{new_id, now_ms, CurrentUser};
use crate::http::{ApiError, AppState};
use crate::tasks::{summaries, TaskSummary};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Person,
    Company,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Company => "company",
        }
    }
}

/// Карточка клиента, как её присылает интерфейс или импорт.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientIn {
    pub kind: Kind,
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_group")]
    pub group_id: String,
    #[serde(default)]
    pub manager_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_group() -> String {
    "default".into()
}

/// Проверенная карточка.
pub struct Valid {
    pub kind: Kind,
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub description: String,
    pub group_id: String,
    pub manager_id: Option<String>,
    pub tags: Vec<String>,
}

fn opt(s: Option<String>, max: usize, what: &str) -> Result<Option<String>, ApiError> {
    match s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
        Some(s) if s.chars().count() > max => Err(ApiError::BadRequest(format!(
            "{what}: не длиннее {max} символов"
        ))),
        other => Ok(other),
    }
}

impl ClientIn {
    pub fn validate(self) -> Result<Valid, ApiError> {
        let name = parse_title(&self.name)
            .map_err(|_| ApiError::BadRequest("название клиента: 1–300 символов".into()))?;
        let email = opt(self.email, 200, "email")?;
        if email.as_deref().is_some_and(|e| !e.contains('@')) {
            return Err(ApiError::BadRequest("email указан неверно".into()));
        }
        if self.description.chars().count() > 20_000 {
            return Err(ApiError::BadRequest(
                "описание: не длиннее 20 000 символов".into(),
            ));
        }
        let mut tags: Vec<String> = Vec::new();
        for t in self.tags {
            let t = t.trim().to_owned();
            if t.is_empty() {
                continue;
            }
            if t.chars().count() > 40 {
                return Err(ApiError::BadRequest("тег: не длиннее 40 символов".into()));
            }
            if !tags.iter().any(|x| x.to_lowercase() == t.to_lowercase()) {
                tags.push(t);
            }
        }
        Ok(Valid {
            kind: self.kind,
            name,
            phone: opt(self.phone, 50, "телефон")?,
            email,
            description: self.description,
            group_id: self.group_id,
            manager_id: self.manager_id.filter(|m| !m.is_empty()),
            tags,
        })
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct Tag {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ClientRow {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub description: String,
    pub group_id: String,
    pub group_name: String,
    pub manager_id: Option<String>,
    pub manager_name: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

/// Запрос карточек клиента с хвостом условия — строка собирается при компиляции.
macro_rules! select_clients {
    ($tail:literal) => {
        concat!("SELECT c.id, c.kind, c.name, c.phone, c.email, c.description, c.group_id,
        g.name AS group_name, c.manager_id, m.display_name AS manager_name,
        c.created_by, c.created_at, c.deleted_at
    FROM clients c JOIN groups g ON g.id = c.group_id LEFT JOIN users m ON m.id = c.manager_id ", $tail)
    };
}

#[derive(Debug, Serialize, Default, Clone, Copy)]
pub struct Counts {
    pub open: usize,
    pub overdue: usize,
    pub done_30d: usize,
    pub unread: i64,
}

#[derive(Serialize)]
pub struct ClientItem {
    #[serde(flatten)]
    pub client: ClientRow,
    pub tags: Vec<Tag>,
    pub counts: Counts,
    pub access: ClientAccess,
}

/// Палитра тегов: цвет выбирается по названию — один и тот же тег всегда одного цвета.
const TAG_COLORS: [&str; 8] = [
    "#2563eb", "#16a34a", "#d97706", "#dc2626", "#7c3aed", "#0891b2", "#db2777", "#4b5563",
];

fn tag_color(name: &str) -> &'static str {
    let h = name
        .to_lowercase()
        .bytes()
        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    TAG_COLORS[h % TAG_COLORS.len()]
}

async fn tags_of(pool: &sqlx::SqlitePool) -> Result<HashMap<String, Vec<Tag>>, ApiError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT ct.client_id, t.name, t.color FROM client_tags ct JOIN tags t ON t.id = ct.tag_id
         ORDER BY t.name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    let mut out: HashMap<String, Vec<Tag>> = HashMap::new();
    for (c, name, color) in rows {
        out.entry(c).or_default().push(Tag { name, color });
    }
    Ok(out)
}

pub(crate) async fn set_tags(
    tx: &mut Transaction<'_, Sqlite>,
    client: &str,
    tags: &[String],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM client_tags WHERE client_id = ?")
        .bind(client)
        .execute(&mut **tx)
        .await?;
    for t in tags {
        sqlx::query("INSERT OR IGNORE INTO tags(id, name, color) VALUES (?, ?, ?)")
            .bind(new_id())
            .bind(t)
            .bind(tag_color(t))
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO client_tags(client_id, tag_id) SELECT ?, id FROM tags WHERE name = ?",
        )
        .bind(client)
        .bind(t)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn check_refs(tx: &mut Transaction<'_, Sqlite>, v: &Valid) -> Result<(), ApiError> {
    let group: i64 = sqlx::query_scalar("SELECT count(*) FROM groups WHERE id = ?")
        .bind(&v.group_id)
        .fetch_one(&mut **tx)
        .await?;
    if group == 0 {
        return Err(ApiError::BadRequest("неизвестная группа".into()));
    }
    if let Some(m) = &v.manager_id {
        let ok: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users WHERE id = ? AND is_active = 1 AND role IN ('host','manager')",
        )
        .bind(m)
        .fetch_one(&mut **tx)
        .await?;
        if ok == 0 {
            return Err(ApiError::BadRequest(
                "ответственным может быть только менеджер или владелец".into(),
            ));
        }
    }
    Ok(())
}

/// Записать нового клиента (используется и импортом).
pub(crate) async fn insert(
    tx: &mut Transaction<'_, Sqlite>,
    v: &Valid,
    by: &str,
    batch: Option<&str>,
) -> Result<String, ApiError> {
    check_refs(tx, v).await?;
    let id = new_id();
    sqlx::query(
        "INSERT INTO clients(id, kind, name, phone, email, description, group_id, manager_id,
                             import_batch_id, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(v.kind.as_str())
    .bind(&v.name)
    .bind(&v.phone)
    .bind(&v.email)
    .bind(&v.description)
    .bind(&v.group_id)
    .bind(&v.manager_id)
    .bind(batch)
    .bind(by)
    .bind(now_ms())
    .execute(&mut **tx)
    .await?;
    set_tags(tx, &id, &v.tags).await?;
    Ok(id)
}

pub(crate) async fn update_row(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    v: &Valid,
) -> Result<(), ApiError> {
    check_refs(tx, v).await?;
    sqlx::query(
        "UPDATE clients SET kind = ?, name = ?, phone = ?, email = ?, description = ?,
                group_id = ?, manager_id = ? WHERE id = ?",
    )
    .bind(v.kind.as_str())
    .bind(&v.name)
    .bind(&v.phone)
    .bind(&v.email)
    .bind(&v.description)
    .bind(&v.group_id)
    .bind(&v.manager_id)
    .bind(id)
    .execute(&mut **tx)
    .await?;
    set_tags(tx, id, &v.tags).await
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    /// Показать корзину.
    #[serde(default)]
    trash: bool,
}

/// Клиенты, видимые пользователю, со сводкой по его задачам.
pub async fn list(
    State(app): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ClientItem>>, ApiError> {
    let _heavy = app.rt.heavy().await?;
    let scope = Scope::load(&app.pool, &user).await?;
    if q.trash {
        if scope.actor.role == crm_core::auth::Role::Employee {
            return Err(ApiError::Forbidden);
        }
        let rows: Vec<ClientRow> = sqlx::query_as(select_clients!(
            "WHERE c.deleted_at IS NOT NULL ORDER BY c.deleted_at DESC"
        ))
        .fetch_all(&app.pool)
        .await?;
        let groups = &scope.actor.groups;
        let host = scope.actor.role == crm_core::auth::Role::Host;
        return Ok(Json(
            rows.into_iter()
                .filter(|c| host || groups.contains(&c.group_id))
                .map(|client| ClientItem {
                    client,
                    tags: Vec::new(),
                    counts: Counts::default(),
                    access: ClientAccess::Manage,
                })
                .collect(),
        ));
    }
    let rows: Vec<ClientRow> = sqlx::query_as(select_clients!(
        "WHERE c.deleted_at IS NULL ORDER BY c.name COLLATE NOCASE"
    ))
    .fetch_all(&app.pool)
    .await?;
    let tags = tags_of(&app.pool).await?;
    let tasks = summaries(&app, &scope, &user.id).await?;
    let counts = counts_by_client(&tasks);
    let needle =
        q.q.as_deref()
            .map(str::to_lowercase)
            .filter(|s| !s.is_empty());
    let tag = q.tag.as_deref().map(str::to_lowercase);
    let out = rows
        .into_iter()
        .filter_map(|client| {
            let access = scope.client(&client.id);
            if access == ClientAccess::None {
                return None;
            }
            let t = tags.get(&client.id).cloned().unwrap_or_default();
            if let Some(n) = &needle {
                let hay = format!(
                    "{} {} {} {}",
                    client.name,
                    client.phone.as_deref().unwrap_or(""),
                    client.email.as_deref().unwrap_or(""),
                    client.description
                )
                .to_lowercase();
                if !hay.contains(n.as_str()) {
                    return None;
                }
            }
            if let Some(tag) = &tag {
                if !t.iter().any(|x| x.name.to_lowercase() == *tag) {
                    return None;
                }
            }
            Some(ClientItem {
                counts: counts.get(&client.id).copied().unwrap_or_default(),
                client,
                tags: t,
                access,
            })
        })
        .collect();
    Ok(Json(out))
}

fn counts_by_client(tasks: &[TaskSummary]) -> HashMap<String, Counts> {
    let month_ago = now_ms() - 30 * 24 * 3600 * 1000;
    let mut out: HashMap<String, Counts> = HashMap::new();
    for t in tasks {
        let Some(d) = &t.details else { continue };
        let c = out.entry(t.client_id.clone()).or_default();
        c.unread += d.unread;
        match d.status {
            crm_core::task::TaskStatus::Completed => {
                if d.completed_at.is_some_and(|at| at > month_ago) {
                    c.done_30d += 1;
                }
            }
            _ => {
                c.open += 1;
                if d.deadline_state == crm_core::task::DeadlineState::Overdue {
                    c.overdue += 1;
                }
            }
        }
    }
    out
}

pub async fn create(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(req): Json<ClientIn>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let v = req.validate()?;
    let scope = Scope::load(&app.pool, &user).await?;
    if !can_create_client(&scope.actor, &v.group_id) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = crate::db::write_tx(&app.pool).await?;
    let id = insert(&mut tx, &v, &user.id, None).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(Serialize)]
pub struct ClientCard {
    #[serde(flatten)]
    item: ClientItem,
    can_trash: bool,
    tasks: Vec<TaskSummary>,
}

pub async fn get(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ClientCard>, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let access = scope.client_or_404(&id)?;
    let client: ClientRow = sqlx::query_as(select_clients!("WHERE c.id = ?"))
        .bind(&id)
        .fetch_optional(&app.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    let tasks: Vec<TaskSummary> = summaries(&app, &scope, &user.id)
        .await?
        .into_iter()
        .filter(|t| t.client_id == id)
        .collect();
    let counts = counts_by_client(&tasks).remove(&id).unwrap_or_default();
    let tags = tags_of(&app.pool).await?.remove(&id).unwrap_or_default();
    let can_trash = can_trash(&scope.actor, access, &client.created_by);
    Ok(Json(ClientCard {
        item: ClientItem {
            client,
            tags,
            counts,
            access,
        },
        can_trash,
        tasks,
    }))
}

fn require_manage(scope: &Scope, id: &str) -> Result<(), ApiError> {
    match scope.client_or_404(id)? {
        ClientAccess::Manage => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

pub async fn update(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<ClientIn>,
) -> Result<StatusCode, ApiError> {
    let v = req.validate()?;
    let scope = Scope::load(&app.pool, &user).await?;
    require_manage(&scope, &id)?;
    // Перенос в чужую группу запрещён: менеджер потерял бы клиента.
    if !can_create_client(&scope.actor, &v.group_id) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = crate::db::write_tx(&app.pool).await?;
    update_row(&mut tx, &id, &v).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn trash(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let access = scope.client_or_404(&id)?;
    let created_by: String = sqlx::query_scalar("SELECT created_by FROM clients WHERE id = ?")
        .bind(&id)
        .fetch_one(&app.pool)
        .await?;
    if !can_trash(&scope.actor, access, &created_by) {
        return Err(ApiError::Forbidden);
    }
    sqlx::query("UPDATE clients SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(now_ms())
        .bind(&id)
        .execute(&app.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Клиент из корзины: владелец — любой, менеджер — своей группы.
async fn trashed_group(app: &AppState, id: &str) -> Result<(String, String), ApiError> {
    sqlx::query_as(
        "SELECT group_id, created_by FROM clients WHERE id = ? AND deleted_at IS NOT NULL",
    )
    .bind(id)
    .fetch_optional(&app.pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn restore(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let (group, created_by) = trashed_group(&app, &id).await?;
    let access = if can_create_client(&scope.actor, &group) {
        ClientAccess::Manage
    } else {
        ClientAccess::None
    };
    if !can_trash(&scope.actor, access, &created_by) {
        return Err(ApiError::Forbidden);
    }
    sqlx::query("UPDATE clients SET deleted_at = NULL WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Окончательное удаление клиента со всеми задачами и чатами — только владелец.
pub async fn purge(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    if !can_purge(&scope.actor) {
        return Err(ApiError::Forbidden);
    }
    trashed_group(&app, &id).await?;
    sqlx::query("DELETE FROM clients WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?;
    crate::attachments::collect_garbage(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn all_tags(
    State(app): State<AppState>,
    _: CurrentUser,
) -> Result<Json<Vec<Tag>>, ApiError> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, color FROM tags ORDER BY name COLLATE NOCASE")
            .fetch_all(&app.pool)
            .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(name, color)| Tag { name, color })
            .collect(),
    ))
}
