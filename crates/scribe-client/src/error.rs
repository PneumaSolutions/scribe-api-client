use thiserror::Error;

/// Every variant that can originate from a server response carries a
/// `message: String` — the server's own human-readable sentence, passed
/// through unchanged. Never build user-facing text from a status code,
/// error code, or the internals of `reqwest`/`serde_json` here; those
/// stay out of `Display` (available via `.source()` for logging only).
#[derive(Debug, Error)]
pub enum ScribeError {
    #[error("Couldn't connect to Scribe. Check your connection and try again.")]
    Http(#[from] reqwest::Error),
    #[error("Something went wrong. Please try again.")]
    Decode(#[from] serde_json::Error),
    #[error("Something went wrong. Please try again.")]
    Url(#[from] url::ParseError),
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("{message}")]
    InvalidGrant { message: String },
    #[error("{message}")]
    ConversionNotComplete { message: String },
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    Forbidden { message: String },
    #[error("{message}")]
    NotTrashed { message: String },
    /// The document channel's WebSocket connection failed or was closed
    /// unexpectedly. Boxed: `tungstenite::Error` is large enough on its own
    /// to blow up the size of every other, far more common, error path.
    #[error("Couldn't connect to Scribe. Check your connection and try again.")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("channel closed before a reply arrived")]
    ChannelClosed,
    // These three only ever arise via the WebSocket channel today (see
    // channel.rs) — the server's channel error frames don't carry a
    // `message` field yet, so these keep the same friendly text that was
    // already hardcoded here before, just reshaped consistently with
    // everything else.
    #[error("{message}")]
    ConversionInProgress { message: String },
    #[error("{message}")]
    RateLimited { message: String },
    #[error("{message}")]
    NeedsPurchase { message: String, purchase_url: String },
    #[error("{message}")]
    Channel { message: String },
}
