//! The real-time document channel (`documents:<id>` over `/socket`).

use std::collections::VecDeque;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::{
    error::ScribeError,
    model::{OutputFormat, Stage},
};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A Phoenix channel wire-protocol frame: `[join_ref, ref, topic, event, payload]`.
type RawFrame = (Option<String>, Option<String>, String, String, Value);

/// An event pushed asynchronously over a [`DocumentChannel`], outside of a
/// direct reply to something we sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelEvent {
    /// A conversion's stage or progress changed.
    Status {
        format: OutputFormat,
        stage: Stage,
        progress: f64,
    },
    /// A chunk of streamed HTML content. Only sent for the `html_stream`
    /// format while it's still converting.
    Chunk { content: String },
    /// A format finished converting.
    ConversionComplete {
        format: OutputFormat,
        output_id: String,
    },
    /// The server reported an error unrelated to a specific request we
    /// made (e.g. a conversion failed after it had already started).
    Error { reason: String },
}

#[derive(Debug, Deserialize)]
struct ReplyPayload {
    status: String,
    #[serde(default)]
    response: Value,
}

#[derive(Debug, Deserialize)]
struct ErrorReason {
    reason: String,
    #[serde(default)]
    purchase_url: Option<String>,
}

/// A live connection to a document's real-time channel, joined via
/// [`crate::ScribeClient::open_document_channel`].
///
/// Progress on formats that were already converting (or complete) when the
/// channel was joined arrives as [`ChannelEvent`]s from [`Self::next_event`]
/// just like anything started with [`Self::start_conversion`] after joining.
pub struct DocumentChannel {
    ws: WsStream,
    topic: String,
    join_ref: String,
    next_ref: u64,
    pending: VecDeque<ChannelEvent>,
}

impl DocumentChannel {
    pub(crate) async fn join(mut ws: WsStream, document_id: &str) -> Result<Self, ScribeError> {
        let topic = format!("documents:{document_id}");
        let join_ref = "1".to_string();
        send_frame(
            &mut ws,
            &join_ref,
            &join_ref,
            &topic,
            "phx_join",
            Value::Object(serde_json::Map::new()),
        )
        .await?;
        let mut channel = DocumentChannel {
            ws,
            topic,
            join_ref: join_ref.clone(),
            next_ref: 2,
            pending: VecDeque::new(),
        };
        channel.await_reply(&join_ref).await?;
        Ok(channel)
    }

