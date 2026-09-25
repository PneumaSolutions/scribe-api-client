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
        .create_document(
            DocumentSource::File {
                file_name: "report.docx".into(),
                bytes: b"pretend docx bytes".to_vec(),
            },
            None,
        )
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
                    "is_preview": true,
                    "downloadable": false
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
    let outputs = client.list_outputs("doc-1").await.unwrap().outputs;
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].format, OutputFormat::HtmlStream);
    assert!(!outputs[0].stage.is_complete());
    assert_eq!(outputs[1].format, OutputFormat::Pdf);
    assert!(outputs[1].stage.is_complete());
    assert!(!outputs[0].downloadable);
    // A server from before the field existed still offers the download.
    assert!(outputs[1].downloadable);
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
    let download = client
        .download_output("doc-1", OutputFormat::Pdf)
        .await
        .unwrap();
    assert_eq!(download.data, b"%PDF-1.4 fake".to_vec());
    assert_eq!(download.file_name, None);
}

async fn file_name_from_header(header: &str) -> Option<String> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/mp3/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-disposition", header)
                .set_body_bytes(b"ID3".to_vec()),
        )
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    client
        .download_output("doc-1", OutputFormat::Mp3)
        .await
        .unwrap()
        .file_name
}

/// The exact header the server sends for the QA report's document.
#[tokio::test]
async fn download_output_prefers_the_utf8_file_name() {
    let name = file_name_from_header(
        r#"attachment; filename="SensePlayer User Manual (Model - T90ET).mp3"; filename*=UTF-8''SensePlayer%E2%84%A2%20User%20Manual%20%28Model%20-%20T90ET%29.mp3"#,
    )
    .await;
    assert_eq!(
        name.as_deref(),
        Some("SensePlayer\u{2122} User Manual (Model - T90ET).mp3")
    );
}

/// The server leaves "+" unencoded, so it must not be read as a space.
#[tokio::test]
async fn download_output_keeps_a_literal_plus() {
    let name = file_name_from_header(
        r#"attachment; filename="C++ Notes.pdf"; filename*=UTF-8''C++%20Notes.pdf"#,
    )
    .await;
    assert_eq!(name.as_deref(), Some("C++ Notes.pdf"));
}

#[tokio::test]
async fn download_output_falls_back_to_the_ascii_file_name() {
    let name = file_name_from_header(r#"attachment; filename="Report.html""#).await;
    assert_eq!(name.as_deref(), Some("Report.html"));
}

#[tokio::test]
async fn download_output_drops_any_folders_in_the_file_name() {
    let name = file_name_from_header(r#"attachment; filename*=UTF-8''..%2F..%2FReport.pdf"#).await;
    assert_eq!(name.as_deref(), Some("Report.pdf"));
}

#[tokio::test]
async fn download_output_reports_progress_with_a_total() {
    let server = MockServer::start().await;
    let body = vec![7u8; 4096];
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/mp3/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let mut reports: Vec<(u64, Option<u64>)> = Vec::new();
    let download = client
        .download_output_with_progress("doc-1", OutputFormat::Mp3, |downloaded, total| {
            reports.push((downloaded, total));
        })
        .await
        .unwrap();

    assert_eq!(download.data.len(), body.len());
    assert_eq!(reports.first(), Some(&(0, Some(4096))));
    assert_eq!(reports.last(), Some(&(4096, Some(4096))));
}

/// The real body's length wins over anything else a response claims. The
/// `x-scribe-content-length` fallback only applies to a chunked response,
/// which has no Content-Length at all and which wiremock cannot produce.
#[tokio::test]
async fn download_output_prefers_the_real_content_length() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/mp3/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-scribe-content-length", "9000")
                .set_body_bytes(vec![1u8; 256]),
        )
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let mut totals: Vec<Option<u64>> = Vec::new();
    client
        .download_output_with_progress("doc-1", OutputFormat::Mp3, |_, total| totals.push(total))
        .await
        .unwrap();

    assert!(totals.iter().all(|total| *total == Some(256)),
            "expected the body's own length, got {totals:?}");
}

