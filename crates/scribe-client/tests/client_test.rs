use scribe_client::{DocumentSource, OutputFormat, ScribeClient, ScribeError, TokenSet};
use time::OffsetDateTime;
use url::Url;
use wiremock::{
    matchers::{body_string_contains, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn valid_tokens() -> TokenSet {
    TokenSet {
        access_token: "at-valid".into(),
        refresh_token: Some("rt-valid".into()),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
    }
}

fn client_for(server: &MockServer, tokens: TokenSet) -> ScribeClient {
    ScribeClient::new(
        reqwest::Client::new(),
        Url::parse(&server.uri()).unwrap(),
        "test-client-id",
        tokens,
    )
}

#[tokio::test]
async fn create_document_from_file_returns_document_id() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/documents"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "document_id": "doc-1"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let doc = client
        .create_document(DocumentSource::File {
            file_name: "report.docx".into(),
            bytes: b"pretend docx bytes".to_vec(),
        })
        .await
        .unwrap();

    assert_eq!(doc.document_id, "doc-1");
}

#[tokio::test]
async fn list_outputs_parses_in_progress_and_complete_rows() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "outputs": [
                {
                    "format": "html_stream",
                    "stage": "convert",
                    "progress": 0.5,
                    "estimated_time_remaining": 10,
                    "is_preview": true
                },
                {
                    "format": "pdf",
                    "stage": "complete",
                    "progress": 1.0,
                    "estimated_time_remaining": null,
                    "is_preview": false
                }
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let outputs = client.list_outputs("doc-1").await.unwrap();

    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].format, OutputFormat::HtmlStream);
    assert!(!outputs[0].stage.is_complete());
    assert_eq!(outputs[1].format, OutputFormat::Pdf);
    assert!(outputs[1].stage.is_complete());
}

#[tokio::test]
async fn download_output_returns_bytes_when_complete() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/pdf/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4 fake".to_vec()))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let bytes = client
        .download_output("doc-1", OutputFormat::Pdf)
        .await
        .unwrap();

    assert_eq!(bytes, b"%PDF-1.4 fake".to_vec());
}

#[tokio::test]
async fn download_output_maps_conversion_not_complete() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/pdf/download"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": "conversion_not_complete"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let result = client.download_output("doc-1", OutputFormat::Pdf).await;

    assert!(matches!(result, Err(ScribeError::ConversionNotComplete)));
}

#[tokio::test]
async fn create_document_from_url_returns_document_id() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/documents"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "document_id": "doc-2"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let doc = client
        .create_document(DocumentSource::Url("https://example.com/report.pdf".into()))
        .await
        .unwrap();

    assert_eq!(doc.document_id, "doc-2");
}

#[tokio::test]
async fn list_outputs_maps_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/missing/outputs"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "not_found"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let result = client.list_outputs("missing").await;

    assert!(matches!(result, Err(ScribeError::NotFound)));
}

#[tokio::test]
async fn list_outputs_maps_forbidden() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/other-users-doc/outputs"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": "forbidden"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let result = client.list_outputs("other-users-doc").await;

    assert!(matches!(result, Err(ScribeError::Forbidden)));
}

#[tokio::test]
async fn list_outputs_maps_unrecognized_error_to_api_variant() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": "internal_error"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let result = client.list_outputs("doc-1").await;

    match result {
        Err(ScribeError::Api { status, error }) => {
            assert_eq!(status, 500);
            assert_eq!(error, "internal_error");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_second_401_after_refresh_is_not_retried_again() {
    let server = MockServer::start().await;

    // Every request gets a 401, including the retry after refresh: the
    // client must not loop forever, so the second 401 should surface as
    // an HTTP error rather than triggering another refresh attempt.
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-fresh",
            "refresh_token": "rt-fresh",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    let stale_tokens = TokenSet {
        access_token: "at-stale".into(),
        refresh_token: Some("rt-stale".into()),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
    };

    let client = client_for(&server, stale_tokens);
    let result = client.list_outputs("doc-1").await;

    // The second 401 has no JSON body, so it should surface as a generic
    // Api error with status 401 rather than panicking or looping.
    assert!(matches!(
        result,
        Err(ScribeError::Api { status: 401, .. })
    ));
}

#[tokio::test]
async fn a_401_with_no_refresh_token_available_surfaces_invalid_grant() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let tokens_without_refresh = TokenSet {
        access_token: "at-stale".into(),
        refresh_token: None,
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
    };

    let client = client_for(&server, tokens_without_refresh);
    let result = client.list_outputs("doc-1").await;

    assert!(matches!(result, Err(ScribeError::InvalidGrant(_))));
}

#[tokio::test]
async fn proactive_refresh_happens_before_expiry_without_a_401() {
    let server = MockServer::start().await;

    // No mock for the stale token: if the client didn't proactively refresh
    // (it's within REFRESH_SKEW of expiring), this request would 404 against
    // wiremock's default "no matching mock" response rather than succeeding.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-fresh",
            "refresh_token": "rt-fresh",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .and(header("authorization", "Bearer at-fresh"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "outputs": [] })),
        )
        .mount(&server)
        .await;

    let about_to_expire = TokenSet {
        access_token: "at-stale".into(),
        refresh_token: Some("rt-stale".into()),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::seconds(5)),
    };

    let client = client_for(&server, about_to_expire);
    let outputs = client.list_outputs("doc-1").await.unwrap();

    assert!(outputs.is_empty());
}

#[tokio::test]
async fn a_401_triggers_refresh_and_retries_once() {
    let server = MockServer::start().await;

    // The initial (stale) token gets a 401, which triggers a refresh, and
    // the retry with the new token succeeds.
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .and(header("authorization", "Bearer at-stale"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-fresh",
            "refresh_token": "rt-fresh",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .and(header("authorization", "Bearer at-fresh"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "outputs": [] })),
        )
        .mount(&server)
        .await;

    let stale_tokens = TokenSet {
        access_token: "at-stale".into(),
        refresh_token: Some("rt-stale".into()),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
    };

    let client = client_for(&server, stale_tokens);
    let outputs = client.list_outputs("doc-1").await.unwrap();

    assert!(outputs.is_empty());
}
