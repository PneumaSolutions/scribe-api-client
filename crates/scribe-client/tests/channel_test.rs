//! Exercises `DocumentChannel` against a tiny local Phoenix-protocol WebSocket server, since wiremock only speaks plain HTTP.

use futures_util::{SinkExt, StreamExt};
use scribe_client::{ChannelEvent, OutputFormat, ScribeClient, ScribeError, Stage, TokenSet};
use serde_json::Value;
use std::{future::Future, pin::Pin, time::Duration};
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
    let output_id = channel
        .start_conversion(OutputFormat::Pdf, None)
        .await
        .unwrap();
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
    assert!(matches!(result, Err(ScribeError::NotFound { .. })));
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
    let result = channel.start_conversion(OutputFormat::Pdf, None).await;
    match result {
        Err(ScribeError::NeedsPurchase { purchase_url, .. }) => {
            assert_eq!(purchase_url, "https://example.test/buy");
        }
        other => panic!("expected NeedsPurchase, got {other:?}"),
    }
}

/// The password only rides along when one was actually supplied — sending
/// `"password": null` on every conversion would be indistinguishable, at the
/// server, from a deliberate attempt to clear it.
#[tokio::test]
async fn start_conversion_omits_the_password_key_when_none_is_given() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {}}),
            )
            .await;
            let (_, ref_, topic, event, payload) = recv(&mut ws).await;
            assert_eq!(event, "start_conversion");
            assert_eq!(payload["format"], "pdf");
            assert!(payload.get("password").is_none());
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {"output_id": "out-1"}}),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    let output_id = channel
        .start_conversion(OutputFormat::Pdf, None)
        .await
        .unwrap();
    assert_eq!(output_id, "out-1");
}

#[tokio::test]
async fn start_conversion_sends_the_password_when_one_is_given() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {}}),
            )
            .await;
            let (_, ref_, topic, event, payload) = recv(&mut ws).await;
            assert_eq!(event, "start_conversion");
            assert_eq!(payload["password"], "hunter2");
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {"output_id": "out-1"}}),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    let output_id = channel
        .start_conversion(OutputFormat::Pdf, Some("hunter2"))
        .await
        .unwrap();
    assert_eq!(output_id, "out-1");
}

#[tokio::test]
async fn start_conversion_maps_password_required_to_its_own_error() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {}}),
            )
            .await;
            let (_, ref_, topic, _event, _payload) = recv(&mut ws).await;
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({
                    "status": "error",
                    "response": {"reason": "password_required"}
                }),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    let err = channel
        .start_conversion(OutputFormat::Pdf, None)
        .await
        .unwrap_err();
    assert!(matches!(err, ScribeError::PasswordRequired { .. }));
}

#[tokio::test]
async fn a_password_required_push_surfaces_as_an_event() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                serde_json::json!({"status": "ok", "response": {}}),
            )
            .await;
            send(
                &mut ws,
                &join_ref,
                None,
                &topic,
                "password_required",
                serde_json::json!({}),
            )
            .await;
        })
    })
    .await;
    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    assert_eq!(
        channel.next_event().await.unwrap(),
        ChannelEvent::PasswordRequired
    );
}

/// Replies to the `phx_join` that opening a channel sends.
async fn accept_join(ws: &mut WebSocketStream<TcpStream>) -> (String, String) {
    let (join_ref, ref_, topic, event, _payload) = recv(ws).await;
    assert_eq!(event, "phx_join");
    let join_ref = join_ref.unwrap();
    send(
        ws,
        &join_ref,
        ref_.as_deref(),
        &topic,
        "phx_reply",
        serde_json::json!({"status": "ok", "response": {}}),
    )
    .await;
    (join_ref, topic)
}

/// Phoenix closes a socket it has heard nothing from for about 66 seconds,
/// however much it's sending, so waiting on a long conversion needs these.
#[tokio::test(start_paused = true)]
async fn waiting_for_an_event_sends_heartbeats_until_one_arrives() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            let (join_ref, topic) = accept_join(&mut ws).await;
            for _ in 0..3 {
                let (heartbeat_join_ref, ref_, heartbeat_topic, event, _) = recv(&mut ws).await;
                assert_eq!(heartbeat_join_ref, None);
                assert_eq!(heartbeat_topic, "phoenix");
                assert_eq!(event, "heartbeat");
                let reply: RawFrame = (
                    None,
                    ref_,
                    "phoenix".to_string(),
                    "phx_reply".to_string(),
                    serde_json::json!({"status": "ok", "response": {}}),
                );
                ws.send(Message::text(serde_json::to_string(&reply).unwrap()))
                    .await
                    .unwrap();
            }
            send(
                &mut ws,
                &join_ref,
                None,
                &topic,
                "conversion_complete",
                serde_json::json!({"format": "pdf", "output_id": "out-1"}),
            )
            .await;
            // Hold the connection open until the client has read the event.
            let _ = ws.next().await;
        })
    })
    .await;

    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();
    let started = tokio::time::Instant::now();

    let event = tokio::time::timeout(Duration::from_secs(300), channel.next_event())
        .await
        .expect("the server should have heard three heartbeats by now")
        .unwrap();

    assert_eq!(
        event,
        ChannelEvent::ConversionComplete {
            format: OutputFormat::Pdf,
            output_id: "out-1".to_string(),
        }
    );
    // Only a lower bound: the paused clock jumps ahead while frames are in
    // flight, so it can read later than the third heartbeat really was.
    let waited = started.elapsed().as_secs();
    assert!(
        waited >= 90,
        "heartbeats should be 30 seconds apart, but three took {waited}s"
    );
}

