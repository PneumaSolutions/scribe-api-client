//! `PyO3` bindings for `scribe-client`.
//!
//! The Python API is synchronous/blocking by design: each method runs the
//! underlying async call on a shared, lazily-started `tokio::Runtime` via
//! `Python::detach` + `Runtime::block_on`, so the GIL is released
//! while the request is in flight but callers never see a coroutine.

use std::{collections::HashMap, sync::OnceLock};

use pyo3::{
    create_exception,
    exceptions::{PyException, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
};
use url::Url;

use scribe_client_core::{
    AuthClient, DocumentChannel, DocumentSource, DocumentSummary, Output, OutputFormat,
    PkceChallenge, ScribeClient, ScribeError, Settings, SettingsUpdate, TokenSet,
};

create_exception!(scribe_client, ScribeApiError, PyException);
create_exception!(scribe_client, InvalidGrantError, ScribeApiError);
create_exception!(scribe_client, NotFoundError, ScribeApiError);
create_exception!(scribe_client, ForbiddenError, ScribeApiError);
create_exception!(scribe_client, ConversionNotCompleteError, ScribeApiError);
create_exception!(scribe_client, ConversionInProgressError, ScribeApiError);
create_exception!(scribe_client, RateLimitedError, ScribeApiError);
create_exception!(scribe_client, NeedsPurchaseError, ScribeApiError);

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start the scribe-client tokio runtime")
    })
}

fn to_py_err(err: ScribeError) -> PyErr {
    match err {
        ScribeError::InvalidGrant(msg) => InvalidGrantError::new_err(msg),
        ScribeError::NotFound => NotFoundError::new_err("not found"),
        ScribeError::Forbidden => ForbiddenError::new_err("forbidden"),
        ScribeError::ConversionNotComplete => {
            ConversionNotCompleteError::new_err("document is not finished converting yet")
        }
        ScribeError::ConversionInProgress => ConversionInProgressError::new_err(
            "a conversion is already in progress for this document",
        ),
        ScribeError::RateLimited => RateLimitedError::new_err("rate limited, try again shortly"),
        ScribeError::NeedsPurchase { purchase_url } => NeedsPurchaseError::new_err(format!(
            "insufficient page credits; purchase more at {purchase_url}"
        )),
        other => ScribeApiError::new_err(other.to_string()),
    }
}

fn parse_url(raw: &str) -> PyResult<Url> {
    Url::parse(raw).map_err(|e| PyValueError::new_err(format!("invalid URL {raw:?}: {e}")))
}

fn parse_format(raw: &str) -> PyResult<OutputFormat> {
    OutputFormat::parse(raw)
        .ok_or_else(|| PyValueError::new_err(format!("unrecognized output format {raw:?}")))
}

/// A generated PKCE (RFC 7636) verifier/challenge pair.
#[pyclass(name = "PkceChallenge")]
struct PyPkceChallenge {
    inner: PkceChallenge,
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
struct PyTokenSet {
    inner: TokenSet,
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

/// Drives the OAuth 2.0 Authorization Code + PKCE flow. Does not open a
/// browser or run a redirect listener; present `authorization_url()` to
/// the user however fits your application and pass the resulting `code`
/// to `exchange_code()`.
#[pyclass(name = "AuthClient")]
struct PyAuthClient {
    inner: AuthClient,
}

#[pymethods]
impl PyAuthClient {
    #[new]
    fn new(base_url: &str, client_id: &str) -> PyResult<Self> {
        let base_url = parse_url(base_url)?;
        let http = reqwest::Client::new();
        Ok(PyAuthClient {
            inner: AuthClient::new(http, base_url, client_id.to_string()),
        })
    }

    fn authorization_url(&self, redirect_uri: &str, pkce: &PyPkceChallenge) -> String {
        self.inner
            .authorization_url(redirect_uri, &pkce.inner)
            .to_string()
    }

    fn exchange_code(
        &self,
        py: Python<'_>,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> PyResult<PyTokenSet> {
        py.detach(|| runtime().block_on(self.inner.exchange_code(redirect_uri, code, verifier)))
            .map(|inner| PyTokenSet { inner })
            .map_err(to_py_err)
    }

