//! The Python classes carrying plain data across the boundary, and the
//! conversions between them and the core crate's own types.

use std::collections::HashMap;

use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};

use scribe_client_core::{
    DocumentList, DocumentSummary, Output, OutputFormat, PkceChallenge, Settings, SettingsUpdate,
    TokenSet, TrashedDocument,
};

pub(crate) fn parse_format(raw: &str) -> PyResult<OutputFormat> {
    OutputFormat::parse(raw)
        .ok_or_else(|| PyValueError::new_err(format!("unrecognized output format {raw:?}")))
}

/// A generated PKCE (RFC 7636) verifier/challenge pair.
#[pyclass(name = "PkceChallenge")]
pub(crate) struct PyPkceChallenge {
    pub(crate) inner: PkceChallenge,
}

#[pymethods]
impl PyPkceChallenge {
    #[new]
    fn new() -> Self {
        PyPkceChallenge {
            inner: PkceChallenge::generate(),
        }
    }

    #[getter]
    fn verifier(&self) -> &str {
        self.inner.verifier()
    }

    #[getter]
    fn challenge(&self) -> &str {
        self.inner.challenge()
    }

    fn __repr__(&self) -> String {
        format!("PkceChallenge(challenge={:?})", self.inner.challenge())
    }
}

/// An access/refresh token pair returned by `POST /oauth/token`.
#[pyclass(name = "TokenSet", from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTokenSet {
    pub(crate) inner: TokenSet,
}

#[pymethods]
impl PyTokenSet {
    /// Constructs a token set directly, useful for tests or for restoring
    /// a session previously persisted by the caller.
    #[new]
    #[pyo3(signature = (access_token, refresh_token=None, expires_at=None))]
    fn new(access_token: String, refresh_token: Option<String>, expires_at: Option<f64>) -> Self {
        PyTokenSet {
            inner: TokenSet {
                access_token,
                refresh_token,
                // Python's time.time() convention is float seconds since the
                // epoch; sub-second precision isn't meaningful for token expiry.
                #[allow(clippy::cast_possible_truncation)]
                expires_at: expires_at.map(|secs| {
                    time::OffsetDateTime::from_unix_timestamp(secs as i64)
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
                }),
            },
        }
    }

    #[getter]
    fn access_token(&self) -> &str {
        &self.inner.access_token
    }

    #[getter]
    fn refresh_token(&self) -> Option<&str> {
        self.inner.refresh_token.as_deref()
    }

    /// Unix timestamp (seconds), or `None` if the server didn't report an
    /// expiry.
    #[getter]
    #[allow(clippy::cast_precision_loss)]
    fn expires_at(&self) -> Option<f64> {
        self.inner.expires_at.map(|t| t.unix_timestamp() as f64)
    }
}

/// One row from `list_outputs()`.
#[pyclass(name = "Output")]
pub(crate) struct PyOutput {
    pub(crate) inner: Output,
}

#[pymethods]
impl PyOutput {
    #[getter]
    fn format(&self) -> &'static str {
        self.inner.format.as_str()
    }

    #[getter]
    fn stage(&self) -> &'static str {
        self.inner.stage.as_str()
    }

    #[getter]
    fn progress(&self) -> f64 {
        self.inner.progress
    }

    #[getter]
    fn estimated_time_remaining(&self) -> Option<i64> {
        self.inner.estimated_time_remaining
    }

    #[getter]
    fn is_preview(&self) -> bool {
        self.inner.is_preview
    }

    fn __repr__(&self) -> String {
        format!(
            "Output(format={:?}, stage={:?}, progress={})",
            self.inner.format.as_str(),
            self.inner.stage.as_str(),
            self.inner.progress
        )
    }
}

/// One row from `list_documents()`.
#[pyclass(name = "DocumentSummary")]
pub(crate) struct PyDocumentSummary {
    pub(crate) inner: DocumentSummary,
}

#[pymethods]
impl PyDocumentSummary {
    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn title(&self) -> Option<&str> {
        self.inner.title.as_deref()
    }

    #[getter]
    fn page_count(&self) -> Option<i64> {
        self.inner.page_count
    }

    /// ISO 8601 UTC timestamp of when the document was created.
    #[getter]
    fn inserted_at(&self) -> &str {
        &self.inner.inserted_at
    }

