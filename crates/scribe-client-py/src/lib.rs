//! `PyO3` bindings for `scribe-client`.
//!
//! The Python API is synchronous/blocking by design: each method runs the
//! underlying async call on a shared, lazily-started `tokio::Runtime` via
//! `Python::detach` + `Runtime::block_on`, so the GIL is released
//! while the request is in flight but callers never see a coroutine.

use std::sync::OnceLock;

use pyo3::{exceptions::PyValueError, prelude::*};
use url::Url;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start the scribe-client tokio runtime")
    })
}

fn parse_url(raw: &str) -> PyResult<Url> {
    Url::parse(raw).map_err(|e| PyValueError::new_err(format!("invalid URL {raw:?}: {e}")))
}

mod auth;
mod channel;
mod client;
mod error;
mod model;

use auth::PyAuthClient;
use channel::PyDocumentChannel;
use client::PyScribeClient;
use error::{
    ConversionInProgressError, ConversionNotCompleteError, ForbiddenError, InvalidGrantError,
    NeedsPurchaseError, NotFoundError, NotTrashedError, RateLimitedError, ScribeApiError,
};
use model::{
    PyDocumentList, PyDocumentSummary, PyOutput, PyPkceChallenge, PySettings, PyTokenSet,
    PyTrashedDocument,
};

#[pymodule]
fn scribe_client(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPkceChallenge>()?;
    m.add_class::<PyTokenSet>()?;
    m.add_class::<PyAuthClient>()?;
    m.add_class::<PyOutput>()?;
    m.add_class::<PyDocumentSummary>()?;
    m.add_class::<PyDocumentList>()?;
    m.add_class::<PyTrashedDocument>()?;
    m.add_class::<PySettings>()?;
    m.add_class::<PyScribeClient>()?;
    m.add_class::<PyDocumentChannel>()?;
    m.add("ScribeApiError", py.get_type::<ScribeApiError>())?;
    m.add("InvalidGrantError", py.get_type::<InvalidGrantError>())?;
    m.add("NotFoundError", py.get_type::<NotFoundError>())?;
    m.add("ForbiddenError", py.get_type::<ForbiddenError>())?;
    m.add("NotTrashedError", py.get_type::<NotTrashedError>())?;
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
