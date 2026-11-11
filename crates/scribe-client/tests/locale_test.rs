//! The client's locale reaches the server on every path it can: `Accept-Language` on API and OAuth requests, `locale` on the socket and on the sign-in page.

use std::sync::{Arc, Mutex};

use scribe_client::{AuthClient, PkceChallenge, ScribeClient, TokenSet};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use url::Url;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn tokens(expires_in: time::Duration) -> TokenSet {
    TokenSet {
        access_token: "at-valid".into(),
        refresh_token: Some("rt-valid".into()),
        expires_at: Some(OffsetDateTime::now_utc() + expires_in),
    }
}

fn client(base_url: &str, tokens: TokenSet) -> ScribeClient {
    ScribeClient::new(
        reqwest::Client::new(),
        Url::parse(base_url).unwrap(),
        "test-client-id",
        tokens,
    )
}

async fn mount_languages(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/settings/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "languages": [["Englisch", "en"]]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn api_requests_carry_the_locale_as_accept_language() {
    let server = MockServer::start().await;
    mount_languages(&server).await;
    let client = client(&server.uri(), tokens(time::Duration::hours(1))).with_locale("de");

    client.languages().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].headers.get("accept-language").unwrap(), "de");
}

#[tokio::test]
async fn without_a_locale_no_accept_language_is_sent() {
    let server = MockServer::start().await;
    mount_languages(&server).await;
    let client = client(&server.uri(), tokens(time::Duration::hours(1)));

    client.languages().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].headers.get("accept-language").is_none());
}

/// A token refresh's errors are shown to the user too ("Your session has
/// expired"), so the refresh is asked for in the same language.
#[tokio::test]
async fn the_token_refresh_a_request_needs_carries_the_locale_too() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(header("accept-language", "fr"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-new",
            "refresh_token": "rt-new",
            "expires_in": 3600
        })))
        .expect(1)
        .mount(&server)
        .await;
    mount_languages(&server).await;
    let client = client(&server.uri(), tokens(time::Duration::seconds(-1))).with_locale("fr");

    client.languages().await.unwrap();

    assert_eq!(client.current_tokens().await.access_token, "at-new");
}

#[tokio::test]
async fn auth_requests_carry_the_locale() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/revoke"))
        .and(header("accept-language", "ja"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let auth = AuthClient::new(
        reqwest::Client::new(),
        Url::parse(&server.uri()).unwrap(),
        "test-client-id",
    )
    .with_locale("ja");

    auth.revoke("rt-1").await.unwrap();
}

#[test]
fn the_sign_in_page_is_asked_for_in_the_locale() {
    let auth = AuthClient::new(
        reqwest::Client::new(),
        Url::parse("https://scribe.example/").unwrap(),
        "test-client-id",
    );
    let pkce = PkceChallenge::generate();

    let plain = auth.authorization_url("myapp://callback", &pkce);
    assert!(!plain.query_pairs().any(|(key, _)| key == "locale"));

    let url = auth
        .with_locale("pt-BR")
        .authorization_url("myapp://callback", &pkce);
    let locale = url
        .query_pairs()
        .find(|(key, _)| key == "locale")
        .unwrap()
        .1;
    assert_eq!(locale, "pt-BR");
}

/// Accepts one WebSocket connection and reports the request URI it was made
/// to, then drops it: the URI is all these tests need.
async fn socket_uri_for(client: impl FnOnce(Url) -> ScribeClient) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let record = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // Its error type is tungstenite's, an HTTP response, not ours to shrink.
        #[allow(clippy::result_large_err)]
        let callback = move |request: &Request, response: Response| {
            *record.lock().unwrap() = request.uri().to_string();
            Ok(response)
        };
        let _ = tokio_tungstenite::accept_hdr_async(stream, callback).await;
    });
    let client = client(Url::parse(&format!("http://{addr}")).unwrap());
    // The server never answers the join, so this fails; the URI was still seen.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.open_document_channel("doc-1"),
    )
    .await;
    server.await.unwrap();
    let uri = seen.lock().unwrap().clone();
    uri
}

#[tokio::test]
async fn the_socket_is_opened_with_the_locale() {
    let uri = socket_uri_for(|base| {
        client(base.as_str(), tokens(time::Duration::hours(1))).with_locale("sv")
    })
    .await;
    let url = Url::parse(&format!("ws://localhost{uri}")).unwrap();
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(pairs.get("locale").unwrap(), "sv");
    assert_eq!(pairs.get("token").unwrap(), "at-valid");
}

#[tokio::test]
async fn without_a_locale_the_socket_names_none() {
    let uri = socket_uri_for(|base| client(base.as_str(), tokens(time::Duration::hours(1)))).await;
    assert!(!uri.contains("locale="), "{uri}");
}