    #[getter]
    fn outputs(&self) -> Vec<PyOutput> {
        self.inner
            .outputs
            .iter()
            .cloned()
            .map(|inner| PyOutput { inner })
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "DocumentSummary(id={:?}, title={:?})",
            self.inner.id, self.inner.title
        )
    }
}

/// The result of `list_documents()`, including the caller's page credit balance.
#[pyclass(name = "DocumentList")]
pub(crate) struct PyDocumentList {
    pub(crate) inner: DocumentList,
}

#[pymethods]
impl PyDocumentList {
    #[getter]
    fn documents(&self) -> Vec<PyDocumentSummary> {
        self.inner
            .documents
            .iter()
            .cloned()
            .map(|inner| PyDocumentSummary { inner })
            .collect()
    }

    #[getter]
    fn pages_remaining(&self) -> Option<i64> {
        self.inner.pages_remaining
    }

    fn __repr__(&self) -> String {
        format!(
            "DocumentList(documents={} items, pages_remaining={:?})",
            self.inner.documents.len(),
            self.inner.pages_remaining
        )
    }
}

/// One row from `list_trashed_documents()`.
#[pyclass(name = "TrashedDocument")]
pub(crate) struct PyTrashedDocument {
    pub(crate) inner: TrashedDocument,
}

#[pymethods]
impl PyTrashedDocument {
    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn title(&self) -> Option<&str> {
        self.inner.title.as_deref()
    }

    #[getter]
    fn page_count(&self) -> Option<i64> {
        self.inner.page_count
    }

    /// ISO 8601 UTC timestamp of when the document was created.
    #[getter]
    fn inserted_at(&self) -> &str {
        &self.inner.inserted_at
    }

    /// ISO 8601 UTC timestamp of when the document was moved to the trash.
    #[getter]
    fn trashed_at(&self) -> &str {
        &self.inner.trashed_at
    }

    /// ISO 8601 UTC timestamp of when the document will be permanently
    /// deleted if it isn't recovered first.
    #[getter]
    fn permanently_delete_at(&self) -> &str {
        &self.inner.permanently_delete_at
    }

    fn __repr__(&self) -> String {
        format!(
            "TrashedDocument(id={:?}, title={:?})",
            self.inner.id, self.inner.title
        )
    }
}

/// A document's current conversion settings.
#[pyclass(name = "Settings")]
pub(crate) struct PySettings {
    pub(crate) inner: Settings,
}

#[pymethods]
impl PySettings {
    #[getter]
    fn language(&self) -> Option<&str> {
        self.inner.language.as_deref()
    }

    #[getter]
    fn dialects<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize::pythonize(py, &self.inner.dialects)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    #[getter]
    fn voices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize::pythonize(py, &self.inner.voices)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    #[getter]
    fn tts_gender(&self) -> Option<&str> {
        self.inner.tts_gender.as_deref()
    }

    #[getter]
    fn tts_rate(&self) -> f64 {
        self.inner.tts_rate
    }

    #[getter]
    fn braille_translation_table(&self) -> &str {
        &self.inner.braille_translation_table
    }

    #[getter]
    fn braille_cells_per_line(&self) -> i64 {
        self.inner.braille_cells_per_line
    }

    #[getter]
    fn braille_split_into_pages(&self) -> bool {
        self.inner.braille_split_into_pages
    }

    #[getter]
    fn braille_lines_per_page(&self) -> i64 {
        self.inner.braille_lines_per_page
    }

    #[getter]
    fn large_print(&self) -> bool {
        self.inner.large_print
    }

    #[getter]
    fn add_image_descriptions(&self) -> bool {
        self.inner.add_image_descriptions
    }

    #[getter]
    fn math(&self) -> bool {
        self.inner.math
    }

    #[getter]
    fn notify_when_complete(&self) -> bool {
        self.inner.notify_when_complete
    }
}

/// `(name, voice_short_name, has_sample)` triples, keyed by dialect locale.
pub(crate) type VoicesByDialect = HashMap<String, Vec<(String, String, bool)>>;

pub(crate) fn dict_to_settings_update(dict: &Bound<'_, PyDict>) -> PyResult<SettingsUpdate> {
    let value: serde_json::Value =
        pythonize::depythonize(dict).map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::from_value(value).map_err(|e| PyValueError::new_err(e.to_string()))
}
