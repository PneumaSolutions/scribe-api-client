//! UniFFI bindings for `scribe-client`, targeting iOS (Swift) and Android (Kotlin).
//!
//! All async operations are executed synchronously on a shared Tokio runtime via
//! `block_on`, matching the same pattern as the PyO3 bindings. Swift callers
//! should dispatch to a background thread / `Task.detached` to avoid blocking
//! the main actor.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use reqwest::Client;
use time::OffsetDateTime;
use tokio::runtime::Runtime;
use url::Url;

use scribe_client_core::{
    AccountInfo as CoreAccountInfo, AuthClient, ChannelEvent as CoreChannelEvent, DocumentChannel,
    DocumentSource, OutputFormat as CoreOutputFormat, ScribeClient,
    SettingsUpdate as CoreSettingsUpdate, Stage as CoreStage,
};

uniffi::setup_scaffolding!();

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("failed to start scribe-client-ffi tokio runtime"))
}

fn http_client() -> Client {
    Client::new()
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ScribeError {
    #[error("request failed: {message}")]
    Http { message: String },
    #[error("{status}: {error}")]
    Api { status: u16, error: String },
    #[error("invalid_grant: {message}")]
    InvalidGrant { message: String },
    #[error("conversion not complete")]
    ConversionNotComplete,
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("a conversion is already in progress for this document")]
    ConversionInProgress,
    #[error("rate limited, try again shortly")]
    RateLimited,
    #[error("insufficient page credits; purchase more at {purchase_url}")]
    NeedsPurchase { purchase_url: String },
    #[error("channel closed before a reply arrived")]
    ChannelClosed,
    #[error("{message}")]
    Other { message: String },
}

impl From<scribe_client_core::ScribeError> for ScribeError {
    fn from(e: scribe_client_core::ScribeError) -> Self {
        match e {
            scribe_client_core::ScribeError::Http(e) => Self::Http {
                message: e.to_string(),
            },
            scribe_client_core::ScribeError::Decode(e) => Self::Other {
                message: e.to_string(),
            },
            scribe_client_core::ScribeError::Url(e) => Self::Other {
                message: e.to_string(),
            },
            scribe_client_core::ScribeError::Api { status, error } => Self::Api { status, error },
            scribe_client_core::ScribeError::InvalidGrant(message) => {
                Self::InvalidGrant { message }
            }
            scribe_client_core::ScribeError::ConversionNotComplete => Self::ConversionNotComplete,
            scribe_client_core::ScribeError::NotFound => Self::NotFound,
            scribe_client_core::ScribeError::Forbidden => Self::Forbidden,
            scribe_client_core::ScribeError::ConversionInProgress => Self::ConversionInProgress,
            scribe_client_core::ScribeError::RateLimited => Self::RateLimited,
            scribe_client_core::ScribeError::NeedsPurchase { purchase_url } => {
                Self::NeedsPurchase { purchase_url }
            }
            scribe_client_core::ScribeError::ChannelClosed => Self::ChannelClosed,
            // Transport-level failures rather than a business-meaningful
            // condition the app would branch on; fall back to a message,
            // same as the Python bindings do for these.
            other @ (scribe_client_core::ScribeError::WebSocket(_)
            | scribe_client_core::ScribeError::Channel(_)) => Self::Other {
                message: other.to_string(),
            },
        }
    }
}