#[tokio::test]
async fn close_succeeds_after_the_server_closed_the_connection() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            accept_join(&mut ws).await;
            ws.close(None).await.unwrap();
            while ws.next().await.is_some() {}
        })
    })
    .await;

    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();

    assert!(matches!(
        channel.next_event().await,
        Err(ScribeError::ChannelClosed)
    ));
    channel.close().await.unwrap();
}

#[tokio::test]
async fn close_succeeds_after_the_connection_was_lost() {
    let base_url = start_fake_server(|mut ws| {
        Box::pin(async move {
            accept_join(&mut ws).await;
            // Dropped without a closing handshake, as a lost network looks.
        })
    })
    .await;

    let client = client_for(base_url);
    let mut channel = client.open_document_channel("doc-1").await.unwrap();

    assert!(channel.next_event().await.is_err());
    channel.close().await.unwrap();
}

/// Answers the join with `reply`, then pushes each of `pushes` as an event.
async fn serve_join(reply: Value, pushes: Vec<(&'static str, Value)>) -> Url {
    start_fake_server(move |mut ws| {
        Box::pin(async move {
            let (join_ref, ref_, topic, _event, _payload) = recv(&mut ws).await;
            let join_ref = join_ref.unwrap();
            send(
                &mut ws,
                &join_ref,
                Some(&ref_.unwrap()),
                &topic,
                "phx_reply",
                reply,
            )
            .await;
            for (event, payload) in pushes {
                send(&mut ws, &join_ref, None, &topic, event, payload).await;
            }
        })
    })
    .await
}

#[tokio::test]
async fn a_join_error_shows_the_servers_message() {
    let base_url = serve_join(
        serde_json::json!({"status": "error", "response": {
            "reason": "forbidden",
            "message": "Sie haben keine Berechtigung, auf dieses Dokument zuzugreifen."
        }}),
        vec![],
    )
    .await;
    match client_for(base_url).open_document_channel("doc-1").await {
        Err(ScribeError::Forbidden { message }) => {
            assert_eq!(
                message,
                "Sie haben keine Berechtigung, auf dieses Dokument zuzugreifen."
            );
        }
        Err(other) => panic!("expected Forbidden, got {other:?}"),
        Ok(_) => panic!("expected Forbidden, got a channel"),
    }
}

#[tokio::test]
async fn a_join_error_without_a_message_falls_back_to_english() {
    let base_url = serve_join(
        serde_json::json!({"status": "error", "response": {"reason": "not_found"}}),
        vec![],
    )
    .await;
    match client_for(base_url).open_document_channel("doc-1").await {
        Err(ScribeError::NotFound { message }) => {
            assert_eq!(message, "We couldn't find that document.");
        }
        Err(other) => panic!("expected NotFound, got {other:?}"),
        Ok(_) => panic!("expected NotFound, got a channel"),
    }
}

#[tokio::test]
async fn an_error_event_carries_the_servers_message_when_there_is_one() {
    let base_url = serve_join(
        serde_json::json!({"status": "ok", "response": {}}),
        vec![
            (
                "error",
                serde_json::json!({
                    "reason": "invalid_password",
                    "message": "Ce mot de passe n'a pas fonctionné."
                }),
            ),
            ("error", serde_json::json!({"reason": "conversion_failed"})),
        ],
    )
    .await;
    let mut channel = client_for(base_url)
        .open_document_channel("doc-1")
        .await
        .unwrap();
    assert_eq!(
        channel.next_event().await.unwrap(),
        ChannelEvent::Error {
            reason: "invalid_password".into(),
            message: Some("Ce mot de passe n'a pas fonctionné.".into()),
        }
    );
    assert_eq!(
        channel.next_event().await.unwrap(),
        ChannelEvent::Error {
            reason: "conversion_failed".into(),
            message: None,
        }
    );
}
