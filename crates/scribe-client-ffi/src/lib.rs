//! UniFFI bindings for `scribe-client`, targeting iOS (Swift) and Android (Kotlin).
//!
//! All async operations are executed synchronously on a shared Tokio runtime via
//! `block_on`, matching the same pattern as the PyO3 bindings. Swift callers
//! should dispatch to a background thread / `Task.detached` to avoid blocking
//! the main actor.
//!
//! The modules mirror `scribe-client`'s own layout: [`error`] for the error
//! type crossing the boundary, [`model`] for the value types, and [`auth`],
//! [`client`] and [`channel`] for the three objects held by handle.

use std::sync::OnceLock;

use reqwest::Client;
use tokio::runtime::Runtime;

mod auth;
mod channel;
mod client;
mod error;
mod model;

pub use auth::{generate_pkce_session, FfiAuthClient};
pub use channel::{ChannelEvent, FfiDocumentChannel};
pub use client::FfiScribeClient;
pub use error::ScribeError;
pub use model::*;

pub(crate) use error::parse_url;

uniffi::setup_scaffolding!();

/// The Tokio runtime every blocking call runs on. One per process: the
/// bindings expose a synchronous API, so each call blocks on this rather than
/// standing up a runtime of its own.
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("failed to start scribe-client-ffi tokio runtime"))
}

fn http_client() -> Client {
    Client::new()
}
