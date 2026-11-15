//! Импорт клиентов из Excel/CSV и выгрузка в Excel/CSV (ТЗ §4.1.1).

use std::collections::HashMap;
use std::io::Cursor;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use crm_core::access::can_create_client;
use crm_core::auth::Role;
use crm_core::import::{
    delimiter, guess_company, guess_mapping, norm_email, norm_name, norm_phone, split_tags, Field,
};
use rust_xlsxwriter::{Format, Workbook};
use serde::{Deserialize, Serialize};

use crate::access::Scope;
use crate::auth::{new_id, now_ms, CurrentUser};
use crate::clients::{ClientIn, Kind};
use crate::http::{ApiError, AppState};

/// Строка выгрузки как она читается из БД.
type ClientExportRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
);
/// Строка выгрузки как она читается из БД.
type ClientDumpRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    i64,
);
/// Строка выгрузки как она читается из БД.
type TaskDumpRow = (
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    String,
    i64,
    i64,
    Option<i64>,
);

const MAX_ROWS: usize = 10_000;
/// Импорт можно отменить в течение суток.
const UNDO_WINDOW_MS: i64 = 24 * 3600 * 1000;

fn require_manager(user: &CurrentUser) -> Result<(), ApiError> {
    if user.role == Role::Employee {
        Err(ApiError::Forbidden)
    } else {
        Ok(())
    }
}

// ---------- Чтение файла ----------

#[derive(Serialize)]
pub struct Parsed {
    file_name: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    mapping: HashMap<Field, usize>,
}

fn cell_text(c: &calamine::Data) -> String {
    use calamine::Data;
    match c {
        Data::Empty => String::new(),
        // Телефоны в Excel часто числа: 79001234567.0 → «79001234567».
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        other => other.to_string(),
    }
}

/// Таблица из файла: первая строка — заголовки.
pub fn read_table(name: &str, bytes: &[u8]) -> Result<Vec<Vec<String>>, ApiError> {
    let lower = name.to_lowercase();
    // Excel узнаём по содержимому (ZIP или старый формат OLE), а не только по имени.
    let excel = bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0])
        || lower.ends_with(".xlsx")
        || lower.ends_with(".xls");
    let rows: Vec<Vec<String>> = if excel {
        use calamine::Reader;
        let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
            .map_err(|e| ApiError::BadRequest(format!("не удалось прочитать Excel: {e}")))?;
        let sheet = wb
            .worksheet_range_at(0)
            .ok_or_else(|| ApiError::BadRequest("в файле нет листов".into()))?
            .map_err(|e| ApiError::BadRequest(format!("не удалось прочитать лист: {e}")))?;
        sheet
            .rows()
            .map(|r| r.iter().map(cell_text).collect())
            .collect()
    } else {
        let text = match std::str::from_utf8(bytes) {
            Ok(s) => s.trim_start_matches('\u{feff}').to_owned(),
            Err(_) => encoding_rs::WINDOWS_1251.decode(bytes).0.into_owned(),
        };
        let first = text.lines().next().unwrap_or("");
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delimiter(first))
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut out = Vec::new();
        for rec in rdr.records() {
            let rec = rec.map_err(|e| ApiError::BadRequest(format!("ошибка в CSV: {e}")))?;
            out.push(rec.iter().map(str::to_owned).collect());
        }
        out
    };
    let rows: Vec<Vec<String>> = rows
        .into_iter()
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .collect();
    if rows.len() > MAX_ROWS + 1 {
        return Err(ApiError::BadRequest(format!(
            "слишком много строк: не больше {MAX_ROWS} за раз"
        )));
    }
    Ok(rows)
}

