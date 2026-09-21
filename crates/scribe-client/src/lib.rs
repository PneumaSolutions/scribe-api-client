//! Rust client for the Scribe document conversion API.

mod auth;
mod channel;
mod client;
mod error;
mod model;

pub use auth::{AuthClient, PkceChallenge, TokenSet};
pub use channel::{ChannelEvent, DocumentChannel};
pub use client::{DocumentSource, ScribeClient, DELETE_ACCOUNT_PATH};
pub use error::ScribeError;
pub use model::{
    AccountInfo, BrailleTable, CreatedDocument, Dialect, DocumentList, DocumentSummary, Language,
    NotificationSettings, Output, OutputFormat, OutputList, Settings, SettingsUpdate, Stage,
    TrashedDocument, Voice,
};
