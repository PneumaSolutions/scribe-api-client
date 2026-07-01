use scribe_client::{DocumentSource, OutputFormat, ScribeClient, ScribeError, TokenSet};
use time::OffsetDateTime;
use url::Url;
use wiremock::{
    matchers::{header, method, path},
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
