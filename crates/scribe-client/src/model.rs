use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A document conversion output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

impl OutputFormat {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputFormat::Html => "html",
            OutputFormat::Pdf => "pdf",
            OutputFormat::Epub => "epub",
            OutputFormat::Daisy => "daisy",
            OutputFormat::Docx => "docx",
            OutputFormat::Brf => "brf",
            OutputFormat::Mp3 => "mp3",
            OutputFormat::OfflineHtml => "offline_html",
            OutputFormat::Mobi => "mobi",
            OutputFormat::HtmlStream => "html_stream",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "html" => OutputFormat::Html,
            "pdf" => OutputFormat::Pdf,
            "epub" => OutputFormat::Epub,
            "daisy" => OutputFormat::Daisy,
            "docx" => OutputFormat::Docx,
            "brf" => OutputFormat::Brf,
            "mp3" => OutputFormat::Mp3,
            "offline_html" => OutputFormat::OfflineHtml,
            "mobi" => OutputFormat::Mobi,
            "html_stream" => OutputFormat::HtmlStream,
            _ => return None,
        })
    }
}

/// The current stage of a document conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Queue,
    Start,
    Convert,
    AddImageDescriptions,
    Complete,
}

impl Stage {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        matches!(self, Stage::Complete)
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Queue => "queue",
            Stage::Start => "start",
            Stage::Convert => "convert",
            Stage::AddImageDescriptions => "add_image_descriptions",
            Stage::Complete => "complete",
        }
    }
}

/// One row from `GET /api/documents/:id/outputs`.
#[derive(Debug, Clone, Deserialize)]
pub struct Output {
    pub format: OutputFormat,
    pub stage: Stage,
    pub progress: f64,
    pub estimated_time_remaining: Option<i64>,
    pub is_preview: bool,
}

/// Response body of `POST /api/documents`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatedDocument {
    pub document_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputListResponse {
    pub outputs: Vec<Output>,
}

/// One row from `GET /api/documents`.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentSummary {
    pub id: String,
    pub title: String,
    pub page_count: Option<i64>,
    /// ISO 8601 UTC timestamp of when the document was created.
    pub inserted_at: String,
    pub outputs: Vec<Output>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocumentListResponse {
    pub documents: Vec<DocumentSummary>,
}

/// A document's current conversion settings
/// (`GET`/`PATCH /api/documents/:id/settings`).
#[allow(clippy::struct_excessive_bools)] // mirrors the server's flat settings shape
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub language: Option<String>,
    pub dialects: serde_json::Value,
    pub voices: serde_json::Value,
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

/// A partial update to a document's conversion settings
/// (`PATCH /api/documents/:id/settings`). Only the fields set to `Some`
/// are sent, so unset fields are left unchanged server-side.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialects: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voices: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_gender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub braille_translation_table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub braille_cells_per_line: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub braille_split_into_pages: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub braille_lines_per_page: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large_print: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_image_descriptions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_when_complete: Option<bool>,
}

/// A `(display_name, language_code)` pair from `GET /api/settings/languages`.
#[derive(Debug, Clone, Deserialize)]
pub struct Language(pub String, pub String);

/// A `(display_name, locale)` pair from `GET /api/settings/dialects`.
#[derive(Debug, Clone, Deserialize)]
pub struct Dialect(pub String, pub String);

/// A `(display_name, table_id)` pair from `GET /api/settings/braille_tables`.
#[derive(Debug, Clone, Deserialize)]
pub struct BrailleTable(pub String, pub String);

/// A `(display_name, voice_short_name, has_sample)` triple from
/// `GET /api/settings/voices`.
#[derive(Debug, Clone, Deserialize)]
pub struct Voice(pub String, pub String, pub bool);

#[derive(Debug, Deserialize)]
pub(crate) struct LanguagesResponse {
    pub languages: Vec<Language>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DialectsResponse {
    pub dialects: HashMap<String, Vec<Dialect>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BrailleTablesResponse {
    pub braille_tables: Vec<BrailleTable>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VoicesResponse {
    pub voices: HashMap<String, Vec<Voice>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_as_str_and_parse_round_trip() {
        let all = [
            OutputFormat::Html,
            OutputFormat::Pdf,
            OutputFormat::Epub,
            OutputFormat::Daisy,
            OutputFormat::Docx,
            OutputFormat::Brf,
            OutputFormat::Mp3,
            OutputFormat::OfflineHtml,
            OutputFormat::Mobi,
            OutputFormat::HtmlStream,
        ];

        for format in all {
            assert_eq!(OutputFormat::parse(format.as_str()), Some(format));
        }

        assert_eq!(OutputFormat::parse("not_a_format"), None);
    }

    #[test]
    fn only_the_complete_stage_reports_is_complete() {
        let all = [
            Stage::Queue,
            Stage::Start,
            Stage::Convert,
            Stage::AddImageDescriptions,
            Stage::Complete,
        ];

        for stage in all {
            assert_eq!(stage.is_complete(), stage == Stage::Complete);
        }
    }

    #[test]
    fn stage_as_str_round_trips_through_json() {
        let all = [
            (Stage::Queue, "queue"),
            (Stage::Start, "start"),
            (Stage::Convert, "convert"),
            (Stage::AddImageDescriptions, "add_image_descriptions"),
            (Stage::Complete, "complete"),
        ];

        for (stage, expected) in all {
            assert_eq!(stage.as_str(), expected);
            let deserialized: Stage = serde_json::from_str(&format!("{expected:?}")).unwrap();
            assert_eq!(deserialized, stage);
        }
    }
}