/// Шаг 1: прочитать файл, предложить сопоставление колонок.
pub async fn parse(
    _: State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Parsed>, ApiError> {
    require_manager(&user)?;
    let name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(crate::attachments::percent_decode)
        .unwrap_or_else(|| "clients.csv".into());
    let mut rows = read_table(&name, &body)?;
    if rows.is_empty() {
        return Err(ApiError::BadRequest("файл пустой".into()));
    }
    let headers = rows.remove(0);
    let mapping = guess_mapping(&headers).into_iter().collect();
    Ok(Json(Parsed {
        file_name: crate::attachments::clean_name(&name),
        headers,
        rows,
        mapping,
    }))
}

// ---------- Проверка ----------

#[derive(Deserialize, Clone, Default)]
pub struct Record {
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Option<String>,
    #[serde(default)]
    manager: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Duplicate {
    id: String,
    name: String,
    /// «название», «телефон», «email».
    by: &'static str,
}

#[derive(Serialize)]
pub struct RowCheck {
    errors: Vec<String>,
    duplicate: Option<Duplicate>,
}

#[derive(Deserialize)]
pub struct CheckIn {
    rows: Vec<Record>,
    #[serde(default = "default_group")]
    group_id: String,
}

fn default_group() -> String {
    "default".into()
}

struct Index {
    by_name: HashMap<String, (String, String)>,
    by_phone: HashMap<String, (String, String)>,
    by_email: HashMap<String, (String, String)>,
    managers: HashMap<String, String>,
}

async fn index(app: &AppState) -> Result<Index, ApiError> {
    let rows: Vec<(String, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, name, phone, email FROM clients WHERE deleted_at IS NULL")
            .fetch_all(&app.pool)
            .await?;
    let mut ix = Index {
        by_name: HashMap::new(),
        by_phone: HashMap::new(),
        by_email: HashMap::new(),
        managers: HashMap::new(),
    };
    for (id, name, phone, email) in rows {
        let v = (id, name.clone());
        ix.by_name.entry(norm_name(&name)).or_insert(v.clone());
        if let Some(p) = phone.as_deref().and_then(norm_phone) {
            ix.by_phone.entry(p).or_insert(v.clone());
        }
        if let Some(e) = email.as_deref().and_then(norm_email) {
            ix.by_email.entry(e).or_insert(v);
        }
    }
    let managers: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, username, display_name FROM users WHERE is_active = 1 AND role IN ('host','manager')",
    )
    .fetch_all(&app.pool)
    .await?;
    for (id, username, display) in managers {
        ix.managers.insert(username.to_lowercase(), id.clone());
        ix.managers.insert(display.to_lowercase(), id);
    }
    Ok(ix)
}

impl Index {
    fn duplicate(&self, r: &Record) -> Option<Duplicate> {
        let hit = |m: &HashMap<String, (String, String)>, k: Option<String>, by| {
            k.and_then(|k| m.get(&k)).map(|(id, name)| Duplicate {
                id: id.clone(),
                name: name.clone(),
                by,
            })
        };
        hit(
            &self.by_name,
            Some(norm_name(&r.name)).filter(|n| !n.is_empty()),
            "название",
        )
        .or_else(|| {
            hit(
                &self.by_phone,
                r.phone.as_deref().and_then(norm_phone),
                "телефон",
            )
        })
        .or_else(|| {
            hit(
                &self.by_email,
                r.email.as_deref().and_then(norm_email),
                "email",
            )
        })
    }

    /// Запись файла → карточка клиента; ошибки — понятным текстом.
    fn to_client(&self, r: &Record, group: &str) -> Result<ClientIn, Vec<String>> {
        let mut errors = Vec::new();
        let manager_id = match r
            .manager
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            Some(m) => match self.managers.get(&m.to_lowercase()) {
                Some(id) => Some(id.clone()),
                None => {
                    errors.push(format!("неизвестный менеджер «{m}»"));
                    None
                }
            },
            None => None,
        };
        let input = ClientIn {
            kind: if guess_company(r.kind.as_deref(), &r.name) {
                Kind::Company
            } else {
                Kind::Person
            },
            name: r.name.clone(),
            phone: r.phone.clone(),
            email: r.email.clone(),
            description: r.description.clone().unwrap_or_default(),
            group_id: group.to_owned(),
            manager_id,
            tags: r.tags.as_deref().map(split_tags).unwrap_or_default(),
        };
        if let Err(ApiError::BadRequest(e)) = input.clone().validate() {
            errors.push(e);
        }
        if errors.is_empty() {
            Ok(input)
        } else {
            Err(errors)
        }
    }
}

/// Шаг 3: ошибки по строкам и дубли.
pub async fn check(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CheckIn>,
) -> Result<Json<Vec<RowCheck>>, ApiError> {
    require_manager(&user)?;
    if req.rows.len() > MAX_ROWS {
        return Err(ApiError::BadRequest(format!("не больше {MAX_ROWS} строк")));
    }
    let ix = index(&app).await?;
    Ok(Json(
        req.rows
            .iter()
            .map(|r| RowCheck {
                errors: ix.to_client(r, &req.group_id).err().unwrap_or_default(),
                duplicate: ix.duplicate(r),
            })
            .collect(),
    ))
}

// ---------- Импорт ----------

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Update,
    Skip,
}

