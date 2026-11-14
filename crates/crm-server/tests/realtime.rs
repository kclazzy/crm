//! WebSocket: событие о сообщении приходит тем, кто видит задачу, и только им.
#![allow(clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::{App, LOCAL};
use futures_util::StreamExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

async fn socket(
    addr: SocketAddr,
    token: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut req = format!("ws://{addr}/api/ws").into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let hello = ws.next().await.unwrap().unwrap();
    assert_eq!(hello, Message::Text(r#"{"type":"hello"}"#.into()));
    ws
}

async fn next_text(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Option<serde_json::Value> {
    match tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => Some(serde_json::from_str(&t).unwrap()),
        _ => None,
    }
}

#[tokio::test]
async fn chat_events_reach_only_the_audience() {
    let app = App::new().await;
    let owner = app.owner().await;
    let (a_id, a) = app.member(&owner, "anna", "employee", "10.0.0.2").await;
    let (_, outsider) = app.member(&owner, "oleg", "employee", "10.0.0.3").await;
    let (_, c) = app
        .call(
            "POST",
            "/api/clients",
            LOCAL,
            Some(&owner),
            Some(json!({"kind": "person", "name": "К"})),
        )
        .await;
    let (_, t) = app
        .call("POST", "/api/tasks", LOCAL, Some(&owner), Some(json!({
            "client_id": c["id"], "title": "Т", "priority": 2,
            "deadline_at": chrono::Utc::now().timestamp_millis() + 86_400_000, "assignees": [a_id]
        })))
        .await;
    let t = t["id"].as_str().unwrap().to_owned();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = crm_server::http::router(app.state.clone())
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, router).await });

    let mut anna = socket(addr, &a).await;
    let mut oleg = socket(addr, &outsider).await;
    app.call(
        "POST",
        &format!("/api/tasks/{t}/messages"),
        LOCAL,
        Some(&owner),
        Some(json!({"content": "Новости"})),
    )
    .await;

    let ev = next_text(&mut anna).await.unwrap();
    assert_eq!(ev["type"], "task");
    assert_eq!(ev["task_id"], t.as_str());
    assert_eq!(ev["what"], "message");
    assert!(
        next_text(&mut oleg).await.is_none(),
        "посторонний ничего не получил"
    );

    // Без входа — отказ.
    let req = format!("ws://{addr}/api/ws").into_client_request().unwrap();
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
}
