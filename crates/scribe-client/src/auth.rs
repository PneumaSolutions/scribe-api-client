use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use url::Url;

use crate::error::ScribeError;

/// A generated PKCE (RFC 7636) verifier/challenge pair. Always uses the
/// `S256` challenge method; there's no reason for a client we control to
/// use `plain`, even though the server supports it.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    verifier: String,
    challenge: String,
}

impl PkceChallenge {
    /// Generates a new random verifier and its S256 challenge.
    #[must_use]
    pub fn generate() -> Self {
        // RFC 7636 section 4.1: verifier is 43-128 chars from [A-Z a-z 0-9 - . _ ~].
        // 32 random bytes, base64url-no-pad-encoded, is 43 chars and pulls
        // only from that alphabet.
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = Self::derive_challenge(&verifier);
        PkceChallenge {
            verifier,
            challenge,
        }
    }

    fn derive_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        URL_SAFE_NO_PAD.encode(hasher.finalize())
    }

    #[must_use]
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    #[must_use]
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

/// An access/refresh token pair returned by `POST /oauth/token`.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<OffsetDateTime>,
}

impl TokenSet {
    /// True if this token is missing or will expire within `skew`.
    #[must_use]
    pub fn needs_refresh(&self, skew: Duration) -> bool {
        match self.expires_at {
            None => false,
            Some(expires_at) => {
                expires_at
                    <= OffsetDateTime::now_utc()
                        + time::Duration::seconds(skew.as_secs().cast_signed())
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[allow(dead_code)]
    error_description: Option<String>,
}

impl From<TokenResponse> for TokenSet {
    fn from(r: TokenResponse) -> Self {
        TokenSet {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            expires_at: r
                .expires_in
                .map(|secs| OffsetDateTime::now_utc() + time::Duration::seconds(secs)),
        }
    }
}

/// Drives the OAuth 2.0 Authorization Code + PKCE flow against a Scribe
/// server. This type does not open a browser or run a redirect listener;
/// the embedding application is responsible for presenting
/// [`AuthClient::authorization_url`] to the user and obtaining the
/// resulting `code` however fits its own UI.
pub struct AuthClient {
    http: reqwest::Client,
    base_url: Url,
    client_id: String,
}

impl AuthClient {
    pub fn new(http: reqwest::Client, base_url: Url, client_id: impl Into<String>) -> Self {
        AuthClient {
            http,
            base_url,
            client_id: client_id.into(),
        }
    }

    /// Builds the URL the user's browser should be sent to. `redirect_uri`
    /// must match one registered for `client_id` server-side.
    #[must_use]
    pub fn authorization_url(&self, redirect_uri: &str, pkce: &PkceChallenge) -> Url {
        let mut url = self.base_url.clone();
        url.set_path("/oauth/authorize");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("code_challenge", pkce.challenge())
            .append_pair("code_challenge_method", "S256");
        url
    }

    /// Exchanges an authorization code (obtained after the user completes
    /// the browser flow at [`AuthClient::authorization_url`]) for a token
    /// set. `verifier` must be the same [`PkceChallenge`] used to build
    /// that URL.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::InvalidGrant`] if the code or verifier is
    /// wrong or expired, or [`ScribeError::Http`]/[`ScribeError::Api`] on
    /// other request failures.
    pub async fn exchange_code(
        &self,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> Result<TokenSet, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/oauth/token");
        let body = [
            ("grant_type", "authorization_code"),
            ("client_id", &self.client_id),
            ("redirect_uri", redirect_uri),
            ("code", code),
            ("code_verifier", verifier),
        ];
        self.send_token_request(url, &body).await
    }

    /// Exchanges a refresh token for a new token set.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::InvalidGrant`] if the refresh token is wrong,
    /// expired, or revoked, or [`ScribeError::Http`]/[`ScribeError::Api`] on
    /// other request failures.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet, ScribeError> {
        let mut url = self.base_url.clone();
        url.set_path("/oauth/token");
        let body = [
            ("grant_type", "refresh_token"),
            ("client_id", &self.client_id),
            ("refresh_token", refresh_token),
        ];
        self.send_token_request(url, &body).await
    }

    async fn send_token_request(
        &self,
        url: Url,
        body: &[(&str, &str)],
    ) -> Result<TokenSet, ScribeError> {
        let response = self.http.post(url).form(body).send().await?;
        let status = response.status();
        if status.is_success() {
            let parsed: TokenResponse = response.json().await?;
            Ok(parsed.into())
        } else {
            let text = response.text().await.unwrap_or_default();
            match serde_json::from_str::<TokenErrorResponse>(&text) {
                Ok(err) if err.error == "invalid_grant" => {
                    Err(ScribeError::InvalidGrant(err.error))
                }
                Ok(err) => Err(ScribeError::Api {
                    status: status.as_u16(),
                    error: err.error,
                }),
                Err(_) => Err(ScribeError::Api {
                    status: status.as_u16(),
                    error: text,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_verifier_matches_rfc7636_charset_and_length() {
        let pkce = PkceChallenge::generate();
        assert_eq!(pkce.verifier().len(), 43);
        assert!(pkce.verifier().chars().all(|c| c.is_ascii_alphanumeric()
            || c == '-'
            || c == '.'
            || c == '_'
            || c == '~'));
    }

    #[test]
    fn challenge_is_deterministic_s256_of_verifier() {
        // RFC 7636 appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(
            PkceChallenge::derive_challenge(verifier),
            expected_challenge
        );
    }

    #[test]
    fn two_generated_challenges_differ() {
        let a = PkceChallenge::generate();
        let b = PkceChallenge::generate();
        assert_ne!(a.verifier(), b.verifier());
        assert_ne!(a.challenge(), b.challenge());
    }

    #[test]
    fn token_with_no_expiry_never_needs_refresh() {
        let tokens = TokenSet {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!tokens.needs_refresh(Duration::from_secs(30)));
    }

    #[test]
    fn token_well_outside_skew_does_not_need_refresh() {
        let tokens = TokenSet {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
        };
        assert!(!tokens.needs_refresh(Duration::from_secs(30)));
    }

    #[test]
    fn token_within_skew_of_expiry_needs_refresh() {
        let tokens = TokenSet {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: Some(OffsetDateTime::now_utc() + time::Duration::seconds(5)),
        };
        assert!(tokens.needs_refresh(Duration::from_secs(30)));
    }

    #[test]
    fn already_expired_token_needs_refresh() {
        let tokens = TokenSet {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: Some(OffsetDateTime::now_utc() - time::Duration::hours(1)),
        };
        assert!(tokens.needs_refresh(Duration::from_secs(30)));
    }
}
