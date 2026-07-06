use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::multipart;
use serde::Deserialize;
use tokio::sync::Mutex;
use url::Url;

use crate::{
    auth::{AuthClient, TokenSet},
    error::ScribeError,
    model::{
        BrailleTable, BrailleTablesResponse, CreatedDocument, Dialect, DialectsResponse,
        Language, LanguagesResponse, Output, OutputFormat, OutputListResponse, Settings,
        SettingsUpdate, Voice, VoicesResponse,
    },
};

/// How early to proactively refresh a token before it actually expires.
const REFRESH_SKEW: Duration = Duration::from_secs(30);

/// What to create a document from.
pub enum DocumentSource {
    /// Upload file bytes directly.
    File { file_name: String, bytes: Vec<u8> },
    /// Have the server fetch the document from a URL.
    Url(String),
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: String,
}

/// A client for the document-conversion endpoints
/// (`/api/documents*`). Holds a [`TokenSet`] and refreshes it
/// automatically as needed, so construct one [`AuthClient`]/[`ScribeClient`]
/// pair per authenticated user session and reuse it across requests.
pub struct ScribeClient {
    http: reqwest::Client,
    base_url: Url,
    auth: AuthClient,
    tokens: Arc<Mutex<TokenSet>>,
}

impl ScribeClient {
    pub fn new(
        http: reqwest::Client,
        base_url: Url,
        client_id: impl Into<String>,
        tokens: TokenSet,
    ) -> Self {
        let auth = AuthClient::new(http.clone(), base_url.clone(), client_id);

        ScribeClient {
            http,
            base_url,
            auth,
            tokens: Arc::new(Mutex::new(tokens)),
        }
    }

    /// The current access token, refreshing first if it's missing or about
    /// to expire and a refresh token is available.
    async fn access_token(&self) -> Result<String, ScribeError> {
        let mut tokens = self.tokens.lock().await;

        if tokens.needs_refresh(REFRESH_SKEW) {
            if let Some(refresh_token) = tokens.refresh_token.clone() {
                *tokens = self.auth.refresh(&refresh_token).await?;
            }
        }

        Ok(tokens.access_token.clone())
    }

    /// Force-refreshes and replaces the current token set. Also used after
    /// a request unexpectedly comes back `401` even though our local
    /// expiry tracking thought the token was still good.
    async fn force_refresh(&self) -> Result<String, ScribeError> {
        let mut tokens = self.tokens.lock().await;
        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or_else(|| ScribeError::InvalidGrant("no refresh token available".into()))?;

        *tokens = self.auth.refresh(&refresh_token).await?;
        Ok(tokens.access_token.clone())
    }

    /// Creates a document from `source`.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] if the request
    /// fails or the server rejects it.
    pub async fn create_document(
        &self,
        source: DocumentSource,
    ) -> Result<CreatedDocument, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/documents");

