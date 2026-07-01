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

    /// `POST /oauth/token` specifically returned `400 invalid_grant` — the
    /// authorization code, refresh token, or PKCE verifier didn't check out.
    #[error("invalid_grant: {0}")]
    InvalidGrant(String),

    #[error("document is not finished converting yet")]
    ConversionNotComplete,

    #[error("not found")]
    NotFound,

    #[error("forbidden")]
    Forbidden,
}
