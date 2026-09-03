//! The OAuth 2.0 Authorization Code + PKCE flow.

use std::sync::Arc;

use reqwest::Client;
use url::Url;

use scribe_client_core::AuthClient;

use crate::{http_client, parse_url, runtime, PkceSession, ScribeError, TokenSet};

/// Generates a fresh PKCE verifier/challenge pair (RFC 7636, S256 method).
#[uniffi::export]
pub fn generate_pkce_session() -> PkceSession {
    let pkce = scribe_client_core::PkceChallenge::generate();
    PkceSession {
        verifier: pkce.verifier().to_string(),
        challenge: pkce.challenge().to_string(),
    }
}

/// Drives the OAuth 2.0 Authorization Code + PKCE flow. Does not open a
/// browser or handle the redirect; the app is responsible for presenting the
/// authorization URL and returning the resulting code.
#[derive(uniffi::Object)]
pub struct FfiAuthClient {
    http: Client,
    base_url: Url,
    client_id: String,
}

#[uniffi::export]
impl FfiAuthClient {
    #[uniffi::constructor]
    pub fn new(base_url: String, client_id: String) -> Result<Arc<Self>, ScribeError> {
        let base_url = parse_url(&base_url)?;
        Ok(Arc::new(FfiAuthClient {
            http: http_client(),
            base_url,
            client_id,
        }))
    }

    /// Returns the URL the user's browser should be sent to.
    /// `pkce_challenge` is the `challenge` field from [`generate_pkce_session`].
    pub fn authorization_url(
        &self,
        redirect_uri: String,
        pkce_challenge: String,
    ) -> Result<String, ScribeError> {
        let auth = AuthClient::new(self.http.clone(), self.base_url.clone(), &self.client_id);
        Ok(auth
            .authorization_url_with_challenge(&redirect_uri, &pkce_challenge)
            .to_string())
    }

    /// Exchanges an authorization code for tokens.
    /// `verifier` is the `verifier` field from the same [`generate_pkce_session`]
    /// call used to build the authorization URL.
    pub fn exchange_code(
        &self,
        redirect_uri: String,
        code: String,
        verifier: String,
    ) -> Result<TokenSet, ScribeError> {
        let auth = AuthClient::new(self.http.clone(), self.base_url.clone(), &self.client_id);
        runtime()
            .block_on(auth.exchange_code(&redirect_uri, &code, &verifier))
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Exchanges a refresh token for a new token set.
    pub fn refresh(&self, refresh_token: String) -> Result<TokenSet, ScribeError> {
        let auth = AuthClient::new(self.http.clone(), self.base_url.clone(), &self.client_id);
        runtime()
            .block_on(auth.refresh(&refresh_token))
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Asks the server to invalidate a token, so that signing out ends the
    /// session there rather than leaving a refresh token usable until it
    /// expires. Pass the refresh token when there is one: revoking it takes
    /// the access tokens issued alongside it with it.
    pub fn revoke(&self, token: String) -> Result<(), ScribeError> {
        let auth = AuthClient::new(self.http.clone(), self.base_url.clone(), &self.client_id);
        runtime().block_on(auth.revoke(&token)).map_err(Into::into)
    }
}