    fn refresh(&self, py: Python<'_>, refresh_token: &str) -> PyResult<PyTokenSet> {
        py.detach(|| runtime().block_on(self.inner.refresh(refresh_token)))
            .map(|inner| PyTokenSet { inner })
            .map_err(to_py_err)
    }
}

/// One row from `list_outputs()`.
#[pyclass(name = "Output")]
struct PyOutput {
    inner: Output,
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
struct PyDocumentSummary {
    inner: DocumentSummary,
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

/// A document's current conversion settings.
#[pyclass(name = "Settings")]
struct PySettings {
    inner: Settings,
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
type VoicesByDialect = HashMap<String, Vec<(String, String, bool)>>;

fn dict_to_settings_update(dict: &Bound<'_, PyDict>) -> PyResult<SettingsUpdate> {
    let value: serde_json::Value =
        pythonize::depythonize(dict).map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::from_value(value).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// A client for the document endpoints (`/api/documents*`). Holds a
/// [`PyTokenSet`] and refreshes it automatically as needed.
#[pyclass(name = "ScribeClient")]
struct PyScribeClient {
    inner: ScribeClient,
}

#[pymethods]
impl PyScribeClient {
    #[new]
    fn new(base_url: &str, client_id: &str, tokens: PyTokenSet) -> PyResult<Self> {
        let base_url = parse_url(base_url)?;
        let http = reqwest::Client::new();
        Ok(PyScribeClient {
            inner: ScribeClient::new(http, base_url, client_id.to_string(), tokens.inner),
        })
    }

    /// Creates a document by uploading file bytes directly. Returns the new
    /// document's id. The server automatically starts an `html_stream`
    /// conversion.
    fn create_document_from_file(
        &self,
        py: Python<'_>,
        file_name: &str,
        bytes: &[u8],
    ) -> PyResult<String> {
        let source = DocumentSource::File {
            file_name: file_name.to_string(),
            bytes: bytes.to_vec(),
        };
        py.detach(|| runtime().block_on(self.inner.create_document(source)))
            .map(|doc| doc.document_id)
            .map_err(to_py_err)
    }

    /// Creates a document by having the server fetch it from `url`.
    /// Returns the new document's id.
    fn create_document_from_url(&self, py: Python<'_>, url: &str) -> PyResult<String> {
        let source = DocumentSource::Url(url.to_string());
        py.detach(|| runtime().block_on(self.inner.create_document(source)))
            .map(|doc| doc.document_id)
            .map_err(to_py_err)
    }

    fn list_documents(&self, py: Python<'_>) -> PyResult<Vec<PyDocumentSummary>> {
        py.detach(|| runtime().block_on(self.inner.list_documents()))
            .map(|docs| {
                docs.into_iter()
                    .map(|inner| PyDocumentSummary { inner })
                    .collect()
            })
            .map_err(to_py_err)
    }

    fn delete_document(&self, py: Python<'_>, document_id: &str) -> PyResult<()> {
        py.detach(|| runtime().block_on(self.inner.delete_document(document_id)))
            .map_err(to_py_err)
    }

    /// Opens a real-time channel for `document_id`. This is the only way
    /// to start converting a format other than the `html_stream` preview
    /// that document creation already starts.
    fn open_document_channel(
        &self,
        py: Python<'_>,
        document_id: &str,
    ) -> PyResult<PyDocumentChannel> {
        py.detach(|| runtime().block_on(self.inner.open_document_channel(document_id)))
            .map(|inner| PyDocumentChannel { inner: Some(inner) })
            .map_err(to_py_err)
    }

    fn list_outputs(&self, py: Python<'_>, document_id: &str) -> PyResult<Vec<PyOutput>> {
        py.detach(|| runtime().block_on(self.inner.list_outputs(document_id)))
            .map(|outputs| {
                outputs
                    .into_iter()
                    .map(|inner| PyOutput { inner })
                    .collect()
            })
            .map_err(to_py_err)
    }

    fn download_output<'py>(
        &self,
        py: Python<'py>,
        document_id: &str,
        format: &str,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let format = parse_format(format)?;
        let bytes = py
            .detach(|| runtime().block_on(self.inner.download_output(document_id, format)))
            .map_err(to_py_err)?;
        Ok(PyBytes::new(py, &bytes))
    }

    fn get_settings(&self, py: Python<'_>, document_id: &str) -> PyResult<PySettings> {
        py.detach(|| runtime().block_on(self.inner.get_settings(document_id)))
            .map(|inner| PySettings { inner })
            .map_err(to_py_err)
    }

    fn update_settings(
        &self,
        py: Python<'_>,
        document_id: &str,
        settings: &Bound<'_, PyDict>,
    ) -> PyResult<PySettings> {
        let update = dict_to_settings_update(settings)?;
        py.detach(|| runtime().block_on(self.inner.update_settings(document_id, &update)))
            .map(|inner| PySettings { inner })
            .map_err(to_py_err)
    }

    /// Lists every language available for TTS narration, as `(name, code)` pairs.
    fn languages(&self, py: Python<'_>) -> PyResult<Vec<(String, String)>> {
        py.detach(|| runtime().block_on(self.inner.languages()))
            .map(|langs| langs.into_iter().map(|l| (l.0, l.1)).collect())
            .map_err(to_py_err)
    }

    /// Lists every dialect available for TTS narration, keyed by language
    /// code, each as a `(name, locale)` pair.
    fn dialects(&self, py: Python<'_>) -> PyResult<HashMap<String, Vec<(String, String)>>> {
        py.detach(|| runtime().block_on(self.inner.dialects()))
            .map(|map| {
                map.into_iter()
                    .map(|(k, v)| (k, v.into_iter().map(|d| (d.0, d.1)).collect()))
                    .collect()
            })
            .map_err(to_py_err)
    }

    /// Lists every Braille translation table available for `brf` output, as
    /// `(name, table_id)` pairs.
    fn braille_tables(&self, py: Python<'_>) -> PyResult<Vec<(String, String)>> {
        py.detach(|| runtime().block_on(self.inner.braille_tables()))
            .map(|tables| tables.into_iter().map(|t| (t.0, t.1)).collect())
            .map_err(to_py_err)
    }

    /// Lists every TTS voice available, keyed by dialect locale, each as a
    /// `(name, voice_short_name, has_sample)` triple.
    fn voices(&self, py: Python<'_>) -> PyResult<VoicesByDialect> {
        py.detach(|| runtime().block_on(self.inner.voices()))
            .map(|map| {
                map.into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            v.into_iter()
                                .map(|voice| (voice.0, voice.1, voice.2))
                                .collect(),
                        )
                    })
                    .collect()
            })
            .map_err(to_py_err)
    }
}

fn channel_closed_err() -> PyErr {
    ScribeApiError::new_err("channel is closed")
}

/// A live connection to a document's real-time channel, obtained from
/// `ScribeClient.open_document_channel()`. This is the only way to start
/// converting a format other than the `html_stream` preview that document
/// creation already starts.
#[pyclass(name = "DocumentChannel")]
struct PyDocumentChannel {
    inner: Option<DocumentChannel>,
}

#[pymethods]
impl PyDocumentChannel {
    /// Starts converting the joined document to `format`, using its
    /// current settings. Idempotent: if that format is already converting
    /// or complete, returns its existing output id. Returns immediately;
    /// progress arrives via subsequent `next_event()` calls.
    fn start_conversion(&mut self, py: Python<'_>, format: &str) -> PyResult<String> {
        let format = parse_format(format)?;
        let channel = self.inner.as_mut().ok_or_else(channel_closed_err)?;
        py.detach(|| runtime().block_on(channel.start_conversion(format)))
            .map_err(to_py_err)
    }

