//! PyO3 bindings for `scribe-client`.
//!
//! The Python API is synchronous/blocking by design: each method runs the
//! underlying async call on a shared, lazily-started `tokio::Runtime` via
//! `Python::allow_threads` + `Runtime::block_on`, so the GIL is released
//! while the request is in flight but callers never see a coroutine.

use std::sync::OnceLock;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use url::Url;

use ::scribe_client::{
    AuthClient, DocumentSource, Output, OutputFormat, PkceChallenge, ScribeClient, ScribeError,
    TokenSet,
};

create_exception!(scribe_client, ScribeApiError, PyException);
create_exception!(scribe_client, InvalidGrantError, ScribeApiError);
create_exception!(scribe_client, NotFoundError, ScribeApiError);
create_exception!(scribe_client, ForbiddenError, ScribeApiError);
create_exception!(scribe_client, ConversionNotCompleteError, ScribeApiError);

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
#[pyclass(name = "TokenSet")]
#[derive(Clone)]
struct PyTokenSet {
    inner: TokenSet,
}

#[pymethods]
impl PyTokenSet {
    /// Constructs a token set directly — useful for tests, or for
    /// restoring a session previously persisted by the caller.
    #[new]
    #[pyo3(signature = (access_token, refresh_token=None, expires_at=None))]
    fn new(access_token: String, refresh_token: Option<String>, expires_at: Option<f64>) -> Self {
        PyTokenSet {
            inner: TokenSet {
                access_token,
                refresh_token,
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
    fn expires_at(&self) -> Option<f64> {
        self.inner.expires_at.map(|t| t.unix_timestamp() as f64)
    }
}

/// Drives the OAuth 2.0 Authorization Code + PKCE flow. Does **not** open a
/// browser or run a redirect listener — present `authorization_url()` to
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
        py.allow_threads(|| {
            runtime().block_on(self.inner.exchange_code(redirect_uri, code, verifier))
        })
        .map(|inner| PyTokenSet { inner })
        .map_err(to_py_err)
    }

    fn refresh(&self, py: Python<'_>, refresh_token: &str) -> PyResult<PyTokenSet> {
        py.allow_threads(|| runtime().block_on(self.inner.refresh(refresh_token)))
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

        py.allow_threads(|| runtime().block_on(self.inner.create_document(source)))
            .map(|doc| doc.document_id)
            .map_err(to_py_err)
    }

    /// Creates a document by having the server fetch it from `url`.
    /// Returns the new document's id.
    fn create_document_from_url(&self, py: Python<'_>, url: &str) -> PyResult<String> {
        let source = DocumentSource::Url(url.to_string());

        py.allow_threads(|| runtime().block_on(self.inner.create_document(source)))
            .map(|doc| doc.document_id)
            .map_err(to_py_err)
    }

    /// Lists every output (in-progress and completed) for a document.
    fn list_outputs(&self, py: Python<'_>, document_id: &str) -> PyResult<Vec<PyOutput>> {
        py.allow_threads(|| runtime().block_on(self.inner.list_outputs(document_id)))
            .map(|outputs| {
                outputs
                    .into_iter()
                    .map(|inner| PyOutput { inner })
                    .collect()
            })
            .map_err(to_py_err)
    }

    /// Downloads the bytes of a completed output. Raises
    /// `ConversionNotCompleteError` if that format hasn't finished
    /// converting yet.
    fn download_output<'py>(
        &self,
        py: Python<'py>,
        document_id: &str,
        format: &str,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let format = parse_format(format)?;

        let bytes = py
            .allow_threads(|| runtime().block_on(self.inner.download_output(document_id, format)))
            .map_err(to_py_err)?;

        Ok(PyBytes::new(py, &bytes))
    }
}

#[pymodule]
fn scribe_client(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPkceChallenge>()?;
    m.add_class::<PyTokenSet>()?;
    m.add_class::<PyAuthClient>()?;
    m.add_class::<PyOutput>()?;
    m.add_class::<PyScribeClient>()?;

    m.add("ScribeApiError", py.get_type::<ScribeApiError>())?;
    m.add("InvalidGrantError", py.get_type::<InvalidGrantError>())?;
    m.add("NotFoundError", py.get_type::<NotFoundError>())?;
    m.add("ForbiddenError", py.get_type::<ForbiddenError>())?;
    m.add(
        "ConversionNotCompleteError",
        py.get_type::<ConversionNotCompleteError>(),
    )?;

    Ok(())
}
