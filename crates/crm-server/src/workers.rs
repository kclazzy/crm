//! Фоновые работы: напоминания о дедлайнах, автоархив, уборка вложений,
//! снимки диска и тревоги владельцу (ТЗ §2.7, §4.4–4.5, §5).

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use crm_core::reminders::{due, plan, ReminderKind};
use crm_core::task::{archive_candidates, ArchiveNode, Priority};

use crate::auth::now_ms;
use crate::http::{ApiError, AppState};
use crate::notify;

const TICK: Duration = Duration::from_secs(60);
const HOUR_MS: i64 = 3600 * 1000;

/// Раз в минуту — напоминания; раз в час — всё остальное.
pub async fn run(app: AppState) {
    let mut tick = tokio::time::interval(TICK);
    let mut last_hourly: Option<i64> = None;
    loop {
        tick.tick().await;
        if let Err(e) = reminders(&app).await {
            tracing::error!(error = %e, "напоминания");
        }
        let now = now_ms();
        if last_hourly.is_none_or(|t| now - t >= HOUR_MS) {
            last_hourly = Some(now);
            hourly(&app).await;
        }
    }
}

pub async fn hourly(app: &AppState) {
    if let Err(e) = archive(app).await {
        tracing::error!(error = %e, "автоархив");
    }
    if let Err(e) = crate::attachments::collect_garbage(app).await {
        tracing::error!(error = %e, "уборка вложений");
    }
    if let Err(e) = health(app).await {
        tracing::error!(error = %e, "проверка диска и копий");
    }
}

fn kind_of(s: &str) -> Option<ReminderKind> {
    Some(match s {
        "h24" => ReminderKind::H24,
        "h4" => ReminderKind::H4,
        "h1" => ReminderKind::H1,
        "overdue" => ReminderKind::Overdue,
        _ => return None,
    })
}

#[derive(sqlx::FromRow)]
struct Open {
    task_id: String,
    user_id: String,
    notify_always: bool,
    title: String,
    client_name: String,
    priority: i64,
    deadline_at: i64,
}