    /// Blocks until the next asynchronous event arrives on this channel
    /// and returns it as a dict. `event["type"]` is one of `"status"`,
    /// `"chunk"`, `"conversion_complete"`, or `"error"`; the remaining
    /// keys depend on the type (see the module documentation).
    fn next_event<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let channel = self.inner.as_mut().ok_or_else(channel_closed_err)?;
        let event = py
            .detach(|| runtime().block_on(channel.next_event()))
            .map_err(to_py_err)?;
        pythonize::pythonize(py, &event).map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Leaves the channel and closes the underlying connection. Safe to call more than once.
    fn close(&mut self, py: Python<'_>) -> PyResult<()> {
        if let Some(channel) = self.inner.take() {
            py.detach(|| runtime().block_on(channel.close()))
                .map_err(to_py_err)?;
        }
        Ok(())
    }
}

#[pymodule]
fn scribe_client(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPkceChallenge>()?;
    m.add_class::<PyTokenSet>()?;
    m.add_class::<PyAuthClient>()?;
    m.add_class::<PyOutput>()?;
    m.add_class::<PyDocumentSummary>()?;
    m.add_class::<PySettings>()?;
    m.add_class::<PyScribeClient>()?;
    m.add_class::<PyDocumentChannel>()?;
    m.add("ScribeApiError", py.get_type::<ScribeApiError>())?;
    m.add("InvalidGrantError", py.get_type::<InvalidGrantError>())?;
    m.add("NotFoundError", py.get_type::<NotFoundError>())?;
    m.add("ForbiddenError", py.get_type::<ForbiddenError>())?;
    m.add(
        "ConversionNotCompleteError",
        py.get_type::<ConversionNotCompleteError>(),
    )?;
    m.add(
        "ConversionInProgressError",
        py.get_type::<ConversionInProgressError>(),
    )?;
    m.add("RateLimitedError", py.get_type::<RateLimitedError>())?;
    m.add("NeedsPurchaseError", py.get_type::<NeedsPurchaseError>())?;
    Ok(())
}
