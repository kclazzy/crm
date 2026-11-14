//! Поиск Ctrl+K (ТЗ §4.8): FTS5 trigram, результаты — только из доступного.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::Json;
use crm_core::access::ClientAccess;
use serde::{Deserialize, Serialize};

use crate::access::Scope;
use crate::auth::CurrentUser;
use crate::http::{ApiError, AppState};

const PER_KIND: usize = 8;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    #[serde(default)]
    archive: bool,
}

#[derive(Serialize)]
pub struct Hit {
    kind: &'static str,
    id: String,
    /// Куда вести: задача для этапа и сообщения.
    task_id: Option<String>,
    client_id: Option<String>,
    title: String,
    snippet: String,
}

/// Строка пользователя → безопасный запрос FTS: каждое слово — фраза в кавычках.
pub fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split_whitespace()
        .filter(|w| w.chars().count() >= 3)
        .take(8)
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" AND "))
    }
}

fn snippet(text: &str, q: &str) -> String {
    let lower = text.to_lowercase();
    let needle = q.split_whitespace().next().unwrap_or("").to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let pos = lower
        .find(&needle)
        .map(|b| lower[..b].chars().count())
        .unwrap_or(0);
    let start = pos.saturating_sub(40);
    let end = (pos + 80).min(chars.len());
    let mut s: String = chars[start..end].iter().collect();
    if start > 0 {
        s.insert(0, '…');
    }
    if end < chars.len() {
        s.push('…');
    }
    s
}

pub async fn search(
    State(app): State<AppState>,
    user: CurrentUser,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<Hit>>, ApiError> {
    let Some(fts) = fts_query(&q.q) else {
        return Ok(Json(Vec::new()));
    };
    let scope = Scope::load(&app.pool, &user).await?;
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT entity, entity_id, title, body FROM search_index WHERE search_index MATCH ? ORDER BY rank LIMIT 500",
    )
    .bind(&fts)
    .fetch_all(&app.pool)
    .await?;
    let stage_task: HashMap<String, String> = sqlx::query_as("SELECT id, task_id FROM task_stages")
        .fetch_all(&app.pool)
        .await?
        .into_iter()
        .collect();
    let archived: std::collections::HashSet<String> =
        sqlx::query_scalar("SELECT id FROM tasks WHERE archived_at IS NOT NULL")
            .fetch_all(&app.pool)
            .await?
            .into_iter()
            .collect();
    let mut msg_task: HashMap<String, String> = HashMap::new();
    for (entity, id, _, _) in &rows {
        if entity == "message" {
            if let Some(t) =
                sqlx::query_scalar::<_, String>("SELECT task_id FROM messages WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&app.pool)
                    .await?
            {
                msg_task.insert(id.clone(), t);
            }
        }
    }
    let task_ok = |t: &str| scope.task(t).can_read() && (q.archive || !archived.contains(t));
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let mut out = Vec::new();
    for (entity, id, title, body) in rows {
        let (kind, task_id, client_id) = match entity.as_str() {
            "client" if scope.client(&id) != ClientAccess::None => {
                ("client", None, Some(id.clone()))
            }
            "task" if task_ok(&id) => (
                "task",
                Some(id.clone()),
                scope.client_of(&id).map(str::to_owned),
            ),
            "stage" => match stage_task.get(&id) {
                Some(t) if task_ok(t) => ("stage", Some(t.clone()), None),
                _ => continue,
            },
            "message" => match msg_task.get(&id) {
                Some(t) if task_ok(t) => ("message", Some(t.clone()), None),
                _ => continue,
            },
            _ => continue,
        };
        let n = counts.entry(kind).or_default();
        if *n >= PER_KIND {
            continue;
        }
        *n += 1;
        let text = if body.is_empty() { &title } else { &body };
        out.push(Hit {
            kind,
            id,
            task_id,
            client_id,
            snippet: snippet(text, &q.q),
            title,
        });
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_quoted_and_short_words_dropped() {
        assert_eq!(fts_query("до договор"), Some("\"договор\"".into()));
        assert_eq!(fts_query("ab"), None);
        assert_eq!(fts_query("x\"y OR z"), Some("\"x\"\"y\"".into()));
    }

    #[test]
    fn snippet_centres_on_match() {
        let text = format!("{}договор{}", "а".repeat(100), "б".repeat(100));
        let s = snippet(&text, "ДОГОВОР");
        assert!(s.starts_with('…') && s.ends_with('…') && s.contains("договор"));
    }
}
