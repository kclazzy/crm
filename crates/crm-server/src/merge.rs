//! Импорт базы с другого сервера (переезд): с заменой — как восстановление из копии,
//! или «добавить к существующим данным» — слияние без дублей.
//!
//! Слияние: сотрудники совпадают по логину, владелец — с владельцем, группы и теги — по
//! названию; всё остальное (клиенты, задачи, чаты…) добавляется со своими id, ссылки на
//! совпавших сотрудников, группы и теги переписываются. Что уже есть (тот же id) —
//! пропускается, поэтому повторный импорт ничего не удваивает. Всё — одной транзакцией.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{AssertSqlSafe, Connection, SqliteConnection};

use crate::auth::{audit, Owner};
use crate::http::{ApiError, AppState};

/// Таблицы, которые не переносятся: входы, приглашения, настройки этого сервера,
/// служебные; сотрудники, группы и теги — отдельно, с сопоставлением.
const SKIP: [&str; 12] = [
    "_sqlx_migrations",
    // Коды подключения действуют только на своём сервере.
    "enroll_codes",
    // Одобренные устройства другого сервера сюда не переносятся.
    "p2p_devices",
    "users",
    "groups",
    "tags",
    "settings",
    "sessions",
    "invites",
    "recovery_codes",
    "disk_usage_snapshots",
    "totp_backup",
];
/// Сопоставляемые таблицы: ссылки на них переписываются через temp.map_*.
const MAPPED: [&str; 3] = ["users", "groups", "tags"];

#[derive(Deserialize)]
pub struct ImportIn {
    source: String,
    /// `replace` — заменить данные этого сервера; `merge` — добавить к ним.
    mode: String,
}

pub async fn import_db(
    State(app): State<AppState>,
    Owner(owner): Owner,
    Json(req): Json<ImportIn>,
) -> Result<Json<Value>, ApiError> {
    let source = PathBuf::from(req.source.trim());
    if !source.is_absolute() || !source.exists() {
        return Err(ApiError::BadRequest(format!(
            "не найдено: {} — укажите файл копии (.db.zst), пакет переноса (.crmpack) или папку копий",
            source.display()
        )));
    }
    let storage = app.rt.config().storage;
    match req.mode.as_str() {
        "replace" => {
            let staged = crate::restore::stage(&storage, &source)
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            app.rt
                .frozen
                .store(true, std::sync::atomic::Ordering::SeqCst);
            audit(
                &app.pool,
                Some(&owner.id),
                "import.replace",
                Some(&staged.name),
                "local",
            )
            .await?;
            Ok(Json(json!({ "restart": true, "staged": staged })))
        }
        "merge" => {
            let report = merge(&app, &storage, &source).await?;
            audit(
                &app.pool,
                Some(&owner.id),
                "import.merge",
                Some(&source.display().to_string()),
                "local",
            )
            .await?;
            Ok(Json(
                serde_json::to_value(report).map_err(|e| ApiError::Failed(e.to_string()))?,
            ))
        }
        _ => Err(ApiError::BadRequest("режим: replace или merge".into())),
    }
}

#[derive(Serialize)]
pub struct Report {
    /// Сколько добавлено: clients, tasks, messages, users…
    added: BTreeMap<String, u64>,
    /// Сотрудники, совпавшие с уже существующими (владелец и одинаковые логины).
    merged_users: u64,
    attachments: usize,
}

fn fail(e: impl std::fmt::Display) -> ApiError {
    ApiError::BadRequest(format!("импорт не выполнен: {e}"))
}

/// Распаковать копию во временную папку и привести её к схеме этой версии.
async fn prepare(work: &Path, source: &Path) -> Result<(PathBuf, PathBuf), ApiError> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(fail)?;
    let is_pack = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("crmpack"));
    let dir = if is_pack {
        let to = work.join("pack");
        crate::restore::unzip(source, &to).map_err(fail)?;
        to
    } else if source.is_dir() {
        source.to_owned()
    } else {
        source.parent().unwrap_or(Path::new(".")).to_owned()
    };
    let file = if is_pack || source.is_dir() {
        let mut files = crate::backup::list_dir(&dir).map_err(fail)?;
        files.sort_by_key(|f| std::cmp::Reverse(f.at));
        let f = files.first().ok_or_else(|| fail("в папке нет копий CRM"))?;
        dir.join(&f.name)
    } else {
        source.to_owned()
    };
    let packed = std::fs::read(&file).map_err(fail)?;
    let raw =
        zstd::decode_all(packed.as_slice()).map_err(|_| fail("это не копия CRM (.db.zst)"))?;
    let db = work.join("source.db");
    std::fs::write(&db, raw).map_err(fail)?;
    crate::backup::check_integrity(&db).await.map_err(fail)?;
    // Копия могла быть с более старой версии — догоняем схему до этой.
    let pool = sqlx::SqlitePool::connect_with(SqliteConnectOptions::new().filename(&db))
        .await
        .map_err(fail)?;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .map_err(|e| {
            fail(format!(
                "копия от другой версии CRM ({e}) — обновите этот сервер до той же версии"
            ))
        })?;
    pool.close().await;
    Ok((db, dir))
}

