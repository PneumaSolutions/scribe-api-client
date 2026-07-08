//! Rust client for the Scribe document conversion API: OAuth 2.0
//! Authorization Code + PKCE authentication, document
//! create/list/delete/download operations, and the real-time document
//! channel for starting and watching conversions.
//!
//! This crate has no `PyO3` dependency; see `scribe-client-py` in the same
//! workspace for Python bindings.

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
    BrailleTable, CreatedDocument, Dialect, DocumentSummary, Language, Output, OutputFormat,
    Settings, SettingsUpdate, Stage, Voice,
};
