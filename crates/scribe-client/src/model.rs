use serde::Deserialize;

/// A document conversion output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
            let deserialized: Stage =
                serde_json::from_str(&format!("{expected:?}")).unwrap();
            assert_eq!(deserialized, stage);
        }
    }
}
