//! The error type that crosses the FFI boundary, and the mapping from the
//! core crate's errors onto it.

use url::Url;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ScribeError {
    #[error("{message}")]
    Http { message: String },
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
    #[error("{message}")]
    ConversionInProgress { message: String },
    #[error("{message}")]
    RateLimited { message: String },
    #[error("{message}")]
    NeedsPurchase {
        message: String,
        purchase_url: String,
    },
    #[error("channel closed before a reply arrived")]
    ChannelClosed,
    #[error("{message}")]
    Other { message: String },
}

impl From<scribe_client_core::ScribeError> for ScribeError {
    fn from(e: scribe_client_core::ScribeError) -> Self {
        match e {
            // Transport-level failures — no server text to draw from.
            // Deliberately NOT calling `.to_string()` on the inner
            // reqwest/serde/url error (that would leak raw transport
            // internals); use the same fixed friendly text every other
            // layer of this app uses for "no server response at all".
            scribe_client_core::ScribeError::Http(_) => Self::Http {
                message: "Couldn't connect to Scribe. Check your connection and try again."
                    .to_string(),
            },
            scribe_client_core::ScribeError::Decode(_) => Self::Other {
                message: "Something went wrong. Please try again.".to_string(),
            },
            scribe_client_core::ScribeError::Url(_) => Self::Other {
                message: "Something went wrong. Please try again.".to_string(),
            },
            scribe_client_core::ScribeError::Api {
                status,
                code,
                message,
            } => Self::Api {
                status,
                code,
                message,
            },
            scribe_client_core::ScribeError::InvalidGrant { message } => {
                Self::InvalidGrant { message }
            }
            scribe_client_core::ScribeError::ConversionNotComplete { message } => {
                Self::ConversionNotComplete { message }
            }
            scribe_client_core::ScribeError::NotFound { message } => Self::NotFound { message },
            scribe_client_core::ScribeError::Forbidden { message } => Self::Forbidden { message },
            scribe_client_core::ScribeError::NotTrashed { message } => Self::NotTrashed { message },
            scribe_client_core::ScribeError::ConversionInProgress { message } => {
                Self::ConversionInProgress { message }
            }
            scribe_client_core::ScribeError::RateLimited { message } => {
                Self::RateLimited { message }
            }
            scribe_client_core::ScribeError::NeedsPurchase {
                message,
                purchase_url,
            } => Self::NeedsPurchase {
                message,
                purchase_url,
            },
            scribe_client_core::ScribeError::ChannelClosed => Self::ChannelClosed,
            scribe_client_core::ScribeError::WebSocket(_) => Self::Other {
                message: "Couldn't connect to Scribe. Check your connection and try again."
                    .to_string(),
            },
            scribe_client_core::ScribeError::Channel { message } => Self::Other { message },
        }
    }
}

pub(crate) fn parse_url(raw: &str) -> Result<Url, ScribeError> {
    Url::parse(raw).map_err(|e| ScribeError::Other {
        message: format!("invalid URL {raw:?}: {e}"),
    })
}
