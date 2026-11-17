//! Организация: рабочее время, группы, справочник сотрудников.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Datelike, TimeZone, Utc};
use crm_core::org::OrgSettings;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::{new_id, now_ms, CurrentUser, Owner};
use crate::http::{ApiError, AppState};

const KEY: &str = "org";

pub async fn load(pool: &SqlitePool) -> Result<OrgSettings, ApiError> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(KEY)
        .fetch_optional(pool)
        .await?;
    serde_json::from_str(raw.as_deref().unwrap_or("{}"))
        .map_err(|e| ApiError::Corrupt(format!("настройки организации: {e}")))
}

pub async fn get_settings(
    State(app): State<AppState>,
    _: CurrentUser,
) -> Result<Json<OrgSettings>, ApiError> {
    Ok(Json(load(&app.pool).await?))
}

pub async fn put_settings(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(raw): Json<crm_core::org::RawOrg>,
) -> Result<Json<OrgSettings>, ApiError> {
    let s = OrgSettings::try_from(raw).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let json = serde_json::to_string(&s).map_err(|e| ApiError::Failed(e.to_string()))?;
    sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(KEY)
        .bind(json)
        .execute(&app.pool)
        .await?;
    Ok(Json(s))
}

#[derive(Deserialize)]
pub struct AtQuery {
    at: i64,
}

#[derive(Serialize)]
pub struct WorkCheck {
    working: bool,
    /// «суббота», «праздник», «после 19:00» — для подсказки «Это суббота — оставить?»
    reason: Option<String>,
}

/// Попадает ли момент в рабочее время — для предупреждения при выборе дедлайна.
pub async fn check_time(
    State(app): State<AppState>,
    _: CurrentUser,
    Query(q): Query<AtQuery>,
) -> Result<Json<WorkCheck>, ApiError> {
    let org = load(&app.pool).await?;
    let at = Utc
        .timestamp_millis_opt(q.at)
        .single()
        .ok_or_else(|| ApiError::BadRequest("некорректное время".into()))?;
    let s = org.schedule();
    if s.is_working(at) {
        return Ok(Json(WorkCheck {
            working: true,
            reason: None,
        }));
    }
    let local = at.with_timezone(&s.tz());
    const DAYS: [&str; 7] = [
        "понедельник",
        "вторник",
        "среда",
        "четверг",
        "пятница",
        "суббота",
        "воскресенье",
    ];
    let day = DAYS
        .get(local.weekday().num_days_from_monday() as usize)
        .copied()
        .unwrap_or("выходной");
    let start = s.next_work_start(at);
    let same_day_work =
        start.is_some_and(|n| n.with_timezone(&s.tz()).date_naive() == local.date_naive());
    let reason = if same_day_work {
        "нерабочее время (до начала рабочего дня)".to_owned()
    } else if s
        .prev_work_end(at)
        .is_some_and(|(_, e)| e.with_timezone(&s.tz()).date_naive() == local.date_naive())
    {
        "нерабочее время (после конца рабочего дня)".to_owned()
    } else {
        format!("{day} — нерабочий день")
    };
    Ok(Json(WorkCheck {
        working: false,
        reason: Some(reason),
    }))
}

// ---------- Справочник сотрудников (для выбора исполнителей) ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Person {
    id: String,
    display_name: String,
    role: String,
}

pub async fn people(
    State(app): State<AppState>,
    _: CurrentUser,
) -> Result<Json<Vec<Person>>, ApiError> {
    let rows = sqlx::query_as(
        "SELECT id, display_name, role FROM users WHERE is_active = 1 ORDER BY display_name COLLATE NOCASE",
    )
    .fetch_all(&app.pool)
    .await?;
    Ok(Json(rows))
}

// ---------- Группы ----------

#[derive(Serialize)]
pub struct Group {
    id: String,
    name: String,
    members: Vec<String>,
    clients: i64,
}

pub async fn groups(
    State(app): State<AppState>,
    _: CurrentUser,
) -> Result<Json<Vec<Group>>, ApiError> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT g.id, g.name,
                (SELECT count(*) FROM clients c WHERE c.group_id = g.id AND c.deleted_at IS NULL)
         FROM groups g ORDER BY g.id <> 'default', g.name COLLATE NOCASE",
    )
    .fetch_all(&app.pool)
    .await?;
    let members: Vec<(String, String)> =
        sqlx::query_as("SELECT group_id, user_id FROM group_members")
            .fetch_all(&app.pool)
            .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, clients)| Group {
                members: members
                    .iter()
                    .filter(|(g, _)| *g == id)
                    .map(|(_, u)| u.clone())
                    .collect(),
                id,
                name,
                clients,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct GroupIn {
    name: String,
    #[serde(default)]
    members: Vec<String>,
}

fn group_name(s: &str) -> Result<String, ApiError> {
    let n = s.trim();
    if n.is_empty() || n.chars().count() > 100 {
        Err(ApiError::BadRequest(
            "название группы: 1–100 символов".into(),
        ))
    } else {
        Ok(n.to_owned())
    }
}

fn unique(e: sqlx::Error) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            ApiError::Conflict("группа с таким названием уже есть")
        }
        _ => ApiError::Db(e),
    }
}

async fn set_members(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    group: &str,
    members: &[String],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM group_members WHERE group_id = ?")
        .bind(group)
        .execute(&mut **tx)
        .await?;
    for u in members {
        let ok = sqlx::query("INSERT OR IGNORE INTO group_members(group_id, user_id) SELECT ?, id FROM users WHERE id = ?")
            .bind(group)
            .bind(u)
            .execute(&mut **tx)
            .await?
            .rows_affected();
        if ok == 0 {
            return Err(ApiError::BadRequest("неизвестный сотрудник".into()));
        }
    }
    Ok(())
}

pub async fn create_group(
    State(app): State<AppState>,
    Owner(_): Owner,
    Json(req): Json<GroupIn>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let name = group_name(&req.name)?;
    let id = new_id();
    let mut tx = crate::db::write_tx(&app.pool).await?;
    sqlx::query("INSERT INTO groups(id, name, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(now_ms())
        .execute(&mut *tx)
        .await
        .map_err(unique)?;
    set_members(&mut tx, &id, &req.members).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

pub async fn update_group(
    State(app): State<AppState>,
    Owner(_): Owner,
    Path(id): Path<String>,
    Json(req): Json<GroupIn>,
) -> Result<StatusCode, ApiError> {
    let name = group_name(&req.name)?;
    let mut tx = crate::db::write_tx(&app.pool).await?;
    let n = sqlx::query("UPDATE groups SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(unique)?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    set_members(&mut tx, &id, &req.members).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_group(
    State(app): State<AppState>,
    Owner(_): Owner,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if id == "default" {
        return Err(ApiError::Conflict("основную группу удалить нельзя"));
    }
    let clients: i64 = sqlx::query_scalar("SELECT count(*) FROM clients WHERE group_id = ?")
        .bind(&id)
        .fetch_one(&app.pool)
        .await?;
    if clients > 0 {
        return Err(ApiError::Conflict(
            "в группе есть клиенты — сначала перенесите их в другую группу",
        ));
    }
    let n = sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await?
        .rows_affected();
    if n == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}