/// Without any size, progress still reports bytes so a caller can show
/// something moving.
#[tokio::test]
async fn download_output_reports_bytes_without_a_total() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/brf/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![3u8; 512]))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let mut last = 0u64;
    client
        .download_output_with_progress("doc-1", OutputFormat::Brf, |downloaded, _| {
            last = downloaded;
        })
        .await
        .unwrap();
    assert_eq!(last, 512);
}

/// The API redirects to storage by default, so a player can stream from there.
#[tokio::test]
async fn download_url_reports_where_the_file_really_lives() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/mp3/download"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "https://storage.example/doc-1.mp3?signature=abc"),
        )
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let url = client.download_url("doc-1", OutputFormat::Mp3).await.unwrap();
    assert_eq!(url.as_deref(), Some("https://storage.example/doc-1.mp3?signature=abc"));
}

/// A server that sends the file itself says so by not redirecting, and the
/// caller downloads it first instead.
#[tokio::test]
async fn download_url_is_none_when_the_server_sends_the_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/mp3/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ID3".to_vec()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    assert_eq!(client.download_url("doc-1", OutputFormat::Mp3).await.unwrap(), None);
}

#[tokio::test]
async fn download_url_maps_conversion_not_complete() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/mp3/download"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": {"code": "conversion_not_complete", "message": "This file isn't ready yet."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.download_url("doc-1", OutputFormat::Mp3).await;
    assert!(matches!(result, Err(ScribeError::ConversionNotComplete { .. })));
}

#[tokio::test]
async fn download_output_maps_conversion_not_complete() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs/pdf/download"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": {"code": "conversion_not_complete", "message": "This file isn't ready yet."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.download_output("doc-1", OutputFormat::Pdf).await;
    assert!(matches!(
        result,
        Err(ScribeError::ConversionNotComplete { .. })
    ));
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
        .create_document(
            DocumentSource::Url("https://example.com/report.pdf".into()),
            None,
        )
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
            "error": {"code": "not_found", "message": "We couldn't find that."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.list_outputs("missing").await;
    assert!(matches!(result, Err(ScribeError::NotFound { .. })));
}

#[tokio::test]
async fn list_outputs_maps_forbidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/other-users-doc/outputs"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": {"code": "forbidden", "message": "You don't have permission to do that."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.list_outputs("other-users-doc").await;
    assert!(matches!(result, Err(ScribeError::Forbidden { .. })));
}

#[tokio::test]
async fn list_outputs_maps_unrecognized_error_to_api_variant() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": {"code": "internal_error", "message": "Something broke upstream."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.list_outputs("doc-1").await;
    match result {
        Err(ScribeError::Api {
            status,
            code,
            message,
        }) => {
            assert_eq!(status, 500);
            assert_eq!(code, "internal_error");
            assert_eq!(message, "Something broke upstream.");
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
    assert!(matches!(result, Err(ScribeError::Api { status: 401, .. })));
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
    assert!(matches!(result, Err(ScribeError::InvalidGrant { .. })));
}

#[tokio::test]
async fn proactive_refresh_happens_before_expiry_without_a_401() {
    let server = MockServer::start().await;
    // No mock for the stale token: if the client didn't proactively refresh
    // (it's within REFRESH_SKEW of expiring), this request would 404 against
    // wiremock's default "no matching mock" response.
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
    let outputs = client.list_outputs("doc-1").await.unwrap().outputs;
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
    let outputs = client.list_outputs("doc-1").await.unwrap().outputs;
    assert!(outputs.is_empty());
}

#[tokio::test]
async fn list_documents_parses_documents_with_embedded_outputs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "documents": [
                {
                    "id": "doc-1",
                    "title": "Report",
                    "page_count": 3,
                    "inserted_at": "2026-07-08T20:04:24.000000Z",
                    "outputs": [
                        {
                            "format": "html_stream",
                            "stage": "complete",
                            "progress": 1.0,
                            "estimated_time_remaining": null,
                            "is_preview": false
                        }
                    ]
                }
            ]
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let documents = client.list_documents().await.unwrap().documents;
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id, "doc-1");
    assert_eq!(documents[0].title.as_deref(), Some("Report"));
    assert_eq!(documents[0].page_count, Some(3));
    assert_eq!(documents[0].outputs.len(), 1);
    assert_eq!(documents[0].outputs[0].format, OutputFormat::HtmlStream);
}

