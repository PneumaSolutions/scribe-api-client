//! The value types that cross the FFI boundary, and their conversions to and
//! from the core crate's own types.
//!
//! These are `uniffi::Record`s and `uniffi::Enum`s: plain data, copied across
//! the boundary rather than held by handle.

use time::OffsetDateTime;

use scribe_client_core::{
    OutputFormat as CoreOutputFormat, SettingsUpdate as CoreSettingsUpdate, Stage as CoreStage,
};

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

/// The result of `list_outputs()`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OutputList {
    pub outputs: Vec<Output>,
    /// The document is a password-protected file that hasn't been unlocked
    /// yet. It has no outputs at all while this is true, so an empty
    /// `outputs` alone doesn't distinguish "locked" from "not started".
    pub is_password_needed: bool,
}

impl From<scribe_client_core::OutputList> for OutputList {
    fn from(l: scribe_client_core::OutputList) -> Self {
        OutputList {
            outputs: l.outputs.into_iter().map(Into::into).collect(),
            is_password_needed: l.is_password_needed,
        }
    }
}

/// The result of `list_documents()`, including the caller's page credit balance.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DocumentList {
    pub documents: Vec<DocumentSummary>,
    pub pages_remaining: Option<i64>,
}

/// One row from `list_documents()`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DocumentSummary {
    pub id: String,
    pub title: Option<String>,
    pub page_count: Option<i64>,
    /// ISO 8601 UTC timestamp of when the document was created.
    pub inserted_at: String,
    /// The document is a password-protected file that hasn't been unlocked
    /// yet, so it has no outputs and nothing can be downloaded from it.
    pub is_password_needed: bool,
    pub outputs: Vec<Output>,
}

impl From<scribe_client_core::DocumentSummary> for DocumentSummary {
    fn from(d: scribe_client_core::DocumentSummary) -> Self {
        DocumentSummary {
            id: d.id,
            title: d.title,
            page_count: d.page_count,
            inserted_at: d.inserted_at,
            is_password_needed: d.is_password_needed,
            outputs: d.outputs.into_iter().map(Into::into).collect(),
        }
    }
}

/// One row from `list_trashed_documents()`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TrashedDocument {
    pub id: String,
    pub title: Option<String>,
    pub page_count: Option<i64>,
    /// ISO 8601 UTC timestamp of when the document was created.
    pub inserted_at: String,
    /// ISO 8601 UTC timestamp of when the document was moved to the trash.
    pub trashed_at: String,
    /// ISO 8601 UTC timestamp of when the document will be permanently
    /// deleted if it isn't recovered first.
    pub permanently_delete_at: String,
}

impl From<scribe_client_core::TrashedDocument> for TrashedDocument {
    fn from(d: scribe_client_core::TrashedDocument) -> Self {
        TrashedDocument {
            id: d.id,
            title: d.title,
            page_count: d.page_count,
            inserted_at: d.inserted_at,
            trashed_at: d.trashed_at,
            permanently_delete_at: d.permanently_delete_at,
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

/// A user's push-notification preference. One flag today, on purpose —
/// device tokens are per-device, so this is user-scoped, not per-document.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NotificationSettings {
    pub push_notify_when_complete: bool,
}

impl From<scribe_client_core::NotificationSettings> for NotificationSettings {
    fn from(s: scribe_client_core::NotificationSettings) -> Self {
        NotificationSettings {
            push_notify_when_complete: s.push_notify_when_complete,
        }
    }
}

/// The authenticated user's name, email, phone number, and (if the account
/// belongs to a real organization rather than an individual) organization
/// name, for read-only display in the app's Account screen.
#[derive(Debug, Clone, uniffi::Record)]
pub struct AccountInfo {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub organization_name: Option<String>,
}

impl From<scribe_client_core::AccountInfo> for AccountInfo {
    fn from(a: scribe_client_core::AccountInfo) -> Self {
        AccountInfo {
            first_name: a.first_name,
            last_name: a.last_name,
            email: a.email,
            phone_number: a.phone_number,
            organization_name: a.organization_name,
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
