use thiserror::Error;

/// Errors returned by this crate.
#[derive(Debug, Error)]
pub enum ScribeError {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to parse response body: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("invalid base URL or endpoint path: {0}")]
    Url(#[from] url::ParseError),

    /// The server rejected the request with a JSON `{"error": ...}` body.
    #[error("{status}: {error}")]
    Api { status: u16, error: String },

    /// `POST /oauth/token` specifically returned `400 invalid_grant`, meaning
    /// the authorization code, refresh token, or PKCE verifier didn't check out.
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

    /// The document channel closed (or replied with an error) before a
    /// reply matching our request ever arrived.
    #[error("channel closed before a reply arrived")]
    ChannelClosed,

    /// Another (non-preview) conversion is already in progress for this
    /// document; only one can run at a time.
    #[error("a conversion is already in progress for this document")]
    ConversionInProgress,

    /// `start_conversion` was rate-limited; back off and retry.
    #[error("rate limited, try again shortly")]
    RateLimited,

    /// The document doesn't have enough page credits for this conversion.
    /// `purchase_url` is where the resource owner can buy more.
    #[error("insufficient page credits; purchase more at {purchase_url}")]
    NeedsPurchase { purchase_url: String },

    /// A channel error reason we don't have a dedicated variant for yet.
    #[error("channel error: {0}")]
    Channel(String),
}
