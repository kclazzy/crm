//! Импорт клиентов из файла и выгрузка в Excel/CSV.
#![allow(clippy::unwrap_used)]

mod common;

use std::io::Cursor;

use axum::http::StatusCode;
use calamine::Reader;
use common::{App, LOCAL};
use serde_json::{json, Value};

/// CSV как его сохраняет Excel в России: Windows-1251, разделитель «;».
fn cp1251_csv() -> Vec<u8> {
    let text = "ФИО;Телефон;E-mail;Комментарий;Теги\r\n\
                ООО «Ромашка»;8 (900) 111-22-33;info@romashka.ru;Новый адрес;VIP\r\n\
                Петров Пётр;+7 900 555 66 77;;Постоянный клиент;опт, Москва\r\n\
                ;;;пустое название;\r\n";
    encoding_rs::WINDOWS_1251.encode(text).0.into_owned()
}

fn records(parsed: &Value) -> Vec<Value> {
    let m = &parsed["mapping"];
    let col = |row: &Value, f: &str| {
        m[f].as_u64()
            .map(|i| row[i as usize].clone())
            .unwrap_or(Value::Null)
    };
    parsed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            json!({
                "name": col(r, "name"), "phone": col(r, "phone"), "email": col(r, "email"),
                "description": col(r, "description"), "tags": col(r, "tags")
            })
        })
        .collect()
}

#[tokio::test]
async fn import_detects_duplicates_imports_atomically_and_undoes() {
    let app = App::new().await;
    let owner = app.owner().await;
    // Уже есть «Ромашка» — с другим написанием.
    let (s, _) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "company", "name": "ромашка"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, parsed) = app
        .post_bytes(
            &owner,
            "/api/import/clients/parse",
            "clients.csv",
            &cp1251_csv(),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{parsed}");
    assert_eq!(parsed["headers"][0], "ФИО");
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 3);
    let recs = records(&parsed);
    assert_eq!(recs[0]["name"], "ООО «Ромашка»");

    let (_, checks) = app
        .call(
            "POST",
            "/api/import/clients/check",
            LOCAL,
            Some(&owner),
            Some(json!({"rows": recs})),
        )
        .await;
    assert_eq!(checks[0]["duplicate"]["by"], "название", "{checks}");
    assert!(checks[1]["duplicate"].is_null());
    assert!(
        !checks[2]["errors"].as_array().unwrap().is_empty(),
        "пустое название"
    );

    // Ошибочная строка не пропущена — импорт целиком отклонён.
    let rows: Vec<Value> = recs
        .iter()
        .map(|r| {
            let mut r = r.clone();
            r["action"] = json!("create");
            r
        })
        .collect();
    let (s, _) = app
        .call(
            "POST",
            "/api/import/clients/commit",
            LOCAL,
            Some(&owner),
            Some(json!({"file_name": "clients.csv", "rows": rows})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, list) = app
        .call("GET", "/api/clients", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1, "ничего не записано");

    let dup = checks[0]["duplicate"]["id"].as_str().unwrap();
    let mut rows = rows;
    rows[0]["action"] = json!("update");
    rows[0]["target_id"] = json!(dup);
    rows[2]["action"] = json!("skip");
    let (s, out) = app
        .call(
            "POST",
            "/api/import/clients/commit",
            LOCAL,
            Some(&owner),
            Some(json!({"file_name": "clients.csv", "rows": rows})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(
        (
            out["created"].as_i64(),
            out["updated"].as_i64(),
            out["skipped"].as_i64()
        ),
        (Some(1), Some(1), Some(1))
    );
    let (_, list) = app
        .call("GET", "/api/clients?tag=опт", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(list[0]["name"], "Петров Пётр");
    assert_eq!(list[0]["kind"], "person");

    let batch = out["batch_id"].as_str().unwrap();
    let (s, undo) = app
        .call(
            "POST",
            &format!("/api/import/batches/{batch}/undo"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(undo["removed"], 1);
    let (_, list) = app
        .call("GET", "/api/clients", LOCAL, Some(&owner), None)
        .await;
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "обновлённый существующий клиент остался"
    );
    let (s, _) = app
        .call(
            "POST",
            &format!("/api/import/batches/{batch}/undo"),
            LOCAL,
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "второй раз нельзя");
}

#[tokio::test]
async fn employees_cannot_import() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, emp) = app.member(&owner, "ivan", "employee", "10.0.0.2").await;
    let (s, _) = app
        .post_bytes(
            &emp,
            "/api/import/clients/parse",
            "c.csv",
            "ФИО\nИван".as_bytes(),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn exports_open_in_excel_and_respect_access() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (_, emp) = app.member(&owner, "ivan", "employee", "10.0.0.2").await;
    app.call(
        "POST",
        "/api/clients",
        LOCAL,
        Some(&owner),
        Some(json!({
            "kind": "company", "name": "ООО Вектор", "description": "Описание", "tags": ["VIP"]
        })),
    )
    .await;

    let (s, bytes) = app.get_bytes(&owner, "/api/export/clients").await;
    assert_eq!(s, StatusCode::OK);
    let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).unwrap();
    let sheet = wb.worksheet_range("Клиенты").unwrap();
    assert_eq!(sheet.get((1, 0)).unwrap().to_string(), "ООО Вектор");
    assert_eq!(sheet.get((1, 5)).unwrap().to_string(), "VIP");

    let (_, csv) = app
        .get_bytes(&owner, "/api/export/clients?format=csv")
        .await;
    let text = String::from_utf8(csv).unwrap();
    assert!(text.starts_with('\u{feff}') && text.contains("ООО Вектор;Компания"));

    let (_, csv) = app.get_bytes(&emp, "/api/export/clients?format=csv").await;
    assert!(
        !String::from_utf8(csv).unwrap().contains("Вектор"),
        "сотруднику — только своё"
    );

    // Задачи — тоже в CSV (для больших объёмов и других программ).
    let (s, csv) = app.get_bytes(&owner, "/api/export/tasks?format=csv").await;
    assert_eq!(s, StatusCode::OK);
    let text = String::from_utf8(csv).unwrap();
    assert!(
        text.starts_with('\u{feff}') && text.lines().next().unwrap().contains("Клиент;"),
        "{text}"
    );

    let (s, tpl) = app.get_bytes(&owner, "/api/import/clients/template").await;
    assert_eq!(s, StatusCode::OK);
    let (s, parsed) = app
        .post_bytes(&owner, "/api/import/clients/parse", "Шаблон.xlsx", &tpl)
        .await;
    assert_eq!(s, StatusCode::OK, "шаблон читается обратно: {parsed}");
    assert_eq!(parsed["mapping"]["manager"], 6, "{parsed}");

    let (s, zip) = app.get_bytes(&owner, "/api/admin/export/full").await;
    assert_eq!(s, StatusCode::OK);
    assert!(zip.starts_with(b"PK"));
    let (s, _) = app.get_bytes(&emp, "/api/admin/export/full").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
