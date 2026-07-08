use scribe_client::{ScribeClient, ScribeError, TokenSet};
use time::OffsetDateTime;
use url::Url;
use wiremock::{
    matchers::{method, path},
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

fn settings_json() -> serde_json::Value {
    serde_json::json!({
        "language": "en",
        "dialects": {},
        "voices": {},
        "tts_gender": null,
        "tts_rate": 1.0,
        "braille_translation_table": "en-us-g2.ctb",
        "braille_cells_per_line": 40,
        "braille_split_into_pages": true,
        "braille_lines_per_page": 25,
        "large_print": false,
        "add_image_descriptions": true,
        "math": false,
        "notify_when_complete": false
    })
}

#[tokio::test]
async fn get_settings_returns_current_document_settings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/doc-1/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings_json()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let settings = client.get_settings("doc-1").await.unwrap();
    assert_eq!(settings.language.as_deref(), Some("en"));
    assert_eq!(settings.braille_translation_table, "en-us-g2.ctb");
    assert!(settings.add_image_descriptions);
    assert!(!settings.large_print);
}

#[tokio::test]
async fn get_settings_maps_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/missing/settings"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "not_found"
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.get_settings("missing").await;
    assert!(matches!(result, Err(ScribeError::NotFound)));
}

#[tokio::test]
async fn update_settings_sends_partial_update_and_returns_new_settings() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/documents/doc-1/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "language": "en",
            "dialects": {},
            "voices": {},
            "tts_gender": null,
            "tts_rate": 1.0,
            "braille_translation_table": "en-gb-g1.utb",
            "braille_cells_per_line": 40,
            "braille_split_into_pages": true,
            "braille_lines_per_page": 25,
            "large_print": true,
            "add_image_descriptions": true,
            "math": false,
            "notify_when_complete": false
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let update = scribe_client::SettingsUpdate {
        large_print: Some(true),
        braille_translation_table: Some("en-gb-g1.utb".into()),
        ..Default::default()
    };
    let settings = client.update_settings("doc-1", &update).await.unwrap();

    assert!(settings.large_print);
    assert_eq!(settings.braille_translation_table, "en-gb-g1.utb");

    let request = &server.received_requests().await.unwrap()[0];
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["settings"]["large_print"], true);
    assert_eq!(
        body["settings"]["braille_translation_table"],
        "en-gb-g1.utb"
    );
    // Fields that weren't set shouldn't be sent at all.
    assert!(body["settings"].get("language").is_none());
}

#[tokio::test]
async fn update_settings_maps_forbidden() {
    let server = MockServer::start().await;

    Mock::given(method("PATCH"))
        .and(path("/api/documents/doc-1/settings"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": "forbidden"
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let result = client
        .update_settings("doc-1", &scribe_client::SettingsUpdate::default())
        .await;

    assert!(matches!(result, Err(ScribeError::Forbidden)));
}

#[tokio::test]
async fn languages_parses_name_code_pairs() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/settings/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "languages": [["English", "en"], ["French", "fr"]]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let languages = client.languages().await.unwrap();

    assert_eq!(languages.len(), 2);
    assert_eq!(languages[0].0, "English");
    assert_eq!(languages[0].1, "en");
}

#[tokio::test]
async fn dialects_parses_map_of_lists() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/settings/dialects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dialects": {
                "en": [["English (United States)", "en-US"], ["English (United Kingdom)", "en-GB"]]
            }
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let dialects = client.dialects().await.unwrap();

    let en = &dialects["en"];
    assert_eq!(en.len(), 2);
    assert_eq!(en[0].1, "en-US");
}

#[tokio::test]
async fn braille_tables_parses_name_id_pairs() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/settings/braille_tables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "braille_tables": [["English (U.S.) grade 2", "en-us-g2.ctb"]]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let tables = client.braille_tables().await.unwrap();

    assert_eq!(tables[0].0, "English (U.S.) grade 2");
    assert_eq!(tables[0].1, "en-us-g2.ctb");
}

#[tokio::test]
async fn voices_parses_map_of_lists() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/settings/voices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "voices": {
                "en-US": [["Jenny (Female)", "en-US-JennyNeural", true]]
            }
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let voices = client.voices().await.unwrap();

    let en_us = &voices["en-US"];
    assert_eq!(en_us[0].0, "Jenny (Female)");
    assert_eq!(en_us[0].1, "en-US-JennyNeural");
    assert!(en_us[0].2);
}