    /// Starts converting the joined document to `format`, using its current
    /// settings. Idempotent: if that format is already converting or
    /// complete, the server returns its existing output id instead of
    /// starting a new conversion. Returns immediately with the output id;
    /// progress arrives via subsequent [`Self::next_event`] calls.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller,
    /// [`ScribeError::ConversionInProgress`] if a different non-preview
    /// conversion is already running, [`ScribeError::RateLimited`] if too
    /// many conversions were started too quickly, or
    /// [`ScribeError::NeedsPurchase`] if the account is out of page
    /// credits.
    pub async fn start_conversion(&mut self, format: OutputFormat) -> Result<String, ScribeError> {
        let ref_ = self.take_ref();
        let payload = serde_json::json!({ "format": format.as_str() });
        send_frame(
            &mut self.ws,
            &self.join_ref,
            &ref_,
            &self.topic,
            "start_conversion",
            payload,
        )
        .await?;
        let response = self.await_reply(&ref_).await?;
        response
            .get("output_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(ScribeError::ChannelClosed)
    }

    /// # Errors
    ///
    /// Returns [`ScribeError::WebSocket`] if the connection fails, or
    /// [`ScribeError::ChannelClosed`] if it closes before another event
    /// arrives.
    pub async fn next_event(&mut self) -> Result<ChannelEvent, ScribeError> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(event);
        }
        loop {
            let (_, _, topic, event, payload) = self.recv_frame().await?;
            if topic != self.topic {
                continue;
            }
            if let Some(event) = parse_event(&event, &payload) {
                return Ok(event);
            }
        }
    }

    /// # Errors
    ///
    /// Returns [`ScribeError::WebSocket`] if sending the close frame fails.
    pub async fn close(mut self) -> Result<(), ScribeError> {
        self.ws
            .close(None)
            .await
            .map_err(|e| ScribeError::WebSocket(Box::new(e)))
    }

    fn take_ref(&mut self) -> String {
        let r = self.next_ref.to_string();
        self.next_ref += 1;
        r
    }

    async fn recv_frame(&mut self) -> Result<RawFrame, ScribeError> {
        loop {
            return match self.ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    Ok(serde_json::from_str::<RawFrame>(text.as_str())?)
                }
                Some(Ok(
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) | Message::Binary(_),
                )) => continue,
                Some(Ok(Message::Close(_))) | None => Err(ScribeError::ChannelClosed),
                Some(Err(e)) => Err(ScribeError::WebSocket(Box::new(e))),
            };
        }
    }

    /// Reads frames until the reply matching `ref_` on our topic arrives,
    /// buffering any other events we recognize along the way.
    async fn await_reply(&mut self, ref_: &str) -> Result<Value, ScribeError> {
        loop {
            let (_, frame_ref, topic, event, payload) = self.recv_frame().await?;
            if topic != self.topic {
                continue;
            }
            if event == "phx_reply" && frame_ref.as_deref() == Some(ref_) {
                let reply: ReplyPayload = serde_json::from_value(payload)?;
                return match reply.status.as_str() {
                    "ok" => Ok(reply.response),
                    _ => Err(map_channel_error(reply.response)),
                };
            }
            if let Some(event) = parse_event(&event, &payload) {
                self.pending.push_back(event);
            }
        }
    }
}

async fn send_frame(
    ws: &mut WsStream,
    join_ref: &str,
    ref_: &str,
    topic: &str,
    event: &str,
    payload: Value,
) -> Result<(), ScribeError> {
    let frame: RawFrame = (
        Some(join_ref.to_string()),
        Some(ref_.to_string()),
        topic.to_string(),
        event.to_string(),
        payload,
    );
    let text = serde_json::to_string(&frame)?;
    ws.send(Message::text(text))
        .await
        .map_err(|e| ScribeError::WebSocket(Box::new(e)))?;
    Ok(())
}

fn parse_event(event: &str, payload: &Value) -> Option<ChannelEvent> {
    match event {
        "status" => {
            let format = OutputFormat::parse(payload.get("format")?.as_str()?)?;
            let stage = serde_json::from_value(payload.get("stage")?.clone()).ok()?;
            let progress = payload.get("progress")?.as_f64()?;
            Some(ChannelEvent::Status {
                format,
                stage,
                progress,
            })
        }
        "chunk" => Some(ChannelEvent::Chunk {
            content: payload.get("content")?.as_str()?.to_string(),
        }),
        "conversion_complete" => Some(ChannelEvent::ConversionComplete {
            format: OutputFormat::parse(payload.get("format")?.as_str()?)?,
            output_id: payload.get("output_id")?.as_str()?.to_string(),
        }),
        "error" => Some(ChannelEvent::Error {
            reason: payload.get("reason")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

fn map_channel_error(payload: Value) -> ScribeError {
    let Ok(err) = serde_json::from_value::<ErrorReason>(payload) else {
        return ScribeError::ChannelClosed;
    };
    match err.reason.as_str() {
        "not_found" => ScribeError::NotFound,
        "forbidden" => ScribeError::Forbidden,
        "conversion_in_progress" => ScribeError::ConversionInProgress,
        "rate_limited" => ScribeError::RateLimited,
        "needs_purchase" => ScribeError::NeedsPurchase {
            purchase_url: err.purchase_url.unwrap_or_default(),
        },
        other => ScribeError::Channel(other.to_string()),
    }
}
