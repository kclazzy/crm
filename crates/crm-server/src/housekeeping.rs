//! Уборка базы, чтобы она не разрасталась годами:
//! - служебные записи (старые уведомления, журнал, закончившиеся входы);
//! - корзина — только если владелец сам включил автоочистку;
//! - обслуживание: статистика для запросов и сжатие файла после крупных удалений.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use axum::extract::State;
use axum::Json;

use crate::auth::{now_ms, Owner};
use crate::http::{ApiError, AppState};

const DAY: i64 = 24 * 3600 * 1000;
/// Уведомления хранятся 90 дней, «удалённые» (скрытые) — ещё 30 дней.
const NOTES_DAYS: i64 = 90;
const HIDDEN_DAYS: i64 = 30;
/// Журнал безопасности — год.
const AUDIT_DAYS: i64 = 365;
/// Закончившиеся и отозванные входы — 30 дней (видны в «Активных входах» как история).
const SESSIONS_DAYS: i64 = 30;
/// Сроки автоочистки корзины, из которых выбирает владелец.
pub const TRASH_DAYS: [u32; 5] = [30, 60, 90, 180, 365];
/// С какого возраста записи в корзине — повод предложить автоочистку.
const SUGGEST_AFTER_DAYS: i64 = 90;
/// «Не сейчас» — снова предложить через 30 дней.
const SNOOZE_DAYS: i64 = 30;

const TRASH_KEY: &str = "trash.auto_days";
const SNOOZE_KEY: &str = "trash.suggest_dismissed_at";
const TIDY_KEY: &str = "housekeeping.tidy_at";
const MAINTAIN_KEY: &str = "housekeeping.maintain_at";

#[derive(Debug, Default, Serialize)]
pub struct Tidied {
    /// Сколько служебных записей убрано.
    pub removed: u64,
    /// Сколько задач и клиентов удалено из корзины навсегда.
    pub purged: u64,
}

/// Уборка служебных записей и (если включено) корзины.
pub async fn tidy(app: &AppState) -> Result<Tidied, ApiError> {
    let now = now_ms();
    let pool = &app.pool;
    let mut out = Tidied::default();
    let jobs: [(&str, i64); 6] = [
        (
            "DELETE FROM notifications WHERE created_at < ?",
            now - NOTES_DAYS * DAY,
        ),
        (
            "DELETE FROM notifications WHERE hidden_at < ?",
            now - HIDDEN_DAYS * DAY,
        ),
        (
            "DELETE FROM audit_log WHERE created_at < ?",
            now - AUDIT_DAYS * DAY,
        ),
        (
            "DELETE FROM audit_log WHERE hidden_at < ?",
            now - HIDDEN_DAYS * DAY,
        ),
        (
            "DELETE FROM sessions WHERE expires_at < ?",
            now - SESSIONS_DAYS * DAY,
        ),
        (
            "DELETE FROM sessions WHERE revoked_at < ?",
            now - SESSIONS_DAYS * DAY,
        ),
    ];
    for (sql, before) in jobs {
        out.removed += sqlx::query(sql)
            .bind(before)
            .execute(pool)
            .await?
            .rows_affected();
    }
    // Отметки отправленных напоминаний нужны, пока задача жива: у завершённых
    // и удалённых больше месяца назад — не нужны.
    out.removed += sqlx::query(
        "DELETE FROM reminders_sent WHERE task_id IN
           (SELECT id FROM tasks WHERE (status = 'completed' OR deleted_at IS NOT NULL)
              AND coalesce(deleted_at, completed_at) < ?)",
    )
    .bind(now - SESSIONS_DAYS * DAY)
    .execute(pool)
    .await?
    .rows_affected();

    if let Some(days) = trash_days(pool).await? {
        let before = now - i64::from(days) * DAY;
        let tasks = sqlx::query("DELETE FROM tasks WHERE deleted_at < ?")
            .bind(before)
            .execute(pool)
            .await?
            .rows_affected();
        let clients = sqlx::query("DELETE FROM clients WHERE deleted_at < ?")
            .bind(before)
            .execute(pool)
            .await?
            .rows_affected();
        out.purged = tasks + clients;
        if out.purged > 0 {
            crate::attachments::collect_garbage(app).await?;
            crate::auth::audit(
                pool,
                None,
                "trash.auto_purged",
                Some(&out.purged.to_string()),
                "local",
            )
            .await?;
        }
    }
    set(pool, TIDY_KEY, &now.to_string()).await?;
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct Maintained {
    pub vacuumed: bool,
    pub pages: i64,
    pub free_pages: i64,
}

/// Сжимать, когда пустого места больше четверти файла и оно не крошечное (> 4 МБ).
pub fn needs_vacuum(pages: i64, free_pages: i64, page_size: i64) -> bool {
    pages > 0 && free_pages * 4 > pages && free_pages * page_size > 4 << 20
}

/// Обслуживание: статистика для планировщика запросов и, если нужно, сжатие файла.
/// Сжатие на время блокирует запись — поэтому по расписанию оно идёт ночью.
pub async fn maintain(app: &AppState) -> Result<Maintained, ApiError> {
    let _heavy = app.rt.heavy.acquire().await;
    let pool = &app.pool;
    sqlx::query("PRAGMA optimize").execute(pool).await?;
    let (pages, free_pages, page_size) = sizes(pool).await?;
    let vacuumed = needs_vacuum(pages, free_pages, page_size);
    if vacuumed {
        // Копия не должна сниматься посреди сжатия.
        let _backup = app.rt.backup_lock.lock().await;
        sqlx::query("VACUUM").execute(pool).await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(pool)
            .await?;
        tracing::info!(pages, free_pages, "база сжата");
    }
    set(pool, MAINTAIN_KEY, &now_ms().to_string()).await?;
    let (pages, free_pages, _) = sizes(pool).await?;
    Ok(Maintained {
        vacuumed,
        pages,
        free_pages,
    })
}

async fn sizes(pool: &SqlitePool) -> Result<(i64, i64, i64), ApiError> {
    Ok(sqlx::query_as(
        "SELECT (SELECT page_count FROM pragma_page_count()),
                (SELECT freelist_count FROM pragma_freelist_count()),
                (SELECT page_size FROM pragma_page_size())",
    )
    .fetch_one(pool)
    .await?)
}

/// Раз в сутки — уборка; раз в неделю ночью (2–5 ч по времени организации) — обслуживание.
pub async fn daily(app: &AppState) -> Result<(), ApiError> {
    let now = now_ms();
    if get_ms(&app.pool, TIDY_KEY)
        .await?
        .is_none_or(|t| now - t >= DAY)
    {
        tidy(app).await?;
    }
    let hour = {
        use chrono::Timelike;
        let tz = crate::org::load(&app.pool).await?.tz();
        chrono::Utc::now().with_timezone(&tz).hour()
    };
    let week_passed = get_ms(&app.pool, MAINTAIN_KEY)
        .await?
        .is_none_or(|t| now - t >= 7 * DAY);
    if week_passed && (2..=5).contains(&hour) {
        maintain(app).await?;
    }
    Ok(())
}

async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?,
    )
}

