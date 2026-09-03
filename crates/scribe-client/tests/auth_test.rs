use scribe_client::{AuthClient, PkceChallenge, ScribeError};
use url::Url;
use wiremock::{
    matchers::{body_string_contains, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn auth_client(base_url: &str) -> AuthClient {
    AuthClient::new(
        reqwest::Client::new(),
        Url::parse(base_url).unwrap(),
        "test-client-id",
    )
}

#[test]
fn authorization_url_wraps_the_authorize_request_in_a_logout_redirect() {
    // Signing out can only drop this device's tokens, so the authorize request
    // is reached through /auth/logout to clear whatever session the browser is
    // still holding for this server.
    let client = auth_client("https://scribe.example/");
    let pkce = PkceChallenge::generate();
    let url = client.authorization_url("myapp://callback", &pkce);

    assert_eq!(url.path(), "/auth/logout");
    let next = url
        .query_pairs()
        .find(|(key, _)| key == "next")
        .expect("carries a next parameter")
        .1
        .into_owned();

    // A path and query, never an absolute URL: the server refuses external
    // targets so this cannot be turned into an open redirect.
    assert!(next.starts_with("/oauth/authorize?"), "{next}");

    let authorize = Url::parse("https://scribe.example/")
        .unwrap()
        .join(&next)
        .unwrap();
    let pairs: std::collections::HashMap<_, _> = authorize.query_pairs().collect();
    assert_eq!(pairs.get("response_type").unwrap(), "code");
    assert_eq!(pairs.get("code_challenge").unwrap(), pkce.challenge());
    assert_eq!(pairs.get("code_challenge_method").unwrap(), "S256");
    assert_eq!(pairs.get("redirect_uri").unwrap(), "myapp://callback");
    assert_eq!(pairs.get("client_id").unwrap(), "test-client-id");
}

#[tokio::test]
async fn revoke_posts_the_token_to_the_revocation_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/revoke"))
        .and(body_string_contains("token=rt-456"))
        .and(body_string_contains("client_id=test-client-id"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    client.revoke("rt-456").await.unwrap();
}

#[tokio::test]
async fn revoke_succeeds_for_a_token_the_server_does_not_know() {
    // RFC 7009: the server answers successfully whether or not it recognised
    // the token, so signing out cannot be blocked by an already dead token.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/revoke"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    assert!(client.revoke("already-revoked").await.is_ok());
}

#[tokio::test]
async fn revoke_maps_a_refusal_to_the_api_variant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/revoke"))
        .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
            "error": "temporarily_unavailable"
        })))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let result = client.revoke("rt-456").await;
    assert!(matches!(
        result,
        Err(ScribeError::Api { status: 503, .. })
    ));
}

#[tokio::test]
async fn exchange_code_returns_token_set_on_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code_verifier=the-verifier"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-123",
            "refresh_token": "rt-456",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let tokens = client
        .exchange_code("myapp://callback", "auth-code", "the-verifier")
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "at-123");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-456"));
    assert!(tokens.expires_at.is_some());
}

#[tokio::test]
async fn exchange_code_maps_invalid_grant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "PKCE verification failed"
        })))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let result = client
        .exchange_code("myapp://callback", "auth-code", "wrong-verifier")
        .await;
    assert!(matches!(result, Err(ScribeError::InvalidGrant { .. })));
}

#[tokio::test]
async fn exchange_code_maps_unrecognized_error_to_api_variant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "unsupported_grant_type"
        })))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let result = client
        .exchange_code("myapp://callback", "auth-code", "the-verifier")
        .await;
    match result {
        Err(ScribeError::Api {
            status,
            code,
            message,
        }) => {
            assert_eq!(status, 400);
            assert_eq!(code, "unsupported_grant_type");
            assert_eq!(message, "We couldn't sign you in. Please try again.");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn exchange_code_maps_non_json_error_body_to_api_variant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(502).set_body_string("upstream timeout"))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let result = client
        .exchange_code("myapp://callback", "auth-code", "the-verifier")
        .await;
    match result {
        // A non-JSON body (e.g. an upstream proxy's plain-text error) must
        // never be surfaced verbatim — falls back to a generic message.
        Err(ScribeError::Api {
            status,
            code,
            message,
        }) => {
            assert_eq!(status, 502);
            assert_eq!(code, "server_error");
            assert_eq!(message, "We couldn't sign you in. Please try again.");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn refresh_maps_invalid_grant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "refresh token revoked"
        })))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let result = client.refresh("rt-revoked").await;
    assert!(matches!(result, Err(ScribeError::InvalidGrant { .. })));
}

#[tokio::test]
async fn refresh_returns_new_token_set() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-new",
            "refresh_token": "rt-new",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;
    let client = auth_client(&server.uri());
    let tokens = client.refresh("rt-old").await.unwrap();
    assert_eq!(tokens.access_token, "at-new");
}