#[derive(Deserialize)]
pub struct CommitRow {
    #[serde(flatten)]
    record: Record,
    action: Action,
    /// Для «обновить» — какой клиент.
    target_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CommitIn {
    file_name: String,
    #[serde(default = "default_group")]
    group_id: String,
    rows: Vec<CommitRow>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Stats {
    created: usize,
    updated: usize,
    skipped: usize,
}

#[derive(Serialize)]
pub struct CommitOut {
    batch_id: String,
    #[serde(flatten)]
    stats: Stats,
}

/// Шаг 4: одной транзакцией — всё или ничего.
pub async fn commit(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CommitIn>,
) -> Result<Json<CommitOut>, ApiError> {
    require_manager(&user)?;
    if req.rows.len() > MAX_ROWS {
        return Err(ApiError::BadRequest(format!("не больше {MAX_ROWS} строк")));
    }
    let scope = Scope::load(&app.pool, &user).await?;
    if !can_create_client(&scope.actor, &req.group_id) {
        return Err(ApiError::Forbidden);
    }
    let ix = index(&app).await?;
    let batch = new_id();
    let mut stats = Stats::default();
    let mut tx = app.pool.begin().await?;
    sqlx::query(
        "INSERT INTO import_batches(id, file_name, created_by, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&batch)
    .bind(crate::attachments::clean_name(&req.file_name))
    .bind(&user.id)
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    for (i, row) in req.rows.iter().enumerate() {
        if row.action == Action::Skip {
            stats.skipped += 1;
            continue;
        }
        let input = ix
            .to_client(&row.record, &req.group_id)
            .map_err(|e| ApiError::BadRequest(format!("строка {}: {}", i + 2, e.join("; "))))?;
        let valid = input.validate()?;
        match (row.action, &row.target_id) {
            (Action::Update, Some(id)) => {
                if scope.client(id) != crm_core::access::ClientAccess::Manage {
                    return Err(ApiError::BadRequest(format!(
                        "строка {}: нет прав изменить существующего клиента",
                        i + 2
                    )));
                }
                crate::clients::update_row(&mut tx, id, &valid).await?;
                stats.updated += 1;
            }
            (Action::Update, None) => {
                return Err(ApiError::BadRequest(format!(
                    "строка {}: не выбран клиент для обновления",
                    i + 2
                )))
            }
            _ => {
                crate::clients::insert(&mut tx, &valid, &user.id, Some(&batch)).await?;
                stats.created += 1;
            }
        }
    }
    let json = serde_json::to_string(&stats).map_err(|e| ApiError::Failed(e.to_string()))?;
    sqlx::query("UPDATE import_batches SET stats = ? WHERE id = ?")
        .bind(json)
        .bind(&batch)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(CommitOut {
        batch_id: batch,
        stats,
    }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Batch {
    id: String,
    file_name: String,
    created_at: i64,
    stats: String,
    undone_at: Option<i64>,
    can_undo: bool,
}

pub async fn batches(
    State(app): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<Batch>>, ApiError> {
    require_manager(&user)?;
    let host = user.role == Role::Host;
    Ok(Json(
        sqlx::query_as(
            "SELECT id, file_name, created_at, stats, undone_at,
                    (undone_at IS NULL AND created_at > ?) AS can_undo
             FROM import_batches WHERE (? OR created_by = ?) ORDER BY created_at DESC LIMIT 20",
        )
        .bind(now_ms() - UNDO_WINDOW_MS)
        .bind(host)
        .bind(&user.id)
        .fetch_all(&app.pool)
        .await?,
    ))
}

#[derive(Serialize)]
pub struct UndoOut {
    removed: u64,
    /// Остались: у них уже появились задачи.
    kept: i64,
}

/// Отмена импорта: удаляются клиенты партии, у которых ещё нет задач.
pub async fn undo(
    State(app): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<UndoOut>, ApiError> {
    require_manager(&user)?;
    let (created_by, created_at, undone): (String, i64, Option<i64>) =
        sqlx::query_as("SELECT created_by, created_at, undone_at FROM import_batches WHERE id = ?")
            .bind(&id)
            .fetch_optional(&app.pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    if user.role != Role::Host && created_by != user.id {
        return Err(ApiError::Forbidden);
    }
    if undone.is_some() || now_ms() - created_at > UNDO_WINDOW_MS {
        return Err(ApiError::Conflict(
            "отменить импорт можно в течение суток и один раз",
        ));
    }
    let mut tx = app.pool.begin().await?;
    let removed = sqlx::query(
        "DELETE FROM clients WHERE import_batch_id = ?
         AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.client_id = clients.id)",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM clients WHERE import_batch_id = ?")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("UPDATE import_batches SET undone_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(UndoOut { removed, kept }))
}

// ---------- Выгрузка ----------

fn download(bytes: Vec<u8>, mime: &'static str, name: &str) -> Response {
    let mut res = bytes.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{encoded}")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    res
}

const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

fn xlsx_err(e: rust_xlsxwriter::XlsxError) -> ApiError {
    ApiError::Failed(format!("не удалось собрать Excel: {e}"))
}

/// Лист Excel из заголовков и строк.
pub fn add_sheet(
    wb: &mut Workbook,
    title: &str,
    headers: &[&str],
    rows: &[Vec<String>],
) -> Result<(), ApiError> {
    let bold = Format::new().set_bold();
    let ws = wb.add_worksheet();
    ws.set_name(title).map_err(xlsx_err)?;
    for (c, h) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *h, &bold)
            .map_err(xlsx_err)?;
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            ws.write_string(r as u32 + 1, c as u16, v)
                .map_err(xlsx_err)?;
        }
    }
    ws.set_freeze_panes(1, 0).map_err(xlsx_err)?;
    ws.autofit();
    Ok(())
}

#[derive(Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
}

const CLIENT_HEADERS: [&str; 8] = [
    "Название",
    "Тип",
    "Телефон",
    "Email",
    "Описание",
    "Теги",
    "Группа",
    "Менеджер",
];

/// Клиенты в пределах прав: `.xlsx` или `.csv` (`?format=csv`).
pub async fn export_clients(
    State(app): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let rows: Vec<ClientExportRow> =
        sqlx::query_as(
            "SELECT c.id, c.name, c.kind, c.phone, c.email, c.description, g.name, m.display_name
             FROM clients c JOIN groups g ON g.id = c.group_id LEFT JOIN users m ON m.id = c.manager_id
             WHERE c.deleted_at IS NULL ORDER BY c.name COLLATE NOCASE",
        )
        .fetch_all(&app.pool)
        .await?;
    let tags: Vec<(String, String)> = sqlx::query_as(
        "SELECT ct.client_id, t.name FROM client_tags ct JOIN tags t ON t.id = ct.tag_id",
    )
    .fetch_all(&app.pool)
    .await?;
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .filter(|r| scope.client(&r.0) != crm_core::access::ClientAccess::None)
        .map(|(id, name, kind, phone, email, desc, group, manager)| {
            let t: Vec<&str> = tags
                .iter()
                .filter(|(c, _)| *c == id)
                .map(|(_, n)| n.as_str())
                .collect();
            vec![
                name,
                if kind == "company" {
                    "Компания"
                } else {
                    "Физлицо"
                }
                .into(),
                phone.unwrap_or_default(),
                email.unwrap_or_default(),
                desc,
                t.join(", "),
                group,
                manager.unwrap_or_default(),
            ]
        })
        .collect();
    if q.format.as_deref() == Some("csv") {
        return Ok(download(
            to_csv(&CLIENT_HEADERS, &table)?,
            "text/csv; charset=utf-8",
            "clients.csv",
        ));
    }
    let mut wb = Workbook::new();
    add_sheet(&mut wb, "Клиенты", &CLIENT_HEADERS, &table)?;
    Ok(download(
        wb.save_to_buffer().map_err(xlsx_err)?,
        XLSX,
        "clients.xlsx",
    ))
}

/// CSV для Excel в России: UTF-8 с BOM и разделитель «;».
fn to_csv(headers: &[&str], rows: &[Vec<String>]) -> Result<Vec<u8>, ApiError> {
    let mut out = "\u{feff}".as_bytes().to_vec();
    {
        let mut w = csv::WriterBuilder::new()
            .delimiter(b';')
            .from_writer(&mut out);
        let fail = |e: csv::Error| ApiError::Failed(e.to_string());
        w.write_record(headers).map_err(fail)?;
        for r in rows {
            w.write_record(r).map_err(fail)?;
        }
        w.flush().map_err(|e| ApiError::Failed(e.to_string()))?;
    }
    Ok(out)
}

/// Шаблон для импорта.
pub async fn template(_: CurrentUser) -> Result<Response, ApiError> {
    let headers: Vec<&str> = Field::ALL.iter().map(|f| f.title()).collect();
    let mut wb = Workbook::new();
    add_sheet(
        &mut wb,
        "Клиенты",
        &headers,
        &[vec![
            "ООО «Ромашка»".into(),
            "Компания".into(),
            "+7 900 123-45-67".into(),
            "info@romashka.ru".into(),
            "Поставка оборудования, договор №12".into(),
            "VIP, опт".into(),
            String::new(),
        ]],
    )?;
    Ok(download(
        wb.save_to_buffer().map_err(xlsx_err)?,
        XLSX,
        "Шаблон клиентов.xlsx",
    ))
}

/// Задачи (по клиенту или все доступные) в Excel.
pub async fn export_tasks(
    State(app): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let scope = Scope::load(&app.pool, &user).await?;
    let tz = crate::org::load(&app.pool).await?.tz();
    let all = crate::tasks::summaries(&app, &scope, &user.id).await?;
    let titles: HashMap<&str, (&str, Option<&str>)> = all
        .iter()
        .map(|t| (t.id.as_str(), (t.title.as_str(), t.parent_id.as_deref())))
        .collect();
    let path = |id: &str| task_path(&titles, id);
    let table: Vec<Vec<String>> = all
        .iter()
        .filter(|t| q.client_id.as_ref().is_none_or(|c| *c == t.client_id))
        .filter_map(|t| {
            let d = t.details.as_ref()?;
            Some(vec![
                t.client_name.clone(),
                path(&t.id),
                d.status.label().into(),
                format!("P{}", i64::from(d.priority)),
                d.assignees
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                crate::tasks::fmt_time(tz, d.deadline_at),
                d.completed_at
                    .map(|c| crate::tasks::fmt_time(tz, c))
                    .unwrap_or_default(),
                format!("{}/{}", d.progress.done, d.progress.total),
            ])
        })
        .collect();
    let mut wb = Workbook::new();
    add_sheet(
        &mut wb,
        "Задачи",
        &[
            "Клиент",
            "Задача",
            "Статус",
            "Приоритет",
            "Исполнители",
            "Дедлайн",
            "Завершена",
            "Этапы",
        ],
        &table,
    )?;
    Ok(download(
        wb.save_to_buffer().map_err(xlsx_err)?,
        XLSX,
        "tasks.xlsx",
    ))
}

/// «Проект → Монтаж → Кабели» — путь задачи по названиям.
fn task_path<'a>(titles: &HashMap<&'a str, (&'a str, Option<&'a str>)>, id: &'a str) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(id);
    while let Some(i) = cur {
        let Some((t, p)) = titles.get(i) else { break };
        parts.push(*t);
        cur = *p;
        if parts.len() > 50 {
            break;
        }
    }
    parts.reverse();
    parts.join(" → ")
}

// ---------- Полная выгрузка (владелец) ----------

/// Все таблицы в Excel + вложения — ZIP для архива или перехода на другую систему.
pub async fn export_full(
    State(app): State<AppState>,
    crate::auth::Owner(_): crate::auth::Owner,
) -> Result<Response, ApiError> {
    let tz = crate::org::load(&app.pool).await?.tz();
    let t = |v: i64| crate::tasks::fmt_time(tz, v);
    let ot = |v: Option<i64>| v.map(t).unwrap_or_default();
    let mut wb = Workbook::new();

    let rows: Vec<ClientDumpRow> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, c.phone, c.email, c.description, g.name, c.created_at
             FROM clients c JOIN groups g ON g.id = c.group_id WHERE c.deleted_at IS NULL",
    )
    .fetch_all(&app.pool)
    .await?;
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(id, n, k, p, e, d, g, c)| {
            vec![
                id,
                n,
                k,
                p.unwrap_or_default(),
                e.unwrap_or_default(),
                d,
                g,
                t(c),
            ]
        })
        .collect();
    add_sheet(
        &mut wb,
        "Клиенты",
        &[
            "ID",
            "Название",
            "Тип",
            "Телефон",
            "Email",
            "Описание",
            "Группа",
            "Создан",
        ],
        &table,
    )?;

    let rows: Vec<TaskDumpRow> =
        sqlx::query_as(
            "SELECT id, client_id, parent_task_id, title, description, priority, status, created_at, deadline_at, completed_at
             FROM tasks WHERE deleted_at IS NULL",
        )
        .fetch_all(&app.pool)
        .await?;
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(id, c, p, title, d, pr, st, cr, dl, done)| {
            vec![
                id,
                c,
                p.unwrap_or_default(),
                title,
                d.unwrap_or_default(),
                pr.to_string(),
                st,
                t(cr),
                t(dl),
                ot(done),
            ]
        })
        .collect();
    add_sheet(
        &mut wb,
        "Задачи",
        &[
            "ID",
            "Клиент",
            "Родитель",
            "Название",
            "Описание",
            "Приоритет",
            "Статус",
            "Создана",
            "Дедлайн",
            "Завершена",
        ],
        &table,
    )?;