        self.with_auth_retry(|token| {
            let form = match &source {
                DocumentSource::File { file_name, bytes } => multipart::Form::new().part(
                    "document[file]",
                    multipart::Part::bytes(bytes.clone()).file_name(file_name.clone()),
                ),
                DocumentSource::Url(source_url) => {
                    multipart::Form::new().text("document[url]", source_url.clone())
                }
            };

            self.http
                .post(url.clone())
                .bearer_auth(token)
                .multipart(form)
        })
        .await
    }

    /// Lists every output (in-progress and completed) for a document.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures.
    pub async fn list_outputs(&self, document_id: &str) -> Result<Vec<Output>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/outputs"));

        let response: OutputListResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;

        Ok(response.outputs)
    }

    /// Downloads the bytes of a completed output.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::ConversionNotComplete`] if that format hasn't
    /// finished converting yet, [`ScribeError::NotFound`]/[`ScribeError::Forbidden`]
    /// if the document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures.
    pub async fn download_output(
        &self,
        document_id: &str,
        format: OutputFormat,
    ) -> Result<Vec<u8>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "/api/documents/{document_id}/outputs/{}/download",
            format.as_str()
        ));

        let token = self.access_token().await?;
        let response = self
            .http
            .get(url)
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status_or_json_error()
            .await?;

        Ok(response.bytes().await?.to_vec())
    }

    /// Fetches a document's current conversion settings.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures.
    pub async fn get_settings(&self, document_id: &str) -> Result<Settings, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/settings"));

        self.with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await
    }

    /// Applies a partial update to a document's conversion settings.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures
    /// (including validation errors on the settings themselves).
    pub async fn update_settings(
        &self,
        document_id: &str,
        update: &SettingsUpdate,
    ) -> Result<Settings, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/settings"));
        let body = serde_json::json!({ "settings": update });

        self.with_auth_retry(|token| {
            self.http.patch(url.clone()).bearer_auth(token).json(&body)
        })
        .await
    }

    /// Starts converting a document to `format`, using its current
    /// settings. Idempotent: calling this again for a format that's already
    /// converting or complete returns that existing output.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures
    /// (including an unsupported `format`).
    pub async fn create_output(
        &self,
        document_id: &str,
        format: OutputFormat,
    ) -> Result<Output, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/outputs"));
        let body = serde_json::json!({ "format": format.as_str() });

        self.with_auth_retry(|token| {
            self.http.post(url.clone()).bearer_auth(token).json(&body)
        })
        .await
    }

    /// Lists every language available for TTS narration.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn languages(&self) -> Result<Vec<Language>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/settings/languages");

        let response: LanguagesResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;

        Ok(response.languages)
    }

    /// Lists every dialect available for TTS narration, keyed by language code.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn dialects(&self) -> Result<HashMap<String, Vec<Dialect>>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/settings/dialects");

        let response: DialectsResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;

        Ok(response.dialects)
    }

    /// Lists every Braille translation table available for `brf` output.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn braille_tables(&self) -> Result<Vec<BrailleTable>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/settings/braille_tables");

        let response: BrailleTablesResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;

        Ok(response.braille_tables)
    }

    /// Lists every TTS voice available, keyed by dialect locale.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn voices(&self) -> Result<HashMap<String, Vec<Voice>>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/settings/voices");

        let response: VoicesResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;

        Ok(response.voices)
    }

    /// Sends a request built by `build`, retrying once with a
    /// force-refreshed token if the server returns `401`.
    async fn with_auth_retry<T, F>(&self, build: F) -> Result<T, ScribeError>
    where
        T: serde::de::DeserializeOwned,
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        let token = self.access_token().await?;
        let response = build(&token).send().await?;

        let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let token = self.force_refresh().await?;
            build(&token).send().await?
        } else {
            response
        };

        response
            .error_for_status_or_json_error()
            .await?
            .json()
            .await
            .map_err(Into::into)
    }
}

/// Small helper trait so response-status handling reads the same way at
/// every call site: map non-2xx responses to [`ScribeError`], parsing the
/// server's `{"error": "..."}` body when present.
trait ResponseExt {
    async fn error_for_status_or_json_error(self) -> Result<reqwest::Response, ScribeError>;
}

impl ResponseExt for reqwest::Response {
    async fn error_for_status_or_json_error(self) -> Result<reqwest::Response, ScribeError> {
        let status = self.status();

        if status.is_success() {
            return Ok(self);
        }

        let text = self.text().await.unwrap_or_default();
        let error = serde_json::from_str::<ApiErrorResponse>(&text)
            .map(|e| e.error)
            .unwrap_or(text);

        Err(match (status.as_u16(), error.as_str()) {
            (404, _) => ScribeError::NotFound,
            (403, _) => ScribeError::Forbidden,
            (409, "conversion_not_complete") => ScribeError::ConversionNotComplete,
            (status, error) => ScribeError::Api {
                status,
                error: error.to_string(),
            },
        })
    }
}
