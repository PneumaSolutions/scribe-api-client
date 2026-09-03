//! The streaming conversion channel.

use pyo3::{exceptions::PyValueError, prelude::*};

use scribe_client_core::DocumentChannel;

use crate::{error::to_py_err, error::ScribeApiError, model::parse_format, runtime};

pub(crate) fn channel_closed_err() -> PyErr {
    ScribeApiError::new_err("channel is closed")
}

/// A live connection to a document's real-time channel, obtained from
/// `ScribeClient.open_document_channel()`. This is the only way to start
/// converting a format other than the `html_stream` preview that document
/// creation already starts.
#[pyclass(name = "DocumentChannel")]
pub(crate) struct PyDocumentChannel {
    inner: Option<DocumentChannel>,
}

impl PyDocumentChannel {
    /// Wraps a channel the client has just opened. Not a `#[pymethods]`
    /// constructor: a channel is only ever handed out by
    /// `ScribeClient.open_document_channel`, which is what keeps the field
    /// private to this module.
    pub(crate) fn new(inner: DocumentChannel) -> Self {
        PyDocumentChannel { inner: Some(inner) }
    }
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
