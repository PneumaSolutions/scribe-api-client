//! Exercises the `UniFFI` document-channel bindings against a tiny local
//! Phoenix-protocol WebSocket server, mirroring
//! `scribe-client/tests/channel_test.rs`'s fake-server helper. The FFI
//! methods are synchronous (they block on an internal Tokio runtime), so
//! each is driven from `spawn_blocking` while this test's own runtime
//! drives the fake server concurrently.

use std::{future::Future, pin::Pin};

use futures_util::{SinkExt, StreamExt};
use scribe_client_ffi::{
    ChannelEvent, FfiScribeClient, OutputFormat, ScribeError, Stage, TokenSet,
};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

type RawFrame = (Option<String>, Option<String>, String, String, Value);

fn valid_tokens() -> TokenSet {
    TokenSet {
        access_token: "at-valid".into(),
        refresh_token: Some("rt-valid".into()),
        expires_at_unix_secs: None,
    }
}

async fn start_fake_server(
    handle_conn: impl FnOnce(WebSocketStream<TcpStream>) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        handle_conn(ws).await;
    });
    format!("http://{addr}")
}

async fn recv(ws: &mut WebSocketStream<TcpStream>) -> RawFrame {
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
async fn ffi_join_start_conversion_and_events() {
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
    let client = FfiScribeClient::new(base_url, "test-client-id".into(), valid_tokens()).unwrap();
    let (output_id, first_event, second_event) = tokio::task::spawn_blocking(move || {
        let channel = client.open_document_channel("doc-1".into()).unwrap();
        let output_id = channel.start_conversion(OutputFormat::Pdf).unwrap();
        let first_event = channel.next_event().unwrap();
        let second_event = channel.next_event().unwrap();
        (output_id, first_event, second_event)
    })
    .await
    .unwrap();
    assert_eq!(output_id, "out-1");
    match first_event {
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
    match second_event {
        ChannelEvent::ConversionComplete { format, output_id } => {
            assert_eq!(format, OutputFormat::Pdf);
            assert_eq!(output_id, "out-1");
        }
        other => panic!("expected ConversionComplete, got {other:?}"),
    }
}

#[tokio::test]
async fn ffi_open_channel_maps_not_found() {
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
    let client = FfiScribeClient::new(base_url, "test-client-id".into(), valid_tokens()).unwrap();
    let result = tokio::task::spawn_blocking(move || client.open_document_channel("doc-1".into()))
        .await
        .unwrap();
    assert!(matches!(result, Err(ScribeError::NotFound)));
}

#[tokio::test]
async fn ffi_start_conversion_maps_needs_purchase() {
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
    let client = FfiScribeClient::new(base_url, "test-client-id".into(), valid_tokens()).unwrap();
    let result = tokio::task::spawn_blocking(move || {
        let channel = client.open_document_channel("doc-1".into()).unwrap();
        channel.start_conversion(OutputFormat::Pdf)
    })
    .await
    .unwrap();
    match result {
        Err(ScribeError::NeedsPurchase { purchase_url }) => {
            assert_eq!(purchase_url, "https://example.test/buy");
        }
        other => panic!("expected NeedsPurchase, got {other:?}"),
    }
}

#[tokio::test]
async fn ffi_close_is_idempotent() {
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
        })
    })
    .await;
    let client = FfiScribeClient::new(base_url, "test-client-id".into(), valid_tokens()).unwrap();
    tokio::task::spawn_blocking(move || {
        let channel = client.open_document_channel("doc-1".into()).unwrap();
        channel.close().unwrap();
        channel.close().unwrap();
    })
    .await
    .unwrap();
}
