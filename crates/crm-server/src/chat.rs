//! Чат задачи, «Мессенджер» и непрочитанные — у каждого свои (ТЗ §4.6).

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::access::Scope;
use crate::auth::{new_id, now_ms, CurrentUser};
use crate::events::{task_changed, unread_changed};
use crate::http::{ApiError, AppState};

/// Своё сообщение можно исправить в течение 15 минут.
pub const EDIT_WINDOW_MS: i64 = 15 * 60 * 1000;
const MAX_TEXT: usize = 10_000;

/// Системная запись в чат задачи: «Иван отметил этап …». Автор — тот, кто действовал,
/// поэтому у него самого она непрочитанной не считается.
pub async fn system(
    tx: &mut Transaction<'_, Sqlite>,
    task: &str,
    actor: &str,
    text: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO messages(id, task_id, sender_id, kind, content, created_at) VALUES (?, ?, ?, 'system', ?, ?)",
    )
    .bind(new_id())
    .bind(task)
    .bind(actor)
    .bind(text)
    .bind(now_ms())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Непрочитанные пользователем сообщения других людей — по задачам.
/// ID сообщений — UUIDv7: строковый порядок совпадает с порядком времени.
/// Непрочитанные по задачам; `only` — JSON-массив задач, которыми ограничиться.
pub async fn unread_by_task(
    pool: &SqlitePool,
    me: &str,
    only: Option<&str>,
) -> Result<HashMap<String, i64>, ApiError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT m.task_id, count(*) FROM messages m
         LEFT JOIN chat_reads r ON r.task_id = m.task_id AND r.user_id = ?
         WHERE m.sender_id <> ? AND m.id > coalesce(r.last_read_message_id, '')
           AND (?3 IS NULL OR m.task_id IN (SELECT value FROM json_each(?3)))
         GROUP BY m.task_id",
    )
    .bind(me)
    .bind(me)
    .bind(only)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

#[derive(Serialize)]
pub struct Unread {
    total: i64,
    tasks: HashMap<String, i64>,
}

/// Счётчики только по чатам, к которым есть доступ.
pub async fn unread(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Unread>, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let tasks: HashMap<String, i64> =
        unread_by_task(&app.pool, &user.id, scope.visible_json().as_deref())
            .await?
            .into_iter()
            .filter(|(t, _)| scope.task(t).can_read())
            .collect();
    Ok(Json(Unread {
        total: tasks.values().sum(),
        tasks,
    }))
}

// ---------- Сообщения ----------

#[derive(Serialize, Clone)]
pub struct AttachmentRef {
    sha256: String,
    file_name: String,
    size_bytes: i64,
    mime: String,
}

#[derive(Serialize)]
pub struct ReplyRef {
    id: String,
    sender_name: String,
    excerpt: String,
}

#[derive(Serialize)]
pub struct Message {
    id: String,
    sender_id: String,
    sender_name: String,
    kind: String,
    content: String,
    created_at: i64,
    edited_at: Option<i64>,
    reply_to: Option<ReplyRef>,
    attachments: Vec<AttachmentRef>,
    can_edit: bool,
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: String,
    sender_id: String,
    sender_name: String,
    kind: String,
    content: String,
    created_at: i64,
    edited_at: Option<i64>,
    reply_to_id: Option<String>,
}

#[derive(Serialize)]
pub struct Chat {
    messages: Vec<Message>,
    /// Последнее прочитанное мной — над следующими интерфейс рисует «Новые сообщения».
    last_read: Option<String>,
}

fn excerpt(s: &str) -> String {
    let mut e: String = s.chars().take(120).collect();
    if s.chars().count() > 120 {
        e.push('…');
    }
    e
}

