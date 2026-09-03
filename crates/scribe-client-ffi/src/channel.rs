//! The streaming conversion channel, and the events it delivers.

use std::sync::Mutex;

use scribe_client_core::{ChannelEvent as CoreChannelEvent, DocumentChannel};

use crate::{runtime, OutputFormat, ScribeError, Stage};

/// An asynchronous event pushed over a [`FfiDocumentChannel`], outside of a
/// direct reply to something the app sent.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
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
    /// The server reported an error unrelated to a specific request the
    /// app made (e.g. a conversion failed after it had already started).
    Error { reason: String },
}

impl From<CoreChannelEvent> for ChannelEvent {
    fn from(e: CoreChannelEvent) -> Self {
        match e {
            CoreChannelEvent::Status {
                format,
                stage,
                progress,
            } => Self::Status {
                format: format.into(),
                stage: stage.into(),
                progress,
            },
            CoreChannelEvent::Chunk { content } => Self::Chunk { content },
            CoreChannelEvent::ConversionComplete { format, output_id } => {
                Self::ConversionComplete {
                    format: format.into(),
                    output_id,
                }
            }
            CoreChannelEvent::Error { reason } => Self::Error { reason },
        }
    }
}

fn channel_closed_err() -> ScribeError {
    ScribeError::ChannelClosed
}

/// A live connection to a document's real-time channel, obtained from
/// [`FfiScribeClient::open_document_channel`]. This is the only way to
/// start converting a format other than the `html_stream` preview that
/// document creation already starts.
///
/// `UniFFI` objects are shared across the FFI boundary (`Arc<Self>`), so the
/// underlying [`DocumentChannel`] (whose methods need exclusive access to
/// its `WebSocket` connection) is guarded by a mutex rather than held by
/// value.
#[derive(uniffi::Object)]
pub struct FfiDocumentChannel {
    inner: Mutex<Option<DocumentChannel>>,
}

impl FfiDocumentChannel {
    /// Wraps a channel the client has just opened. Not exported: a channel is
    /// only ever handed out by `FfiScribeClient::open_document_channel`, which
    /// is what keeps the field private to this module.
    pub(crate) fn new(inner: DocumentChannel) -> Self {
        FfiDocumentChannel {
            inner: Mutex::new(Some(inner)),
        }
    }
}

#[uniffi::export]
impl FfiDocumentChannel {
    /// Starts converting the joined document to `format`, using its
    /// current settings. Idempotent: if that format is already converting
    /// or complete, returns its existing output id. Returns immediately;
    /// progress arrives via subsequent [`Self::next_event`] calls.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::ConversionInProgress`] if a different
    /// non-preview conversion is already running,
    /// [`ScribeError::RateLimited`] if too many conversions were started
    /// too quickly, [`ScribeError::NeedsPurchase`] if the account is out
    /// of page credits, or [`ScribeError::ChannelClosed`] if the channel
    /// was already closed.
    pub fn start_conversion(&self, format: OutputFormat) -> Result<String, ScribeError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let channel = guard.as_mut().ok_or_else(channel_closed_err)?;
        runtime()
            .block_on(channel.start_conversion(format.into()))
            .map_err(Into::into)
    }

    /// Blocks until the next asynchronous event arrives on this channel.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::ChannelClosed`] if the channel is already
    /// closed, or closes before another event arrives.
    pub fn next_event(&self) -> Result<ChannelEvent, ScribeError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let channel = guard.as_mut().ok_or_else(channel_closed_err)?;
        runtime()
            .block_on(channel.next_event())
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Leaves the channel and closes the underlying connection. Safe to
    /// call more than once.
    ///
    /// # Errors
    ///
    /// Returns an error if sending the close frame fails.
    pub fn close(&self) -> Result<(), ScribeError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(channel) = guard.take() {
            runtime().block_on(channel.close()).map_err(Into::into)
        } else {
            Ok(())
        }
    }
}