/// Напоминания исполнителям: идемпотентно по `reminders_sent`, с учётом тихих часов.
pub async fn reminders(app: &AppState) -> Result<usize, ApiError> {
    let org = crate::org::load(&app.pool).await?;
    let schedule = org.schedule();
    let rows: Vec<Open> = sqlx::query_as(
        "SELECT t.id AS task_id, u.id AS user_id, u.notify_always, t.title, c.name AS client_name,
                t.priority, t.deadline_at
         FROM tasks t JOIN clients c ON c.id = t.client_id
         JOIN task_assignees a ON a.task_id = t.id JOIN users u ON u.id = a.user_id
         WHERE t.status <> 'completed' AND t.archived_at IS NULL AND t.deleted_at IS NULL
           AND c.deleted_at IS NULL AND u.is_active = 1",
    )
    .fetch_all(&app.pool)
    .await?;
    let sent_rows: Vec<(String, String, i64, String)> =
        sqlx::query_as("SELECT task_id, user_id, deadline_at, kind FROM reminders_sent")
            .fetch_all(&app.pool)
            .await?;
    let mut sent: HashMap<(String, String, i64), Vec<ReminderKind>> = HashMap::new();
    for (t, u, d, k) in sent_rows {
        if let Some(k) = kind_of(&k) {
            sent.entry((t, u, d)).or_default().push(k);
        }
    }
    let now = Utc::now();
    let mut delivered = 0;
    for r in rows {
        let (Ok(priority), Some(deadline)) = (
            Priority::try_from(r.priority),
            DateTime::from_timestamp_millis(r.deadline_at),
        ) else {
            continue;
        };
        let quiet = if r.notify_always {
            None
        } else {
            Some(schedule)
        };
        let p = plan(priority, deadline, quiet);
        let key = (r.task_id.clone(), r.user_id.clone(), r.deadline_at);
        let already = sent.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let Some(d) = due(&p, now, already) else {
            continue;
        };
        let mut first = false;
        for k in &d.mark_sent {
            let n = sqlx::query(
                "INSERT OR IGNORE INTO reminders_sent(task_id, user_id, kind, deadline_at, sent_at) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&r.task_id)
            .bind(&r.user_id)
            .bind(k.as_str())
            .bind(r.deadline_at)
            .bind(now_ms())
            .execute(&app.pool)
            .await?
            .rows_affected();
            first |= *k == d.send && n > 0;
        }
        if first {
            notify::push(
                app,
                &r.user_id,
                Some(&r.task_id),
                "reminder",
                serde_json::json!({
                    "reminder": d.send.as_str(),
                    "title": r.title,
                    "client": r.client_name,
                    "deadline_at": r.deadline_at,
                }),
            )
            .await?;
            delivered += 1;
        }
    }
    Ok(delivered)
}

/// Автоархив завершённых задач старше N дней (всё поддерево).
pub async fn archive(app: &AppState) -> Result<usize, ApiError> {
    let org = crate::org::load(&app.pool).await?;
    let rows: Vec<(String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT id, parent_task_id, completed_at FROM tasks WHERE deleted_at IS NULL AND archived_at IS NULL",
    )
    .fetch_all(&app.pool)
    .await?;
    let nodes: Vec<ArchiveNode> = rows
        .into_iter()
        .map(|(id, parent, completed_at)| ArchiveNode {
            id,
            parent,
            completed_at,
        })
        .collect();
    let older_than = now_ms() - org.archive_after_days() * 24 * HOUR_MS;
    let ids = archive_candidates(&nodes, older_than);
    let now = now_ms();
    let mut tx = app.pool.begin().await?;
    for id in &ids {
        sqlx::query("UPDATE tasks SET archived_at = ? WHERE id = ? AND archived_at IS NULL AND status = 'completed'")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if !ids.is_empty() {
        tracing::info!(count = ids.len(), "задачи ушли в архив");
    }
    Ok(ids.len())
}

async fn owners(app: &AppState) -> Result<Vec<String>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT id FROM users WHERE role = 'host' AND is_active = 1")
            .fetch_all(&app.pool)
            .await?,
    )
}

/// Тревоги владельцу: диск заполняется, копии не делаются.
pub async fn health(app: &AppState) -> Result<(), ApiError> {
    let vols = crate::admin::snapshot_disks(app).await?;
    let owners = owners(app).await?;
    for v in &vols {
        let (kind, every) = match v.level {
            crm_core::disk::DiskLevel::Critical => ("disk_critical", 24 * HOUR_MS),
            crm_core::disk::DiskLevel::Warning => ("disk_warning", 72 * HOUR_MS),
            crm_core::disk::DiskLevel::Ok => continue,
        };
        for o in &owners {
            if !notify::recently(&app.pool, o, kind, every).await? {
                notify::push(
                    app,
                    o,
                    None,
                    kind,
                    serde_json::json!({ "volume": v.path, "days_left": v.days_left }),
                )
                .await?;
            }
        }
    }
    let cfg = app.rt.config();
    if cfg.backup.enabled() && !cfg.storage.backup_dirs.is_empty() {
        let last = crate::backup::last_status(&app.pool).await;
        let stale_after = i64::from(cfg.backup.every_hours()) * HOUR_MS + 2 * HOUR_MS;
        let failing = last.as_ref().is_some_and(|s| !s.ok);
        let stale = last
            .as_ref()
            .is_some_and(|s| s.ok && now_ms() - s.at > stale_after);
        if failing || stale {
            for o in &owners {
                if !notify::recently(&app.pool, o, "backup_problem", 6 * HOUR_MS).await? {
                    notify::push(
                        app,
                        o,
                        None,
                        "backup_problem",
                        serde_json::json!({ "error": last.as_ref().and_then(|s| s.error.clone()) }),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}
