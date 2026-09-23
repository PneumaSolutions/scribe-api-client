use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::multipart;
use serde::Deserialize;
use tokio::sync::Mutex;
use url::Url;

use crate::{
    auth::{AuthClient, TokenSet},
    channel::DocumentChannel,
    error::ScribeError,
    model::{
        AccountInfo, BrailleTable, BrailleTablesResponse, CreatedDocument, Dialect,
        DialectsResponse, DocumentList, DocumentListResponse, DocumentSummary, Download, Language,
        LanguagesResponse, NotificationSettings, OutputFormat, OutputList, OutputListResponse,
        Settings, SettingsUpdate, TrashedDocument, TrashedDocumentListResponse, Voice,
        VoicesResponse,
    },
};

/// How early to proactively refresh a token before it actually expires.
/// 5 minutes gives enough headroom for slow mobile networks and cold starts.
const REFRESH_SKEW: Duration = Duration::from_secs(300);

/// What to create a document from.
pub enum DocumentSource {
    /// Upload file bytes directly.
    File { file_name: String, bytes: Vec<u8> },
    /// Have the server fetch the document from a URL.
    Url(String),
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    #[serde(default)]
    message: Option<String>,
}

/// A client for the document-conversion endpoints
/// (`/api/documents*`). Holds a [`TokenSet`] and refreshes it
/// automatically as needed, so construct one [`AuthClient`]/[`ScribeClient`]
/// pair per authenticated user session and reuse it across requests.
/// Path of the web page where a user requests, or cancels, deletion of their own
/// account.
///
/// Mounted under `/settings` in the Pneumalith webapp, not at a top-level
/// `/delete_account`. Reach it through [`ScribeClient::delete_account_url`] so the
/// browser arrives already signed in. The same page starts a deletion and cancels
/// a pending one, so one path serves both.
pub const DELETE_ACCOUNT_PATH: &str = "/settings/delete_account";

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

    /// Returns a snapshot of the current token set. Useful for persisting
    /// the session after the client has auto-refreshed it.
    pub async fn current_tokens(&self) -> TokenSet {
        self.tokens.lock().await.clone()
    }

    /// Force-refreshes and replaces the current token set. Also used after
    /// a request unexpectedly comes back `401` even though our local
    /// expiry tracking thought the token was still good.
    async fn force_refresh(&self) -> Result<String, ScribeError> {
        let mut tokens = self.tokens.lock().await;
        let refresh_token =
            tokens
                .refresh_token
                .clone()
                .ok_or_else(|| ScribeError::InvalidGrant {
                    message: "Your session has expired. Please sign in again.".to_string(),
                })?;
        *tokens = self.auth.refresh(&refresh_token).await?;
        Ok(tokens.access_token.clone())
    }

    /// Creates a document from `source`.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] if the request
    /// fails or the server rejects it.
    /// `password` unlocks a password-protected source file. Supplying it up
    /// front saves a round trip when the caller already knows the document
    /// is protected; otherwise leave it `None` and handle the
    /// `password_required` signal on the document channel.
    pub async fn create_document(
        &self,
        source: DocumentSource,
        password: Option<&str>,
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
            let form = match password {
                Some(password) => form.text("document[password]", password.to_string()),
                None => form,
            };
            self.http
                .post(url.clone())
                .bearer_auth(token)
                .multipart(form)
        })
        .await
    }

    /// Lists every document owned by the current user, each with its
    /// outputs embedded (so this alone is enough to show a document list
    /// with per-document conversion status, no follow-up `list_outputs`
    /// calls needed).
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn list_documents(&self) -> Result<DocumentList, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/documents");

        let response: DocumentListResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;

        Ok(DocumentList {
            documents: response.documents,
            pages_remaining: response.pages_remaining,
        })
    }

    /// Moves a document to the trash. It's permanently deleted 7 days later,
    /// or sooner per the owner's org retention policy, unless recovered
    /// first with [`recover_document`](Self::recover_document).
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures.
    pub async fn trash_document(&self, document_id: &str) -> Result<(), ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/trash"));
        self.with_auth_retry_raw(|token| self.http.post(url.clone()).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Permanently deletes a document and all of its outputs. The document
    /// must already be in the trash (see
    /// [`trash_document`](Self::trash_document)).
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller,
    /// [`ScribeError::NotTrashed`] if it hasn't been moved to the trash yet,
    /// or [`ScribeError::Http`]/[`ScribeError::Api`] on other request
    /// failures.
    pub async fn delete_document_permanently(&self, document_id: &str) -> Result<(), ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}"));
        self.with_auth_retry_raw(|token| self.http.delete(url.clone()).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Restores a trashed document, clearing its trash state.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures.
    pub async fn recover_document(&self, document_id: &str) -> Result<(), ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/recover"));
        self.with_auth_retry_raw(|token| self.http.post(url.clone()).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Lists the caller's trashed documents, most recently trashed first.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn list_trashed_documents(&self) -> Result<Vec<TrashedDocument>, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/documents/trash");
        let response: TrashedDocumentListResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;
        Ok(response.documents)
    }

    /// Submits a document for human review, attaching `comment` as the
    /// reason / description of the problem.
    pub async fn submit_document_feedback(
        &self,
        document_id: &str,
        comment: &str,
    ) -> Result<(), ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/feedback"));
        let body = serde_json::json!({ "comment": comment });
        self.with_auth_retry_raw(|token| {
            self.http.post(url.clone()).bearer_auth(token).json(&body)
        })
        .await?;
        Ok(())
    }

    /// Opens a real-time channel for `document_id`, subscribing to
    /// progress on whatever formats are already in progress (or their
    /// incomplete parent formats) and allowing new conversions to be
    /// started with [`DocumentChannel::start_conversion`]. This is the
    /// only way to start a conversion for a format other than the
    /// `html_stream` preview `create_document` already starts — it's no
    /// longer possible over plain REST, so the server can guarantee it's
    /// subscribed to whatever it just started converting.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::WebSocket`] if the connection fails.
    pub async fn open_document_channel(
        &self,
        document_id: &str,
    ) -> Result<DocumentChannel, ScribeError> {
        let token = self.access_token().await?;
        match self.connect_ws_channel(document_id, &token).await {
            // WebSocket handshake failure likely means the token was rejected
            // (Phoenix returns 403 when UserSocket.connect/3 returns :error).
            // Retry once with a force-refreshed token before giving up.
            Err(ScribeError::WebSocket(_)) => {
                let token = self.force_refresh().await?;
                self.connect_ws_channel(document_id, &token).await
            }
            other => other,
        }
    }

    async fn connect_ws_channel(
        &self,
        document_id: &str,
        token: &str,
    ) -> Result<DocumentChannel, ScribeError> {
        let mut url = self.base_url.clone();
        let ws_scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(ws_scheme)
            .map_err(|()| ScribeError::Channel {
                message: "Something went wrong. Please try again.".to_string(),
            })?;
        url.set_path("/socket/websocket");
        url.query_pairs_mut()
            .append_pair("vsn", "2.0.0")
            .append_pair("token", token);
        let (ws, _response) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| ScribeError::WebSocket(Box::new(e)))?;
        DocumentChannel::join(ws, document_id).await
    }

    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Http`]/[`ScribeError::Api`] on other request failures.
    pub async fn list_outputs(&self, document_id: &str) -> Result<OutputList, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}/outputs"));
        let response: OutputListResponse = self
            .with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;
        Ok(OutputList {
            outputs: response.outputs,
            is_password_needed: response.is_password_needed,
        })
    }

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
    ) -> Result<Download, ScribeError> {
        self.download_output_with_progress(document_id, format, |_, _| {})
            .await
    }

    /// As [`download_output`](Self::download_output), reporting bytes received
    /// as they arrive: `(downloaded, total)`, where `total` is `None` when the
    /// server doesn't say how big the file is.
    ///
    /// Audio of a long document is tens of megabytes, and the whole download
    /// used to be one silent wait.
    ///
    /// # Errors
    ///
    /// The same as [`download_output`](Self::download_output).
    pub async fn download_output_with_progress<F>(
        &self,
        document_id: &str,
        format: OutputFormat,
        mut on_progress: F,
    ) -> Result<Download, ScribeError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "/api/documents/{document_id}/outputs/{}/download",
            format.as_str()
        ));
        let mut response = self
            .with_auth_retry_raw(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;
        let file_name = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(content_disposition_file_name);
        let total = response_total_bytes(&response);

        let mut data: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
        on_progress(0, total);
        while let Some(chunk) = response.chunk().await? {
            data.extend_from_slice(&chunk);
            on_progress(data.len() as u64, total);
        }
        Ok(Download { data, file_name })
    }

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
        self.with_auth_retry(|token| self.http.patch(url.clone()).bearer_auth(token).json(&body))
            .await
    }

    /// Changes a document's title.
    ///
    /// Titles start life as the uploaded file name, which for a photo or a
    /// share-sheet import is something like "IMG_6746783276432", so this is how
    /// a caller fixes one after the fact. Returns the document as the list
    /// endpoint describes it, so a caller can refresh its row from the result.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or
    /// [`ScribeError::Api`] if the title is rejected (it may not be blank and
    /// is capped at 255 characters).
    pub async fn rename_document(
        &self,
        document_id: &str,
        title: &str,
    ) -> Result<DocumentSummary, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/documents/{document_id}"));
        let body = serde_json::json!({ "document": { "title": title } });
        self.with_auth_retry(|token| self.http.patch(url.clone()).bearer_auth(token).json(&body))
            .await
    }

    /// A URL that signs the system browser in as this client's user, then lands
    /// on `return_to_path`.
    ///
    /// The app and the browser do not share a session, so a bare link to a
    /// settings page would greet the user with a login form. This hands the
    /// app's access token to `/auth/browser_from_oauth`, which trades it for a
    /// browser session and redirects.
    ///
    /// Refreshes the token first when it is near expiry. The server rejects a
    /// stale one with a "Failed to log in" page, and that is the page the user
    /// would land on.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::InvalidGrant`] if the token has expired and cannot
    /// be refreshed, or [`ScribeError::Http`] if the refresh request fails.
    pub async fn browser_entry_url(&self, return_to_path: &str) -> Result<String, ScribeError> {
        let token = self.access_token().await?;
        let state = serde_json::json!({ "return_to": return_to_path }).to_string();
        let mut url = self.base_url.clone();
        url.set_path("/auth/browser_from_oauth");
        url.query_pairs_mut()
            .append_pair("token", &token)
            .append_pair("state", &state);
        Ok(url.into())
    }

    /// The account-deletion page, signed in. App Store guideline 5.1.1(v)
    /// requires an app with account creation to let users start deletion from
    /// inside the app.
    ///
    /// # Errors
    ///
    /// As for [`ScribeClient::browser_entry_url`].
    pub async fn delete_account_url(&self) -> Result<String, ScribeError> {
        self.browser_entry_url(DELETE_ACCOUNT_PATH).await
    }

    /// The caller's default conversion settings, which every new document's
    /// settings are seeded from.
    ///
    /// Account-scoped, so there is no id: a user has exactly one set, and the
    /// server creates it on first read rather than returning a 404.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn default_settings(&self) -> Result<Settings, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/document_settings");
        self.with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await
    }

    /// Updates the caller's default conversion settings. Partial: fields left
    /// `None` on `update` are not sent, so they keep their current value.
    ///
    /// Changing these does not touch documents that already exist.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure,
    /// including validation errors on the settings themselves.
    pub async fn update_default_settings(
        &self,
        update: &SettingsUpdate,
    ) -> Result<Settings, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/document_settings");
        let body = serde_json::json!({ "settings": update });
        self.with_auth_retry(|token| self.http.patch(url.clone()).bearer_auth(token).json(&body))
            .await
    }

    /// Registers `token` (the hex-encoded APNs device token) so the server
    /// can send this device push notifications. Registration is an upsert —
    /// re-registering the same token re-points it at the current user (e.g.
    /// after signing out and into a different account on the same device).
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn register_device(&self, token: &str, platform: &str) -> Result<(), ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/devices");
        let body = serde_json::json!({ "token": token, "platform": platform });
        self.with_auth_retry_raw(|token| {
            self.http.post(url.clone()).bearer_auth(token).json(&body)
        })
        .await?;
        Ok(())
    }

    /// Unregisters `token`, e.g. on sign-out, so this device stops receiving push.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn unregister_device(&self, token: &str) -> Result<(), ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/devices/{token}"));
        self.with_auth_retry_raw(|t| self.http.delete(url.clone()).bearer_auth(t))
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn get_notification_settings(&self) -> Result<NotificationSettings, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/notification_settings");
        self.with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await
    }

    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn update_notification_settings(
        &self,
        push_notify_when_complete: bool,
    ) -> Result<NotificationSettings, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/notification_settings");
        let body = serde_json::json!({ "push_notify_when_complete": push_notify_when_complete });
        self.with_auth_retry(|token| self.http.patch(url.clone()).bearer_auth(token).json(&body))
            .await
    }

    /// # Errors
    ///
    /// Returns [`ScribeError::Http`]/[`ScribeError::Api`] on request failure.
    pub async fn get_account_info(&self) -> Result<AccountInfo, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/api/account_info");
        self.with_auth_retry(|token| self.http.get(url.clone()).bearer_auth(token))
            .await
    }

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
    /// force-refreshed token if the server returns `401`. Returns the
    /// validated response for callers that need to read the body themselves.
    async fn with_auth_retry_raw<F>(&self, build: F) -> Result<reqwest::Response, ScribeError>
    where
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
        response.error_for_status_or_json_error().await
    }

    /// Like [`with_auth_retry_raw`] but deserializes the response body as JSON.
    async fn with_auth_retry<T, F>(&self, build: F) -> Result<T, ScribeError>
    where
        T: serde::de::DeserializeOwned,
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        self.with_auth_retry_raw(build)
            .await?
            .json()
            .await
            .map_err(Into::into)
    }
}