pub async fn messages(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(task): Path<String>,
) -> Result<Json<Chat>, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &task).await?;
    scope.task_or_404(&task)?;
    let rows: Vec<MessageRow> =
        sqlx::query_as(
            "SELECT m.id, m.sender_id, u.display_name AS sender_name, m.kind, m.content, m.created_at, m.edited_at, m.reply_to_id
             FROM messages m JOIN users u ON u.id = m.sender_id
             WHERE m.task_id = ? ORDER BY m.id",
        )
        .bind(&task)
        .fetch_all(&app.pool)
        .await?;
    let files: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT ma.message_id, ma.sha256, ma.file_name, a.size_bytes, a.mime
         FROM message_attachments ma JOIN attachments a ON a.sha256 = ma.sha256
         JOIN messages m ON m.id = ma.message_id WHERE m.task_id = ?",
    )
    .bind(&task)
    .fetch_all(&app.pool)
    .await?;
    let mut by_msg: HashMap<String, Vec<AttachmentRef>> = HashMap::new();
    for (m, sha256, file_name, size_bytes, mime) in files {
        by_msg.entry(m).or_default().push(AttachmentRef {
            sha256,
            file_name,
            size_bytes,
            mime,
        });
    }
    let index: HashMap<String, (String, String)> = rows
        .iter()
        .map(|r| (r.id.clone(), (r.sender_name.clone(), r.content.clone())))
        .collect();
    let now = now_ms();
    let messages = rows
        .into_iter()
        .map(
            |MessageRow {
                 id,
                 sender_id,
                 sender_name,
                 kind,
                 content,
                 created_at,
                 edited_at,
                 reply_to_id: reply,
             }| {
                let can_edit =
                    kind == "user" && sender_id == user.id && now - created_at < EDIT_WINDOW_MS;
                Message {
                    reply_to: reply.and_then(|r| {
                        index.get(&r).map(|(n, c)| ReplyRef {
                            id: r.clone(),
                            sender_name: n.clone(),
                            excerpt: excerpt(c),
                        })
                    }),
                    attachments: by_msg.remove(&id).unwrap_or_default(),
                    id,
                    sender_id,
                    sender_name,
                    kind,
                    content,
                    created_at,
                    edited_at,
                    can_edit,
                }
            },
        )
        .collect();
    let last_read = sqlx::query_scalar(
        "SELECT last_read_message_id FROM chat_reads WHERE user_id = ? AND task_id = ?",
    )
    .bind(&user.id)
    .bind(&task)
    .fetch_optional(&app.pool)
    .await?;
    Ok(Json(Chat {
        messages,
        last_read,
    }))
}

#[derive(Deserialize)]
pub struct PostIn {
    #[serde(default)]
    content: String,
    reply_to: Option<String>,
    #[serde(default)]
    attachments: Vec<FileIn>,
}

#[derive(Deserialize)]
pub struct FileIn {
    sha256: String,
    file_name: String,
}

fn check_text(s: &str) -> Result<String, ApiError> {
    let t = s.trim();
    if t.chars().count() > MAX_TEXT {
        Err(ApiError::BadRequest(format!(
            "сообщение не длиннее {MAX_TEXT} символов"
        )))
    } else {
        Ok(t.to_owned())
    }
}

pub async fn post(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(task): Path<String>,
    Json(req): Json<PostIn>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &task).await?;
    if !scope.task_or_404(&task)?.can_work() {
        return Err(ApiError::Forbidden);
    }
    let content = check_text(&req.content)?;
    if content.is_empty() && req.attachments.is_empty() {
        return Err(ApiError::BadRequest("пустое сообщение".into()));
    }
    if req.attachments.len() > 20 {
        return Err(ApiError::BadRequest(
            "не больше 20 файлов в сообщении".into(),
        ));
    }
    let id = new_id();
    let mut tx = crate::db::write_tx(&app.pool).await?;
    if let Some(r) = &req.reply_to {
        let same: i64 =
            sqlx::query_scalar("SELECT count(*) FROM messages WHERE id = ? AND task_id = ?")
                .bind(r)
                .bind(&task)
                .fetch_one(&mut *tx)
                .await?;
        if same == 0 {
            return Err(ApiError::BadRequest(
                "ответ на сообщение из другого чата".into(),
            ));
        }
    }
    sqlx::query(
        "INSERT INTO messages(id, task_id, sender_id, kind, content, reply_to_id, created_at)
         VALUES (?, ?, ?, 'user', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&task)
    .bind(&user.id)
    .bind(&content)
    .bind(&req.reply_to)
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    for f in &req.attachments {
        let name = crate::attachments::clean_name(&f.file_name);
        let n = sqlx::query(
            "INSERT OR IGNORE INTO message_attachments(message_id, sha256, file_name)
             SELECT ?, sha256, ? FROM attachments WHERE sha256 = ?",
        )
        .bind(&id)
        .bind(&name)
        .bind(&f.sha256)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(ApiError::BadRequest("файл не загружен".into()));
        }
    }
    // Своё сообщение — прочитано.
    mark_read(&mut tx, &user.id, &task, &id).await?;
    tx.commit().await?;
    task_changed(&app, &task, "message").await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(Deserialize)]
pub struct EditIn {
    content: String,
}