#[tokio::test]
async fn list_documents_handles_a_null_title() {
    // A URL-sourced document has no title until the converter determines
    // one; the server returns `"title": null` for it.
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/documents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "documents": [
                {
                    "id": "doc-1",
                    "title": null,
                    "page_count": null,
                    "inserted_at": "2026-07-09T13:19:30.000000Z",
                    "outputs": []
                }
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let documents = client.list_documents().await.unwrap().documents;

    assert_eq!(documents[0].title, None);
}

#[tokio::test]
async fn trash_document_succeeds_on_204() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/documents/doc-1/trash"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    client.trash_document("doc-1").await.unwrap();
}

#[tokio::test]
async fn trash_document_maps_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/documents/missing/trash"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": {"code": "not_found", "message": "We couldn't find that."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.trash_document("missing").await;
    assert!(matches!(result, Err(ScribeError::NotFound { .. })));
}

#[tokio::test]
async fn trash_document_maps_forbidden() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/documents/doc-1/trash"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": {"code": "forbidden", "message": "You don't have permission to do that."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.trash_document("doc-1").await;
    assert!(matches!(result, Err(ScribeError::Forbidden { .. })));
}

#[tokio::test]
async fn delete_document_permanently_succeeds_on_204() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/documents/doc-1"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    client.delete_document_permanently("doc-1").await.unwrap();
}

#[tokio::test]
async fn delete_document_permanently_maps_not_trashed() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/documents/doc-1"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": {"code": "not_trashed", "message": "This document isn't in the trash."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.delete_document_permanently("doc-1").await;
    assert!(matches!(result, Err(ScribeError::NotTrashed { .. })));
}

#[tokio::test]
async fn recover_document_succeeds_on_204() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/documents/doc-1/recover"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    client.recover_document("doc-1").await.unwrap();
}

#[tokio::test]
async fn recover_document_maps_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/documents/missing/recover"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": {"code": "not_found", "message": "We couldn't find that."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.recover_document("missing").await;
    assert!(matches!(result, Err(ScribeError::NotFound { .. })));
}

#[tokio::test]
async fn list_trashed_documents_parses_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/trash"))
        .and(header("authorization", "Bearer at-valid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "documents": [
                {
                    "id": "doc-1",
                    "title": "Report",
                    "page_count": 3,
                    "inserted_at": "2026-07-08T20:04:24.000000Z",
                    "trashed_at": "2026-08-03T13:46:26.000000Z",
                    "permanently_delete_at": "2026-08-10T13:46:26.000000Z"
                }
            ]
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let documents = client.list_trashed_documents().await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id, "doc-1");
    assert_eq!(documents[0].trashed_at, "2026-08-03T13:46:26.000000Z");
    assert_eq!(
        documents[0].permanently_delete_at,
        "2026-08-10T13:46:26.000000Z"
    );
}

#[tokio::test]
async fn list_outputs_reports_a_locked_document() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/outputs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "outputs": [],
            "is_password_needed": true
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let list = client.list_outputs("doc-1").await.unwrap();
    assert!(list.outputs.is_empty());
    assert!(list.is_password_needed);
}

#[tokio::test]
async fn list_documents_reports_a_locked_document() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "documents": [
                {
                    "id": "doc-1",
                    "title": "Locked",
                    "page_count": null,
                    "inserted_at": "2026-09-15T00:00:00Z",
                    "is_password_needed": true,
                    "outputs": []
                }
            ],
            "pages_remaining": 3
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let list = client.list_documents().await.unwrap();
    assert!(list.documents[0].is_password_needed);
}

