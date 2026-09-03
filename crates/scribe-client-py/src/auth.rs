//! The OAuth 2.0 Authorization Code + PKCE flow.

use pyo3::prelude::*;

use scribe_client_core::AuthClient;

use crate::{
    error::to_py_err,
    model::{PyPkceChallenge, PyTokenSet},
    parse_url, runtime,
};

/// Drives the OAuth 2.0 Authorization Code + PKCE flow. Does not open a
/// browser or run a redirect listener; present `authorization_url()` to
/// the user however fits your application and pass the resulting `code`
/// to `exchange_code()`.
#[pyclass(name = "AuthClient")]
pub(crate) struct PyAuthClient {
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