pub async fn edit(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<EditIn>,
) -> Result<StatusCode, ApiError> {
    let (task, sender, kind, created_at): (String, String, String, i64) =
        sqlx::query_as("SELECT task_id, sender_id, kind, created_at FROM messages WHERE id = ?")
            .bind(&id)
            .fetch_optional(&app.pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &task).await?;
    scope.task_or_404(&task)?;
    if sender != user.id || kind != "user" {
        return Err(ApiError::Forbidden);
    }
    if now_ms() - created_at >= EDIT_WINDOW_MS {
        return Err(ApiError::Conflict("исправлять можно в течение 15 минут"));
    }
    let content = check_text(&req.content)?;
    if content.is_empty() {
        return Err(ApiError::BadRequest("пустое сообщение".into()));
    }
    sqlx::query("UPDATE messages SET content = ?, edited_at = ? WHERE id = ?")
        .bind(&content)
        .bind(now_ms())
        .bind(&id)
        .execute(&app.pool)
        .await?;
    task_changed(&app, &task, "message").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Прочитанность только растёт: старая отметка с другого устройства не откатывает новую.
async fn mark_read(
    tx: &mut Transaction<'_, Sqlite>,
    user: &str,
    task: &str,
    message: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO chat_reads(user_id, task_id, last_read_message_id) VALUES (?, ?, ?)
         ON CONFLICT(user_id, task_id) DO UPDATE SET last_read_message_id = excluded.last_read_message_id
         WHERE excluded.last_read_message_id > chat_reads.last_read_message_id",
    )
    .bind(user)
    .bind(task)
    .bind(message)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ReadIn {
    /// До какого сообщения дочитал; нет — до последнего.
    message_id: Option<String>,
}

pub async fn read(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(task): Path<String>,
    Json(req): Json<ReadIn>,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::for_task(&app.pool, &user.id, user.role, &task).await?;
    scope.task_or_404(&task)?;
    let last: Option<String> = match req.message_id {
        Some(m) => {
            sqlx::query_scalar("SELECT id FROM messages WHERE id = ? AND task_id = ?")
                .bind(m)
                .bind(&task)
                .fetch_optional(&app.pool)
                .await?
        }
        None => {
            sqlx::query_scalar("SELECT max(id) FROM messages WHERE task_id = ?")
                .bind(&task)
                .fetch_one(&app.pool)
                .await?
        }
    };
    if let Some(m) = last {
        let mut tx = crate::db::write_tx(&app.pool).await?;
        mark_read(&mut tx, &user.id, &task, &m).await?;
        tx.commit().await?;
        unread_changed(&app, &user.id);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// «Прочитать все» — по всем доступным чатам.
pub async fn read_all(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let last: Vec<(String, String)> =
        sqlx::query_as("SELECT task_id, max(id) FROM messages GROUP BY task_id")
            .fetch_all(&app.pool)
            .await?;
    let mut tx = crate::db::write_tx(&app.pool).await?;
    for (task, m) in last {
        if scope.task(&task).can_read() {
            mark_read(&mut tx, &user.id, &task, &m).await?;
        }
    }
    tx.commit().await?;
    unread_changed(&app, &user.id);
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Мессенджер ----------

#[derive(Deserialize, Default)]
pub struct ThreadsQuery {
    #[serde(default)]
    unread_only: bool,
}

#[derive(Serialize)]
pub struct Thread {
    #[serde(flatten)]
    task: crate::tasks::TaskSummary,
    last_message: Option<LastMessage>,
}

#[derive(Serialize)]
pub struct LastMessage {
    sender_name: String,
    excerpt: String,
    created_at: i64,
    kind: String,
}

/// Треды «Клиент → Задача», свежие сверху.
pub async fn threads(
    State(app): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ThreadsQuery>,
) -> Result<Json<Vec<Thread>>, ApiError> {
    let _heavy = app.rt.heavy().await?;
    let scope = Scope::load(&app.pool, &user).await?;
    let last: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT m.task_id, u.display_name, m.content, m.created_at, m.kind
         FROM messages m JOIN users u ON u.id = m.sender_id
         WHERE m.id = (SELECT max(id) FROM messages x WHERE x.task_id = m.task_id)",
    )
    .fetch_all(&app.pool)
    .await?;
    let mut last: HashMap<String, LastMessage> = last
        .into_iter()
        .map(|(t, sender_name, content, created_at, kind)| {
            (
                t,
                LastMessage {
                    sender_name,
                    excerpt: excerpt(&content),
                    created_at,
                    kind,
                },
            )
        })
        .collect();
    let mut out: Vec<Thread> = crate::tasks::summaries(&app, &scope, &user.id)
        .await?
        .into_iter()
        .filter(|t| t.details.as_ref().is_some_and(|d| d.archived_at.is_none()))
        .filter(|t| !q.unread_only || t.details.as_ref().is_some_and(|d| d.unread > 0))
        .map(|task| Thread {
            last_message: last.remove(&task.id),
            task,
        })
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.last_message.as_ref().map_or(0, |m| m.created_at)));
    Ok(Json(out))
}