    let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT task_id, title, completed_at FROM task_stages ORDER BY task_id, sort_order",
    )
    .fetch_all(&app.pool)
    .await?;
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(task, title, done)| vec![task, title, ot(done)])
        .collect();
    add_sheet(&mut wb, "Этапы", &["Задача", "Этап", "Отмечен"], &table)?;

    let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        "SELECT m.task_id, u.display_name, m.kind, m.content, m.created_at
         FROM messages m JOIN users u ON u.id = m.sender_id ORDER BY m.task_id, m.id",
    )
    .fetch_all(&app.pool)
    .await?;
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(task, who, kind, text, at)| vec![task, who, kind, text, t(at)])
        .collect();
    add_sheet(
        &mut wb,
        "Сообщения",
        &["Задача", "Автор", "Вид", "Текст", "Время"],
        &table,
    )?;

    let rows: Vec<(String, String, String, i64)> =
        sqlx::query_as("SELECT username, display_name, role, created_at FROM users")
            .fetch_all(&app.pool)
            .await?;
    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(u, n, r, c)| vec![u, n, r, t(c)])
        .collect();
    add_sheet(
        &mut wb,
        "Сотрудники",
        &["Логин", "Имя", "Роль", "Создан"],
        &table,
    )?;
    let xlsx = wb.save_to_buffer().map_err(xlsx_err)?;

    let files: Vec<(String, String)> =
        sqlx::query_as("SELECT DISTINCT sha256, file_name FROM message_attachments")
            .fetch_all(&app.pool)
            .await?;
    let dir = app.rt.config().storage.attachments_dir;
    let tmp_dir = dir.join("tmp");
    let out = tmp_dir.join(format!("export-{}.zip", new_id()));
    let out2 = out.clone();
    tokio::task::spawn_blocking(move || write_zip(&out2, &xlsx, &dir, &files))
        .await
        .map_err(|e| ApiError::Failed(e.to_string()))??;
    let file = tokio::fs::File::open(&out)
        .await
        .map_err(|e| ApiError::Failed(format!("выгрузка: {e}")))?;
    // Файл уже открыт — его можно убрать из папки (Unix); на Windows уберёт ночная уборка.
    let _ = std::fs::remove_file(&out);
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file));
    let mut res = Response::new(body);
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    let name = format!("crm_export_{}.zip", chrono::Utc::now().format("%Y_%m_%d"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        res.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

fn write_zip(
    out: &std::path::Path,
    xlsx: &[u8],
    dir: &std::path::Path,
    files: &[(String, String)],
) -> Result<(), ApiError> {
    use std::io::Write;
    let fail = |e: &dyn std::fmt::Display| ApiError::Failed(format!("выгрузка: {e}"));
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| fail(&e))?;
    }
    let f = std::fs::File::create(out).map_err(|e| fail(&e))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default();
    z.start_file("data.xlsx", opts).map_err(|e| fail(&e))?;
    z.write_all(xlsx).map_err(|e| fail(&e))?;
    for (sha, name) in files {
        let Some(prefix) = sha.get(..2) else { continue };
        let src = dir.join(prefix).join(sha);
        let Ok(mut f) = std::fs::File::open(&src) else {
            continue;
        };
        let short = sha.get(..8).unwrap_or(sha);
        z.start_file(format!("attachments/{short}_{name}"), opts)
            .map_err(|e| fail(&e))?;
        std::io::copy(&mut f, &mut z).map_err(|e| fail(&e))?;
    }
    z.finish().map_err(|e| fail(&e))?;
    Ok(())
}
