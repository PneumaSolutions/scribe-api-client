// The README is the crate documentation, so its example is compiled as a
// doctest and cannot drift from the API.
#![doc = include_str!("../README.md")]

mod auth;
mod channel;
mod client;
mod error;
mod model;

// The constructors take a `reqwest::Client` and a `Url`, so these are the
// versions to build them with; another version of either is another type.
pub use reqwest;
pub use url::{self, Url};

pub use auth::{AuthClient, PkceChallenge, TokenSet};
pub use channel::{ChannelEvent, DocumentChannel};
pub use client::{DocumentSource, ScribeClient, DELETE_ACCOUNT_PATH};
pub use error::ScribeError;
pub use model::{
    AccountInfo, BillingStatus, BillingSubscription, BrailleTable, CreatedDocument, Dialect,
    DocumentList, DocumentSummary, Download, Language, NotificationSettings, Output, OutputFormat,
    OutputList, Settings, SettingsUpdate, Stage, TrashedDocument, Voice,
};