#[tokio::test]
async fn create_document_sends_a_password_when_one_is_given() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/documents"))
        .and(body_string_contains("document[password]"))
        .and(body_string_contains("hunter2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "document_id": "doc-1"
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let doc = client
        .create_document(
            DocumentSource::File {
                file_name: "locked.pdf".into(),
                bytes: b"%PDF".to_vec(),
            },
            Some("hunter2"),
        )
        .await
        .unwrap();
    assert_eq!(doc.document_id, "doc-1");
}

fn document_json(title: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "doc-1",
        "title": title,
        "page_count": 3,
        "inserted_at": "2026-09-18T15:04:05.000000Z",
        "is_password_needed": false,
        "outputs": []
    })
}

#[tokio::test]
async fn rename_document_patches_the_document_and_returns_it() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/documents/doc-1"))
        .and(header("authorization", "Bearer at-valid"))
        // The server reads the title out of a nested "document" object, the
        // same shape its own form posts. A flat {"title": ...} is ignored.
        .and(body_string_contains(r#""document":{"title":"Quarterly Earnings"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(document_json("Quarterly Earnings")))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let document = client
        .rename_document("doc-1", "Quarterly Earnings")
        .await
        .unwrap();
    assert_eq!(document.title.as_deref(), Some("Quarterly Earnings"));
    assert_eq!(document.id, "doc-1");
}

#[tokio::test]
async fn rename_document_maps_a_rejected_title_to_an_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/documents/doc-1"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "error": {"code": "unprocessable_entity", "message": "Title can't be blank"}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let error = client.rename_document("doc-1", "").await.unwrap_err();
    assert!(
        matches!(error, ScribeError::Api { ref message, .. } if message.contains("blank")),
        "got {error:?}"
    );
}

fn query_params(url: &str) -> std::collections::HashMap<String, String> {
    Url::parse(url).unwrap().query_pairs().into_owned().collect()
}

#[tokio::test]
async fn delete_account_url_signs_the_browser_in_and_lands_on_the_settings_page() {
    let server = MockServer::start().await;
    let client = client_for(&server, valid_tokens());
    let url = client.delete_account_url().await.unwrap();

    let parsed = Url::parse(&url).unwrap();
    assert_eq!(parsed.path(), "/auth/browser_from_oauth");
    let params = query_params(&url);
    assert_eq!(params.get("token").map(String::as_str), Some("at-valid"));
    // The page is under /settings, not a top-level /delete_account.
    assert_eq!(
        params
            .get("state")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
        Some(serde_json::json!({ "return_to": scribe_client::DELETE_ACCOUNT_PATH }))
    );
    assert_eq!(scribe_client::DELETE_ACCOUNT_PATH, "/settings/delete_account");
}

/// The server answers a stale token with a "Failed to log in" page, which is
/// exactly where the user would land. So the URL has to carry a fresh one.
#[tokio::test]
async fn delete_account_url_refreshes_a_token_that_is_about_to_expire() {
    let server = MockServer::start().await;
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
    let about_to_expire = TokenSet {
        access_token: "at-stale".into(),
        refresh_token: Some("rt-stale".into()),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::seconds(5)),
    };
    let client = client_for(&server, about_to_expire);
    let url = client.delete_account_url().await.unwrap();
    assert_eq!(query_params(&url).get("token").map(String::as_str), Some("at-fresh"));
}

#[tokio::test]
async fn browser_entry_url_escapes_awkward_values() {
    let server = MockServer::start().await;
    let client = client_for(&server, valid_tokens());
    let url = client.browser_entry_url("/some path?a=b&c=d").await.unwrap();
    // It must round-trip exactly, or the redirect after sign-in goes astray.
    assert_eq!(
        query_params(&url)
            .get("state")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
        Some(serde_json::json!({ "return_to": "/some path?a=b&c=d" }))
    );
}
