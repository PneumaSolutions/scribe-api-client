//! The document-conversion client.

use std::collections::HashMap;

use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};

use scribe_client_core::{DocumentSource, ScribeClient};

use crate::{
    channel::PyDocumentChannel,
    error::to_py_err,
    model::{
        dict_to_settings_update, parse_format, PyDocumentList, PyOutputList, PySettings,
        PyTokenSet, PyTrashedDocument, VoicesByDialect,
    },
    parse_url, runtime,
};

/// A client for the document endpoints (`/api/documents*`). Holds a
/// [`PyTokenSet`] and refreshes it automatically as needed.
#[pyclass(name = "ScribeClient")]
pub(crate) struct PyScribeClient {
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
    ///
    /// `password` unlocks a password-protected source file. Supply it when
    /// the file is known to be protected; otherwise leave it unset and
    /// handle the `password_required` channel event.
    #[pyo3(signature = (file_name, bytes, password=None))]
    fn create_document_from_file(
        &self,
        py: Python<'_>,
        file_name: &str,
        bytes: &[u8],
        password: Option<&str>,
    ) -> PyResult<String> {
        let source = DocumentSource::File {
            file_name: file_name.to_string(),
            bytes: bytes.to_vec(),
        };
        py.detach(|| runtime().block_on(self.inner.create_document(source, password)))
            .map(|doc| doc.document_id)
            .map_err(to_py_err)
    }

    /// Creates a document by having the server fetch it from `url`.
    /// Returns the new document's id.
    #[pyo3(signature = (url, password=None))]
    fn create_document_from_url(
        &self,
        py: Python<'_>,
        url: &str,
        password: Option<&str>,
    ) -> PyResult<String> {
        let source = DocumentSource::Url(url.to_string());
        py.detach(|| runtime().block_on(self.inner.create_document(source, password)))
            .map(|doc| doc.document_id)
            .map_err(to_py_err)
    }

    fn list_documents(&self, py: Python<'_>) -> PyResult<PyDocumentList> {
        py.detach(|| runtime().block_on(self.inner.list_documents()))
            .map(|inner| PyDocumentList { inner })
            .map_err(to_py_err)
    }

    /// Moves a document to the trash. It's permanently deleted 7 days
    /// later, or sooner per the owner's org retention policy, unless
    /// recovered first with `recover_document`.
    fn trash_document(&self, py: Python<'_>, document_id: &str) -> PyResult<()> {
        py.detach(|| runtime().block_on(self.inner.trash_document(document_id)))
            .map_err(to_py_err)
    }

    /// Permanently deletes a document and all of its outputs. The document
    /// must already be in the trash (see `trash_document`).
    fn delete_document_permanently(&self, py: Python<'_>, document_id: &str) -> PyResult<()> {
        py.detach(|| runtime().block_on(self.inner.delete_document_permanently(document_id)))
            .map_err(to_py_err)
    }

    /// Restores a trashed document, clearing its trash state.
    fn recover_document(&self, py: Python<'_>, document_id: &str) -> PyResult<()> {
        py.detach(|| runtime().block_on(self.inner.recover_document(document_id)))
            .map_err(to_py_err)
    }

    /// Lists the caller's trashed documents, most recently trashed first.
    fn list_trashed_documents(&self, py: Python<'_>) -> PyResult<Vec<PyTrashedDocument>> {
        py.detach(|| runtime().block_on(self.inner.list_trashed_documents()))
            .map(|documents| {
                documents
                    .into_iter()
                    .map(|inner| PyTrashedDocument { inner })
                    .collect()
            })
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
            .map(PyDocumentChannel::new)
            .map_err(to_py_err)
    }

    fn list_outputs(&self, py: Python<'_>, document_id: &str) -> PyResult<PyOutputList> {
        py.detach(|| runtime().block_on(self.inner.list_outputs(document_id)))
            .map(|inner| PyOutputList { inner })
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