/// Small helper trait so response-status handling reads the same way at every call site.
/// How many bytes the body will be, if the server says.
///
/// A download streamed from object storage goes out chunked, which has no
/// `Content-Length`, so the API repeats the size it already knows in a header
/// of its own. Without either, the caller only learns the total at the end.
fn response_total_bytes(response: &reqwest::Response) -> Option<u64> {
    response.content_length().or_else(|| {
        response
            .headers()
            .get("x-scribe-content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
    })
}

/// The file name in a `Content-Disposition` header. Prefers the UTF-8
/// `filename*` (RFC 6266) over the ASCII-only `filename` fallback, and keeps
/// only the last path component so a name can never point outside the folder
/// it is saved in.
fn content_disposition_file_name(header: &str) -> Option<String> {
    let mut fallback = None;
    let mut extended = None;
    for param in header.split(';').map(str::trim) {
        if let Some(value) = param.strip_prefix("filename*=") {
            // `charset'language'value`. The server always sends UTF-8, and
            // its value keeps a literal "+", so this must not be decoded as a
            // form, which would turn it into a space.
            let mut parts = value.splitn(3, '\'');
            if let (Some(charset), Some(_), Some(encoded)) =
                (parts.next(), parts.next(), parts.next())
            {
                if charset.eq_ignore_ascii_case("utf-8") {
                    extended = percent_encoding::percent_decode_str(encoded)
                        .decode_utf8()
                        .ok()
                        .map(|name| name.into_owned());
                }
            }
        } else if let Some(value) = param.strip_prefix("filename=") {
            fallback = Some(value.trim_matches('"').to_string());
        }
    }
    extended
        .or(fallback)
        .and_then(|name| name.rsplit(['/', '\\']).next().map(str::to_string))
        .filter(|name| !name.is_empty())
}

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
        // A parse failure here means the body wasn't our JSON envelope at
        // all (an HTML error page, a proxy's plain-text response, etc.) —
        // never surface that raw text, fall back to a generic message.
        let Ok(body) = serde_json::from_str::<ApiErrorResponse>(&text) else {
            return Err(ScribeError::Api {
                status: status.as_u16(),
                code: "server_error".to_string(),
                message: "Something went wrong. Please try again.".to_string(),
            });
        };
        let message = body
            .error
            .message
            .unwrap_or_else(|| "Something went wrong. Please try again.".to_string());
        Err(match body.error.code.as_str() {
            "not_found" => ScribeError::NotFound { message },
            "forbidden" => ScribeError::Forbidden { message },
            "not_trashed" => ScribeError::NotTrashed { message },
            "conversion_not_complete" => ScribeError::ConversionNotComplete { message },
            _ => ScribeError::Api {
                status: status.as_u16(),
                code: body.error.code,
                message,
            },
        })
    }
}
