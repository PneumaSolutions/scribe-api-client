use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScribeError {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to parse response body: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid base URL or endpoint path: {0}")]
    Url(#[from] url::ParseError),
    #[error("{status}: {error}")]
    Api { status: u16, error: String },
    #[error("invalid_grant: {0}")]
    InvalidGrant(String),
    #[error("document is not finished converting yet")]
    ConversionNotComplete,
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    /// The document channel's WebSocket connection failed or was closed
    /// unexpectedly. Boxed: `tungstenite::Error` is large enough on its own
    /// to blow up the size of every other, far more common, error path.
    #[error("channel connection failed: {0}")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("channel closed before a reply arrived")]
    ChannelClosed,
    #[error("a conversion is already in progress for this document")]
    ConversionInProgress,
    #[error("rate limited, try again shortly")]
    RateLimited,
    #[error("insufficient page credits; purchase more at {purchase_url}")]
    NeedsPurchase { purchase_url: String },
    #[error("channel error: {0}")]
    Channel(String),
}
