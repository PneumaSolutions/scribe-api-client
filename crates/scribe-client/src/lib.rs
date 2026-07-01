//! Rust client for the Scribe document conversion API: OAuth 2.0
//! Authorization Code + PKCE authentication, and document
//! create/list-outputs/download operations.
//!
//! This crate has no PyO3 dependency; see `scribe-client-py` in the same
//! workspace for Python bindings.

mod auth;
mod client;
mod error;
mod model;

pub use auth::{AuthClient, PkceChallenge, TokenSet};
pub use client::{DocumentSource, ScribeClient};
pub use error::ScribeError;
pub use model::{CreatedDocument, Output, OutputFormat, Stage};