async fn get_ms(pool: &SqlitePool, key: &str) -> Result<Option<i64>, ApiError> {
    Ok(get(pool, key).await?.and_then(|v| v.parse().ok()))
}

async fn set(pool: &SqlitePool, key: &str, value: &str) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

async fn trash_days(pool: &SqlitePool) -> Result<Option<u32>, ApiError> {
    Ok(get(pool, TRASH_KEY)
        .await?
        .and_then(|v| v.parse().ok())
        .filter(|d| TRASH_DAYS.contains(d)))
}

// ---------- Настройка автоочистки корзины (владелец) ----------

#[derive(Serialize)]
pub struct TrashPolicy {
    /// Через сколько дней удалять из корзины навсегда; `null` — не удалять.
    days: Option<u32>,
    choices: [u32; 5],
    /// Сколько записей лежит в корзине дольше 90 дней.
    old_items: i64,
    /// Подсказать владельцу включить автоочистку.
    suggest: bool,
}

pub async fn get_trash_policy(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<TrashPolicy>, ApiError> {
    let pool = &app.pool;
    let now = now_ms();
    let before = now - SUGGEST_AFTER_DAYS * DAY;
    let old_items: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM tasks WHERE deleted_at < ?1)
              + (SELECT count(*) FROM clients WHERE deleted_at < ?1)",
    )
    .bind(before)
    .fetch_one(pool)
    .await?;
    let days = trash_days(pool).await?;
    let snoozed = get_ms(pool, SNOOZE_KEY)
        .await?
        .is_some_and(|t| now - t < SNOOZE_DAYS * DAY);
    Ok(Json(TrashPolicy {
        days,
        choices: TRASH_DAYS,
        old_items,
        suggest: days.is_none() && old_items > 0 && !snoozed,
    }))
}

#[derive(Deserialize)]
pub struct TrashPolicyIn {
    days: Option<u32>,
}

pub async fn set_trash_policy(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<TrashPolicyIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match req.days {
        Some(d) if !TRASH_DAYS.contains(&d) => {
            return Err(ApiError::BadRequest(
                "срок: 30, 60, 90, 180 или 365 дней".into(),
            ))
        }
        Some(d) => set(&app.pool, TRASH_KEY, &d.to_string()).await?,
        None => {
            sqlx::query("DELETE FROM settings WHERE key = ?")
                .bind(TRASH_KEY)
                .execute(&app.pool)
                .await?;
        }
    }
    crate::auth::audit(
        &app.pool,
        Some(&owner.id),
        "trash.policy",
        Some(&req.days.map_or("выкл".to_owned(), |d| format!("{d} дн."))),
        "local",
    )
    .await?;
    Ok(Json(serde_json::json!({ "days": req.days })))
}

/// «Не сейчас»: не предлагать автоочистку 30 дней.
pub async fn dismiss_trash_suggestion(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<serde_json::Value>, ApiError> {
    set(&app.pool, SNOOZE_KEY, &now_ms().to_string()).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacuum_only_when_a_lot_of_space_is_free() {
        let page = 4096;
        assert!(!needs_vacuum(0, 0, page));
        assert!(
            !needs_vacuum(10_000, 1_000, page),
            "10% пустого — не трогаем"
        );
        assert!(needs_vacuum(10_000, 3_000, page), "30% и 12 МБ — сжать");
        assert!(
            !needs_vacuum(1_000, 900, page),
            "пусто, но всего 3,6 МБ — не стоит"
        );
    }
}