fn parse_url(raw: &str) -> Result<Url, ScribeError> {
    Url::parse(raw).map_err(|e| ScribeError::Other {
        message: format!("invalid URL {raw:?}: {e}"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OutputFormat {
    Html,
    Pdf,
    Epub,
    Daisy,
    Docx,
    Brf,
    Mp3,
    OfflineHtml,
    Mobi,
    HtmlStream,
}

impl From<CoreOutputFormat> for OutputFormat {
    fn from(f: CoreOutputFormat) -> Self {
        match f {
            CoreOutputFormat::Html => Self::Html,
            CoreOutputFormat::Pdf => Self::Pdf,
            CoreOutputFormat::Epub => Self::Epub,
            CoreOutputFormat::Daisy => Self::Daisy,
            CoreOutputFormat::Docx => Self::Docx,
            CoreOutputFormat::Brf => Self::Brf,
            CoreOutputFormat::Mp3 => Self::Mp3,
            CoreOutputFormat::OfflineHtml => Self::OfflineHtml,
            CoreOutputFormat::Mobi => Self::Mobi,
            CoreOutputFormat::HtmlStream => Self::HtmlStream,
        }
    }
}

impl From<OutputFormat> for CoreOutputFormat {
    fn from(f: OutputFormat) -> Self {
        match f {
            OutputFormat::Html => Self::Html,
            OutputFormat::Pdf => Self::Pdf,
            OutputFormat::Epub => Self::Epub,
            OutputFormat::Daisy => Self::Daisy,
            OutputFormat::Docx => Self::Docx,
            OutputFormat::Brf => Self::Brf,
            OutputFormat::Mp3 => Self::Mp3,
            OutputFormat::OfflineHtml => Self::OfflineHtml,
            OutputFormat::Mobi => Self::Mobi,
            OutputFormat::HtmlStream => Self::HtmlStream,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Stage {
    Queue,
    Start,
    Convert,
    AddImageDescriptions,
    Complete,
}

impl From<CoreStage> for Stage {
    fn from(s: CoreStage) -> Self {
        match s {
            CoreStage::Queue => Self::Queue,
            CoreStage::Start => Self::Start,
            CoreStage::Convert => Self::Convert,
            CoreStage::AddImageDescriptions => Self::AddImageDescriptions,
            CoreStage::Complete => Self::Complete,
        }
    }
}

/// An OAuth 2.0 token pair. `expires_at_unix_secs` is a Unix timestamp (seconds
/// since epoch) when the access token expires, or `None` if the server didn't
/// report an expiry.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_unix_secs: Option<i64>,
}

impl From<scribe_client_core::TokenSet> for TokenSet {
    fn from(t: scribe_client_core::TokenSet) -> Self {
        TokenSet {
            access_token: t.access_token,
            refresh_token: t.refresh_token,
            expires_at_unix_secs: t.expires_at.map(|dt| dt.unix_timestamp()),
        }
    }
}

impl From<TokenSet> for scribe_client_core::TokenSet {
    fn from(t: TokenSet) -> Self {
        scribe_client_core::TokenSet {
            access_token: t.access_token,
            refresh_token: t.refresh_token,
            expires_at: t.expires_at_unix_secs.map(|secs| {
                OffsetDateTime::from_unix_timestamp(secs).unwrap_or(OffsetDateTime::UNIX_EPOCH)
            }),
        }
    }
}

/// The verifier and challenge for a single PKCE session. Keep the `verifier`
/// secret; pass only the `challenge` in the authorization URL.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PkceSession {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CreatedDocument {
    pub document_id: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AccountInfo {
    pub pages_remaining: i64,
}

impl From<CoreAccountInfo> for AccountInfo {
    fn from(a: CoreAccountInfo) -> Self {
        Self {
            pages_remaining: a.pages_remaining,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Output {
    pub format: OutputFormat,
    pub stage: Stage,
    pub progress: f64,
    pub estimated_time_remaining: Option<i64>,
    pub is_preview: bool,
}

impl From<scribe_client_core::Output> for Output {
    fn from(o: scribe_client_core::Output) -> Self {
        Output {
            format: o.format.into(),
            stage: o.stage.into(),
            progress: o.progress,
            estimated_time_remaining: o.estimated_time_remaining,
            is_preview: o.is_preview,
        }
    }
}

/// One row from `list_documents()`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DocumentSummary {
    pub id: String,
    pub title: Option<String>,
    pub page_count: Option<i64>,
    /// ISO 8601 UTC timestamp of when the document was created.
    pub inserted_at: String,
    pub outputs: Vec<Output>,
}

impl From<scribe_client_core::DocumentSummary> for DocumentSummary {
    fn from(d: scribe_client_core::DocumentSummary) -> Self {
        DocumentSummary {
            id: d.id,
            title: d.title,
            page_count: d.page_count,
            inserted_at: d.inserted_at,
            outputs: d.outputs.into_iter().map(Into::into).collect(),
        }
    }
}

/// A document's current conversion settings. `dialects_json` and `voices_json`
/// are JSON-serialized because their shape is a flexible server-defined map
/// that the caller can decode with a JSON library.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Settings {
    pub language: Option<String>,
    pub dialects_json: String,
    pub voices_json: String,
    pub tts_gender: Option<String>,
    pub tts_rate: f64,
    pub braille_translation_table: String,
    pub braille_cells_per_line: i64,
    pub braille_split_into_pages: bool,
    pub braille_lines_per_page: i64,
    pub large_print: bool,
    pub add_image_descriptions: bool,
    pub math: bool,
    pub notify_when_complete: bool,
}

impl From<scribe_client_core::Settings> for Settings {
    fn from(s: scribe_client_core::Settings) -> Self {
        Settings {
            language: s.language,
            dialects_json: s.dialects.to_string(),
            voices_json: s.voices.to_string(),
            tts_gender: s.tts_gender,
            tts_rate: s.tts_rate,
            braille_translation_table: s.braille_translation_table,
            braille_cells_per_line: s.braille_cells_per_line,
            braille_split_into_pages: s.braille_split_into_pages,
            braille_lines_per_page: s.braille_lines_per_page,
            large_print: s.large_print,
            add_image_descriptions: s.add_image_descriptions,
            math: s.math,
            notify_when_complete: s.notify_when_complete,
        }
    }
}

/// A partial update. Only `Some` fields are sent to the server; `None` fields
/// are left unchanged.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct SettingsUpdate {
    pub language: Option<String>,
    pub tts_gender: Option<String>,
    pub tts_rate: Option<f64>,
    pub braille_translation_table: Option<String>,
    pub braille_cells_per_line: Option<i64>,
    pub braille_split_into_pages: Option<bool>,
    pub braille_lines_per_page: Option<i64>,
    pub large_print: Option<bool>,
    pub add_image_descriptions: Option<bool>,
    pub math: Option<bool>,
    pub notify_when_complete: Option<bool>,
}

impl From<SettingsUpdate> for CoreSettingsUpdate {
    fn from(u: SettingsUpdate) -> Self {
        CoreSettingsUpdate {
            language: u.language,
            dialects: None,
            voices: None,
            tts_gender: u.tts_gender,
            tts_rate: u.tts_rate,
            braille_translation_table: u.braille_translation_table,
            braille_cells_per_line: u.braille_cells_per_line,
            braille_split_into_pages: u.braille_split_into_pages,
            braille_lines_per_page: u.braille_lines_per_page,
            large_print: u.large_print,
            add_image_descriptions: u.add_image_descriptions,
            math: u.math,
            notify_when_complete: u.notify_when_complete,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Language {
    pub display_name: String,
    pub code: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Dialect {
    pub display_name: String,
    pub locale: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BrailleTable {
    pub display_name: String,
    pub id: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Voice {
    pub display_name: String,
    pub short_name: String,
    pub has_sample: bool,
}

/// An asynchronous event pushed over a [`FfiDocumentChannel`], outside of a
/// direct reply to something the app sent.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ChannelEvent {
    /// A conversion's stage or progress changed.
    Status {
        format: OutputFormat,
        stage: Stage,
        progress: f64,
    },
    /// A chunk of streamed HTML content. Only sent for the `html_stream`
    /// format while it's still converting.
    Chunk { content: String },
    /// A format finished converting.
    ConversionComplete {
        format: OutputFormat,
        output_id: String,
    },
    /// The server reported an error unrelated to a specific request the
    /// app made (e.g. a conversion failed after it had already started).
    Error { reason: String },
}

impl From<CoreChannelEvent> for ChannelEvent {
    fn from(e: CoreChannelEvent) -> Self {
        match e {
            CoreChannelEvent::Status {
                format,
                stage,
                progress,
            } => Self::Status {
                format: format.into(),
                stage: stage.into(),
                progress,
            },
            CoreChannelEvent::Chunk { content } => Self::Chunk { content },
            CoreChannelEvent::ConversionComplete { format, output_id } => {
                Self::ConversionComplete {
                    format: format.into(),
                    output_id,
                }
            }
            CoreChannelEvent::Error { reason } => Self::Error { reason },
        }
    }
}

/// Generates a fresh PKCE verifier/challenge pair (RFC 7636, S256 method).
#[uniffi::export]
pub fn generate_pkce_session() -> PkceSession {
    let pkce = scribe_client_core::PkceChallenge::generate();
    PkceSession {
        verifier: pkce.verifier().to_string(),
        challenge: pkce.challenge().to_string(),
    }
}

/// Drives the OAuth 2.0 Authorization Code + PKCE flow. Does not open a
/// browser or handle the redirect; the app is responsible for presenting the
/// authorization URL and returning the resulting code.
#[derive(uniffi::Object)]
pub struct FfiAuthClient {
    http: Client,
    base_url: Url,
    client_id: String,
}

#[uniffi::export]
impl FfiAuthClient {
    #[uniffi::constructor]
    pub fn new(base_url: String, client_id: String) -> Result<Arc<Self>, ScribeError> {
        let base_url = parse_url(&base_url)?;
        Ok(Arc::new(FfiAuthClient {
            http: http_client(),
            base_url,
            client_id,
        }))
    }

    /// Returns the URL the user's browser should be sent to.
    /// `pkce_challenge` is the `challenge` field from [`generate_pkce_session`].
    pub fn authorization_url(
        &self,
        redirect_uri: String,
        pkce_challenge: String,
    ) -> Result<String, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/oauth/authorize");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &pkce_challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.to_string())
    }

    /// Exchanges an authorization code for tokens.
    /// `verifier` is the `verifier` field from the same [`generate_pkce_session`]
    /// call used to build the authorization URL.
    pub fn exchange_code(
        &self,
        redirect_uri: String,
        code: String,
        verifier: String,
    ) -> Result<TokenSet, ScribeError> {
        let auth = AuthClient::new(self.http.clone(), self.base_url.clone(), &self.client_id);
        runtime()
            .block_on(auth.exchange_code(&redirect_uri, &code, &verifier))
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Exchanges a refresh token for a new token set.
    pub fn refresh(&self, refresh_token: String) -> Result<TokenSet, ScribeError> {
        let auth = AuthClient::new(self.http.clone(), self.base_url.clone(), &self.client_id);
        runtime()
            .block_on(auth.refresh(&refresh_token))
            .map(Into::into)
            .map_err(Into::into)
    }
}

/// A client for the document-conversion endpoints. Holds a token set and
/// refreshes it automatically. Call [`FfiScribeClient::current_tokens`] after
/// any operation to persist the potentially-refreshed token set.
#[derive(uniffi::Object)]
pub struct FfiScribeClient {
    inner: ScribeClient,
}

#[uniffi::export]
impl FfiScribeClient {
    #[uniffi::constructor]
    pub fn new(
        base_url: String,
        client_id: String,
        tokens: TokenSet,
    ) -> Result<Arc<Self>, ScribeError> {
        let base_url = parse_url(&base_url)?;
        let http = http_client();
        let core_tokens: scribe_client_core::TokenSet = tokens.into();
        Ok(Arc::new(FfiScribeClient {
            inner: ScribeClient::new(http, base_url, client_id, core_tokens),
        }))
    }

    /// Returns the current token set, including any access token that was
    /// auto-refreshed since construction. Persist this after each operation.
    pub fn current_tokens(&self) -> TokenSet {
        runtime().block_on(self.inner.current_tokens()).into()
    }

    pub fn create_document_from_file(
        &self,
        file_name: String,
        bytes: Vec<u8>,
    ) -> Result<CreatedDocument, ScribeError> {
        let source = DocumentSource::File { file_name, bytes };
        runtime()
            .block_on(self.inner.create_document(source))
            .map(|d| CreatedDocument {
                document_id: d.document_id,
            })
            .map_err(Into::into)
    }

    pub fn create_document_from_url(&self, url: String) -> Result<CreatedDocument, ScribeError> {
        let source = DocumentSource::Url(url);
        runtime()
            .block_on(self.inner.create_document(source))
            .map(|d| CreatedDocument {
                document_id: d.document_id,
            })
            .map_err(Into::into)
    }

    pub fn list_documents(&self) -> Result<Vec<DocumentSummary>, ScribeError> {
        runtime()
            .block_on(self.inner.list_documents())
            .map(|docs| docs.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    pub fn delete_document(&self, document_id: String) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.delete_document(&document_id))
            .map_err(Into::into)
    }

    pub fn get_account_info(&self) -> Result<AccountInfo, ScribeError> {
        runtime()
            .block_on(self.inner.get_account_info())
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn submit_document_feedback(
        &self,
        document_id: String,
        comment: String,
    ) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.submit_document_feedback(&document_id, &comment))
            .map_err(Into::into)
    }

    /// Opens a real-time channel for `document_id`. This is the only way
    /// to start converting a format other than the `html_stream` preview
    /// that document creation already starts.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or an error if
    /// the connection fails.
    pub fn open_document_channel(
        &self,
        document_id: String,
    ) -> Result<Arc<FfiDocumentChannel>, ScribeError> {
        runtime()
            .block_on(self.inner.open_document_channel(&document_id))
            .map(|inner| {
                Arc::new(FfiDocumentChannel {
                    inner: Mutex::new(Some(inner)),
                })
            })
            .map_err(Into::into)
    }

    pub fn list_outputs(&self, document_id: String) -> Result<Vec<Output>, ScribeError> {
        runtime()
            .block_on(self.inner.list_outputs(&document_id))
            .map(|outs| outs.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    /// Downloads the raw bytes of a completed output.
    /// Returns `ScribeError::ConversionNotComplete` if still in progress.
    pub fn download_output(
        &self,
        document_id: String,
        format: OutputFormat,
    ) -> Result<Vec<u8>, ScribeError> {
        runtime()
            .block_on(self.inner.download_output(&document_id, format.into()))
            .map_err(Into::into)
    }

    pub fn get_settings(&self, document_id: String) -> Result<Settings, ScribeError> {
        runtime()
            .block_on(self.inner.get_settings(&document_id))
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn update_settings(
        &self,
        document_id: String,
        update: SettingsUpdate,
    ) -> Result<Settings, ScribeError> {
        let core_update: CoreSettingsUpdate = update.into();
        runtime()
            .block_on(self.inner.update_settings(&document_id, &core_update))
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Lists every language available for TTS narration.
    pub fn languages(&self) -> Result<Vec<Language>, ScribeError> {
        runtime()
            .block_on(self.inner.languages())
            .map(|langs| {
                langs
                    .into_iter()
                    .map(|l| Language {
                        display_name: l.0,
                        code: l.1,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists every dialect available for TTS narration, keyed by language code.
    pub fn dialects(&self) -> Result<HashMap<String, Vec<Dialect>>, ScribeError> {
        runtime()
            .block_on(self.inner.dialects())
            .map(|map| {
                map.into_iter()
                    .map(|(k, v)| {
                        let dialects = v
                            .into_iter()
                            .map(|d| Dialect {
                                display_name: d.0,
                                locale: d.1,
                            })
                            .collect();
                        (k, dialects)
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists every Braille translation table available for `brf` output.
    pub fn braille_tables(&self) -> Result<Vec<BrailleTable>, ScribeError> {
        runtime()
            .block_on(self.inner.braille_tables())
            .map(|tables| {
                tables
                    .into_iter()
                    .map(|t| BrailleTable {
                        display_name: t.0,
                        id: t.1,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists every TTS voice available, keyed by dialect locale.
    pub fn voices(&self) -> Result<HashMap<String, Vec<Voice>>, ScribeError> {
        runtime()
            .block_on(self.inner.voices())
            .map(|map| {
                map.into_iter()
                    .map(|(k, v)| {
                        let voices = v
                            .into_iter()
                            .map(|voice| Voice {
                                display_name: voice.0,
                                short_name: voice.1,
                                has_sample: voice.2,
                            })
                            .collect();
                        (k, voices)
                    })
                    .collect()
            })
            .map_err(Into::into)
    }
}

fn channel_closed_err() -> ScribeError {
    ScribeError::ChannelClosed
}

/// A live connection to a document's real-time channel, obtained from
/// [`FfiScribeClient::open_document_channel`]. This is the only way to
/// start converting a format other than the `html_stream` preview that
/// document creation already starts.
///
/// `UniFFI` objects are shared across the FFI boundary (`Arc<Self>`), so the
/// underlying [`DocumentChannel`] (whose methods need exclusive access to
/// its `WebSocket` connection) is guarded by a mutex rather than held by
/// value.
#[derive(uniffi::Object)]
pub struct FfiDocumentChannel {
    inner: Mutex<Option<DocumentChannel>>,
}

#[uniffi::export]
impl FfiDocumentChannel {
    /// Starts converting the joined document to `format`, using its
    /// current settings. Idempotent: if that format is already converting
    /// or complete, returns its existing output id. Returns immediately;
    /// progress arrives via subsequent [`Self::next_event`] calls.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::ConversionInProgress`] if a different
    /// non-preview conversion is already running,
    /// [`ScribeError::RateLimited`] if too many conversions were started
    /// too quickly, [`ScribeError::NeedsPurchase`] if the account is out
    /// of page credits, or [`ScribeError::ChannelClosed`] if the channel
    /// was already closed.
    pub fn start_conversion(&self, format: OutputFormat) -> Result<String, ScribeError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let channel = guard.as_mut().ok_or_else(channel_closed_err)?;
        runtime()
            .block_on(channel.start_conversion(format.into()))
            .map_err(Into::into)
    }

    /// Blocks until the next asynchronous event arrives on this channel.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::ChannelClosed`] if the channel is already
    /// closed, or closes before another event arrives.
    pub fn next_event(&self) -> Result<ChannelEvent, ScribeError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let channel = guard.as_mut().ok_or_else(channel_closed_err)?;
        runtime()
            .block_on(channel.next_event())
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Leaves the channel and closes the underlying connection. Safe to
    /// call more than once.
    ///
    /// # Errors
    ///
    /// Returns an error if sending the close frame fails.
    pub fn close(&self) -> Result<(), ScribeError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(channel) = guard.take() {
            runtime().block_on(channel.close()).map_err(Into::into)
        } else {
            Ok(())
        }
    }
}
