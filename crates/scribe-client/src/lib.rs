//! Rust client for the Scribe document conversion API.

mod auth;
mod channel;
mod client;
mod error;
mod model;

pub use auth::{AuthClient, PkceChallenge, TokenSet};
pub use channel::{ChannelEvent, DocumentChannel};
pub use client::{DocumentSource, ScribeClient};
pub use error::ScribeError;
pub use model::{
    BrailleTable, CreatedDocument, Dialect, DocumentList, DocumentSummary, Language, Output,
    OutputFormat, Settings, SettingsUpdate, Stage, Voice,
};