/// Таблицы в порядке ссылок: сначала те, на кого ссылаются.
async fn tables_in_order(
    conn: &mut SqliteConnection,
) -> Result<Vec<(String, HashMap<String, String>)>, ApiError> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM main.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         AND sql NOT LIKE 'CREATE VIRTUAL%' AND name NOT LIKE 'search_index%' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut fks: HashMap<String, HashMap<String, String>> = HashMap::new();
    for t in &names {
        let rows: Vec<(i64, i64, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, seq, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?)",
        )
        .bind(t)
        .fetch_all(&mut *conn)
        .await?;
        fks.insert(t.clone(), rows.into_iter().map(|r| (r.3, r.2)).collect());
    }
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    fn visit(
        t: &str,
        fks: &HashMap<String, HashMap<String, String>>,
        seen: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) {
        if !seen.insert(t.to_owned()) {
            return;
        }
        if let Some(refs) = fks.get(t) {
            for to in refs.values() {
                if to != t {
                    visit(to, fks, seen, order);
                }
            }
        }
        order.push(t.to_owned());
    }
    for t in &names {
        visit(t, &fks, &mut seen, &mut order);
    }
    Ok(order
        .into_iter()
        .filter(|t| names.contains(t) && !SKIP.contains(&t.as_str()))
        .map(|t| {
            let f = fks.remove(&t).unwrap_or_default();
            (t, f)
        })
        .collect())
}

async fn columns(conn: &mut SqliteConnection, table: &str) -> Result<Vec<String>, ApiError> {
    Ok(sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(&mut *conn)
        .await?)
}

async fn merge(
    app: &AppState,
    storage: &crate::config::Storage,
    source: &Path,
) -> Result<Report, ApiError> {
    let work = storage.db_dir.join("import-merge");
    let (db, dir) = prepare(&work, source).await?;
    let result = merge_db(app, &db).await;
    let attachments = match &result {
        Ok(_) => crate::backup::copy_new_files(&dir.join("attachments"), &storage.attachments_dir)
            .map_err(fail)?,
        Err(_) => 0,
    };
    let _ = std::fs::remove_dir_all(&work);
    let (added, merged_users) = result?;
    Ok(Report {
        added,
        merged_users,
        attachments,
    })
}

async fn merge_db(app: &AppState, db: &Path) -> Result<(BTreeMap<String, u64>, u64), ApiError> {
    let mut conn = app.pool.acquire().await?;
    sqlx::query("ATTACH DATABASE ? AS src")
        .bind(db.to_string_lossy().into_owned())
        .execute(&mut *conn)
        .await?;
    let r = merge_attached(&mut conn).await;
    let _ = sqlx::query("DETACH DATABASE src").execute(&mut *conn).await;
    r
}

async fn merge_attached(
    conn: &mut SqliteConnection,
) -> Result<(BTreeMap<String, u64>, u64), ApiError> {
    let mut tx = conn.begin().await?;
    // Ссылки проверяются в конце: порядок вставки внутри таблицы (подзадачи) не важен.
    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *tx)
        .await?;
    for m in MAPPED {
        sqlx::query(AssertSqlSafe(format!("DROP TABLE IF EXISTS temp.map_{m}")))
            .execute(&mut *tx)
            .await?;
        sqlx::query(AssertSqlSafe(format!(
            "CREATE TEMP TABLE map_{m} (old TEXT PRIMARY KEY, new TEXT NOT NULL)"
        )))
        .execute(&mut *tx)
        .await?;
    }
    // Сотрудники: владелец — с владельцем, остальные — по логину.
    sqlx::query(
        "INSERT INTO temp.map_users(old, new)
         SELECT s.id, COALESCE(
             CASE WHEN s.role = 'host' THEN (SELECT id FROM main.users WHERE role = 'host' LIMIT 1) END,
             (SELECT m.id FROM main.users m WHERE m.username = s.username COLLATE NOCASE),
             s.id)
         FROM src.users s",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO temp.map_groups(old, new)
         SELECT s.id, COALESCE((SELECT m.id FROM main.groups m WHERE m.name = s.name), s.id) FROM src.groups s",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO temp.map_tags(old, new)
         SELECT s.id, COALESCE((SELECT m.id FROM main.tags m WHERE m.name = s.name COLLATE NOCASE), s.id) FROM src.tags s",
    )
    .execute(&mut *tx)
    .await?;
    let merged_users: i64 =
        sqlx::query_scalar("SELECT count(*) FROM temp.map_users WHERE old <> new")
            .fetch_one(&mut *tx)
            .await?;

    let mut added = BTreeMap::new();
    // Новые сотрудники, группы, теги — как есть.
    for m in MAPPED {
        let cols = columns(&mut tx, m).await?.join(", ");
        let n = sqlx::query(AssertSqlSafe(format!(
            "INSERT OR IGNORE INTO main.\"{m}\" ({cols})
             SELECT {cols} FROM src.\"{m}\" s WHERE (SELECT new FROM temp.map_{m} WHERE old = s.id) = s.id"
        )))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        added.insert(m.to_owned(), n);
    }
    // Остальное — со ссылками на совпавших сотрудников, группы и теги.
    for (table, fks) in tables_in_order(&mut tx).await? {
        let cols = columns(&mut tx, &table).await?;
        let select: Vec<String> = cols
            .iter()
            .map(|c| match fks.get(c).map(String::as_str) {
                Some(to) if MAPPED.contains(&to) => {
                    format!("COALESCE((SELECT new FROM temp.map_{to} WHERE old = s.\"{c}\"), s.\"{c}\")")
                }
                _ => format!("s.\"{c}\""),
            })
            .collect();
        let list = cols
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let n = sqlx::query(AssertSqlSafe(format!(
            "INSERT OR IGNORE INTO main.\"{table}\" ({list}) SELECT {} FROM src.\"{table}\" s",
            select.join(", ")
        )))
        .execute(&mut *tx)
        .await
        .map_err(|e| fail(format!("{table}: {e}")))?
        .rows_affected();
        added.insert(table, n);
    }
    tx.commit()
        .await
        .map_err(|e| fail(format!("ссылки не сошлись: {e}")))?;
    Ok((added, u64::try_from(merged_users).unwrap_or_default()))
}
