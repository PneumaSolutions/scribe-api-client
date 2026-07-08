//! Exercises `DocumentChannel` against a tiny local Phoenix-protocol WebSocket server, since wiremock only speaks plain HTTP.

use futures_util::{SinkExt, StreamExt};
use scribe_client::{ChannelEvent, OutputFormat, ScribeClient, ScribeError, Stage, TokenSet};
use serde_json::Value;
use std::{future::Future, pin::Pin};
use time::OffsetDateTime;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use url::Url;

type RawFrame = (Option<String>, Option<String>, String, String, Value);

fn valid_tokens() -> TokenSet {
    TokenSet {
        access_token: "at-valid".into(),
        refresh_token: Some("rt-valid".into()),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
    }
}

/// Starts a fake `/socket/websocket` server on an ephemeral local port and
/// returns its base `http://` URL (the same shape `ScribeClient` expects,
/// since it derives the `ws://` URL itself).
async fn start_fake_server(
    handle_conn: impl FnOnce(WebSocketStream<TcpStream>) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        handle_conn(ws).await;
    });
    Url::parse(&format!("http://{addr}")).unwrap()
}

fn client_for(base_url: Url) -> ScribeClient {
    ScribeClient::new(
        reqwest::Client::new(),
        base_url,
        "test-client-id",
        valid_tokens(),
    )
}

async fn recv(ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> RawFrame {
    match ws.next().await.unwrap().unwrap() {
        Message::Text(text) => serde_json::from_str(text.as_str()).unwrap(),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

async fn send(
    ws: &mut WebSocketStream<TcpStream>,
    join_ref: &str,
    ref_: Option<&str>,
    topic: &str,
    event: &str,
    payload: Value,
) {
    let frame: RawFrame = (
        Some(join_ref.to_string()),
        ref_.map(str::to_string),
        topic.to_string(),
        event.to_string(),
        payload,
    );
    ws.send(Message::text(serde_json::to_string(&frame).unwrap()))
        .await
        .unwrap();
}

#[tokio::test]
async fn join_then_start_conversion_then_events() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, event, _payload) = recv(&mut ws).await;
            assert_eq!(event, "phx_join");
            let join_ref = join_ref.unwrap();
            let ref_ = ref_.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {}}),
            )
            .await;
            let (_, ref_, topic, event, payload) = recv(&mut ws).await;
            assert_eq!(event, "start_conversion");
            assert_eq!(payload["format"], "pdf");
            let ref_ = ref_.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {"output_id": "out-1"}}),
            )
            .await;
            send(
                &mut ws,
                &join_ref,
                None,
                &topic,
                "status",
                serde_json::json!({"format": "pdf", "stage": "convert", "progress": 0.5}),
            )
            .await;
            send(
                &mut ws,
                &join_ref,
                None,
                &topic,
                "conversion_complete",
                serde_json::json!({"format": "pdf", "output_id": "out-1"}),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    let output_id = channel.start_conversion(OutputFormat::Pdf).await.unwrap();
    assert_eq!(output_id, "out-1");
    match channel.next_event().await.unwrap() {
        ChannelEvent::Status {
            format,
            stage,
            progress,
        } => {
            assert_eq!(format, OutputFormat::Pdf);
            assert_eq!(stage, Stage::Convert);
            assert!((progress - 0.5).abs() < f64::EPSILON);
        }
        other => panic!("expected Status, got {other:?}"),
    }
    match channel.next_event().await.unwrap() {
        ChannelEvent::ConversionComplete { format, output_id } => {
            assert_eq!(format, OutputFormat::Pdf);
            assert_eq!(output_id, "out-1");
        }
        other => panic!("expected ConversionComplete, got {other:?}"),
    }
}

#[tokio::test]
async fn join_error_maps_not_found() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            let ref_ = ref_.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "error", "response": {"reason": "not_found"}}),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let result = client.open_document_channel("doc-1").await;
    assert!(matches!(result, Err(ScribeError::NotFound)));
}

#[tokio::test]
async fn start_conversion_error_maps_needs_purchase() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            let ref_ = ref_.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {}}),
            )
            .await;
            let (_, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let ref_ = ref_.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_),
                &topic,
                "phx_reply",
                serde_json::json!({
                    "status": "error",
                    "response": {"reason": "needs_purchase", "purchase_url": "https://example.test/buy"}
                }),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    let result = channel.start_conversion(OutputFormat::Pdf).await;
    match result {
        Err(ScribeError::NeedsPurchase { purchase_url }) => {
            assert_eq!(purchase_url, "https://example.test/buy");
        }
        other => panic!("expected NeedsPurchase, got {other:?}"),
    }
}
