//! The real-time document channel (`documents:<id>` over `/socket`).

use std::{collections::VecDeque, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{net::TcpStream, time::Instant};
use tokio_tungstenite::{
    tungstenite::{error::ProtocolError, Error as WsError, Message},
    MaybeTlsStream, WebSocketStream,
};

use crate::{
    error::ScribeError,
    model::{OutputFormat, Stage},
};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A Phoenix channel wire-protocol frame: `[join_ref, ref, topic, event, payload]`.
type RawFrame = (Option<String>, Option<String>, String, String, Value);

/// How often to send a heartbeat while waiting for a frame. The server closes
/// a connection it has heard nothing from for about 66 seconds, however much
/// it's sending; 30 seconds is what Phoenix's own client uses.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

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
    /// `message` is a sentence to show for it, in the client's language,
    /// from servers that send one.
    Error {
        reason: String,
        message: Option<String>,
    },
    /// The document is password-protected and hasn't been unlocked yet.
    /// Pushed on join, since a locked document has no outputs and would
    /// otherwise look like a silent channel. Prompt for the password and
    /// pass it to [`DocumentChannel::start_conversion`].
    PasswordRequired,
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
    message: Option<String>,
    #[serde(default)]
    purchase_url: Option<String>,
}

/// A live connection to a document's real-time channel, joined via
/// [`crate::ScribeClient::open_document_channel`].
///
/// Progress on formats that were already converting (or complete) when the
/// channel was joined arrives as [`ChannelEvent`]s from [`Self::next_event`]
/// just like anything started with [`Self::start_conversion`] after joining.
///
/// Heartbeats keep the connection open only while a call on this channel is
/// waiting, so keep calling [`Self::next_event`] during a conversion. The
/// connection can still be lost, for example to a network failure or a server
/// restart. A conversion carries on without it: open a new channel on the same
/// document, and don't upload it again.
pub struct DocumentChannel {
    ws: WsStream,
    topic: String,
    join_ref: String,
    next_ref: u64,
    pending: VecDeque<ChannelEvent>,
    next_heartbeat: Instant,
}

impl DocumentChannel {
    pub(crate) async fn join(mut ws: WsStream, document_id: &str) -> Result<Self, ScribeError> {
        let topic = format!("documents:{document_id}");
        let join_ref = "1".to_string();
        send_frame(
            &mut ws,
            Some(&join_ref),
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
            next_heartbeat: Instant::now() + HEARTBEAT_INTERVAL,
        };
        channel.await_reply(&join_ref).await?;
        Ok(channel)
    }

    /// Starts converting the joined document to `format`, using its current
    /// settings. If that format is already complete, the server returns its
    /// existing output id instead of starting a new conversion. Returns
    /// immediately with the output id; progress arrives via subsequent
    /// [`Self::next_event`] calls.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller,
    /// [`ScribeError::ConversionInProgress`] if any non-preview conversion
    /// of the document is running, including that format and the
    /// `html_stream` that creating the document started (wait for its
    /// [`ChannelEvent::ConversionComplete`], then call this again),
    /// [`ScribeError::RateLimited`] if too many conversions were started too
    /// quickly,
    /// [`ScribeError::NeedsPurchase`] if the account is out of page
    /// credits, or [`ScribeError::PasswordRequired`] if the document is a
    /// protected file and `password` was `None` or wrong.
    ///
    /// `password` unlocks a password-protected document. It's stored
    /// against the document, so later conversions of other formats don't
    /// need it again. A wrong password isn't detected here — the server
    /// accepts it, the converter rejects it, and an
    /// [`ChannelEvent::Error`] with reason `invalid_password` arrives from
    /// [`Self::next_event`]; retry this call with a corrected password.
    pub async fn start_conversion(
        &mut self,
        format: OutputFormat,
        password: Option<&str>,
    ) -> Result<String, ScribeError> {
        let ref_ = self.take_ref();
        let mut payload = serde_json::json!({ "format": format.as_str() });
        if let Some(password) = password {
            payload["password"] = Value::String(password.to_string());
        }
        send_frame(
            &mut self.ws,
            Some(&self.join_ref),
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

    /// Closes the connection. One that's already closed or lost is fine, since
    /// that's what this is for.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::WebSocket`] if closing the connection fails with
    /// it still open.
    pub async fn close(mut self) -> Result<(), ScribeError> {
        match self.ws.close(None).await {
            Ok(())
            | Err(
                WsError::ConnectionClosed
                | WsError::AlreadyClosed
                | WsError::Protocol(ProtocolError::SendAfterClosing)
                | WsError::Io(_),
            ) => Ok(()),
            Err(e) => Err(ScribeError::WebSocket(Box::new(e))),
        }
    }

    fn take_ref(&mut self) -> String {
        let r = self.next_ref.to_string();
        self.next_ref += 1;
        r
    }

    async fn send_heartbeat(&mut self) -> Result<(), ScribeError> {
        let ref_ = self.take_ref();
        self.next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        send_frame(
            &mut self.ws,
            None,
            &ref_,
            "phoenix",
            "heartbeat",
            Value::Object(serde_json::Map::new()),
        )
        .await
    }

    async fn recv_frame(&mut self) -> Result<RawFrame, ScribeError> {
        loop {
            if Instant::now() >= self.next_heartbeat {
                self.send_heartbeat().await?;
            }
            let Ok(message) = tokio::time::timeout_at(self.next_heartbeat, self.ws.next()).await
            else {
                continue;
            };
            return match message {
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
    join_ref: Option<&str>,
    ref_: &str,
    topic: &str,
    event: &str,
    payload: Value,
) -> Result<(), ScribeError> {
    let frame: RawFrame = (
        join_ref.map(str::to_string),
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
            message: payload
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        "password_required" => Some(ChannelEvent::PasswordRequired),
        _ => None,
    }
}

fn map_channel_error(payload: Value) -> ScribeError {
    let Ok(err) = serde_json::from_value::<ErrorReason>(payload) else {
        return ScribeError::ChannelClosed;
    };
    // The server's own message is in the language the client asked for.
    // A server too old to send one gets the English these codes have always
    // shown.
    let message = |english: &str| err.message.clone().unwrap_or_else(|| english.to_string());
    match err.reason.as_str() {
        "not_found" => ScribeError::NotFound {
            message: message("We couldn't find that document."),
        },
        "forbidden" => ScribeError::Forbidden {
            message: message("You don't have permission to access that document."),
        },
        "conversion_in_progress" => ScribeError::ConversionInProgress {
            message: message("A conversion is already in progress for this document."),
        },
        "rate_limited" => ScribeError::RateLimited {
            message: message("Rate limited, try again shortly."),
        },
        "password_required" => ScribeError::PasswordRequired {
            message: message(
                "This document is password-protected. Enter its password to continue.",
            ),
        },
        "needs_purchase" => ScribeError::NeedsPurchase {
            message: message("You need to purchase more pages to do that."),
            purchase_url: err.purchase_url.clone().unwrap_or_default(),
        },
        _ => ScribeError::Channel {
            message: message("Something went wrong. Please try again."),
        },
    }
}
