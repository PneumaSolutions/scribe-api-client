use scribe_client::{ScribeClient, ScribeError, SettingsUpdate, TokenSet};
use time::OffsetDateTime;
use url::Url;
use wiremock::{
    matchers::{body_string_contains, method, path},
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
            "error": {"code": "not_found", "message": "We couldn't find that."}
        })))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let result = client.get_settings("missing").await;
    assert!(matches!(result, Err(ScribeError::NotFound { .. })));
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
            "error": {"code": "forbidden", "message": "You don't have permission to do that."}
        })))
        .mount(&server)
        .await;

    let client = client_for(&server, valid_tokens());
    let result = client
        .update_settings("doc-1", &scribe_client::SettingsUpdate::default())
        .await;

    assert!(matches!(result, Err(ScribeError::Forbidden { .. })));
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

#[tokio::test]
async fn default_settings_reads_the_account_level_defaults() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/document_settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings_json()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let settings = client.default_settings().await.unwrap();
    assert_eq!(settings.language.as_deref(), Some("en"));
    assert_eq!(settings.braille_translation_table, "en-us-g2.ctb");
}

/// The account endpoint carries no document id. Getting that wrong would send
/// the defaults screen's edits to a document called "document_settings".
#[tokio::test]
async fn update_default_settings_patches_the_account_path() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/document_settings"))
        .and(body_string_contains("\"tts_rate\":1.5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings_json()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let update = SettingsUpdate {
        tts_rate: Some(1.5),
        ..Default::default()
    };
    client.update_default_settings(&update).await.unwrap();
}

/// Same string-on-write rule as the per-document endpoint: the columns are
/// strings even though reads come back decoded.
#[tokio::test]
async fn update_default_settings_sends_voices_as_a_json_string() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/document_settings"))
        // The value is a JSON *string*, so the quotes inside it arrive escaped.
        .and(body_string_contains(
            r#""voices":"{\"en-US\":\"en-US-AriaNeural\"}""#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings_json()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let update = SettingsUpdate {
        voices: Some(serde_json::Value::String(
            r#"{"en-US":"en-US-AriaNeural"}"#.to_owned(),
        )),
        ..Default::default()
    };
    client.update_default_settings(&update).await.unwrap();
}

#[tokio::test]
async fn voice_sample_downloads_the_mp3_without_a_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sounds/tts-samples/en-US-JennyNeural.mp3"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ID3mp3".to_vec()))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let data = client.voice_sample("en-US-JennyNeural").await.unwrap();
    assert_eq!(data, b"ID3mp3");
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn voice_sample_maps_a_missing_sample_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("<html>Not Found</html>"))
        .mount(&server)
        .await;
    let client = client_for(&server, valid_tokens());
    let err = client.voice_sample("xx-XX-NobodyNeural").await.unwrap_err();
    assert!(matches!(err, ScribeError::NotFound { .. }), "{err:?}");
}

#[tokio::test]
async fn voice_sample_url_names_the_file_and_refuses_other_paths() {
    let server = MockServer::start().await;
    let client = client_for(&server, valid_tokens());
    assert_eq!(
        client.voice_sample_url("en-US-JennyNeural").unwrap(),
        format!("{}/sounds/tts-samples/en-US-JennyNeural.mp3", server.uri())
    );
    for bad in ["", "../secret", "a/b", "a.mp3", "a?b"] {
        assert!(
            matches!(
                client.voice_sample_url(bad),
                Err(ScribeError::NotFound { .. })
            ),
            "{bad}"
        );
        assert!(client.voice_sample(bad).await.is_err(), "{bad}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn voices_and_dialects_objects_go_out_as_json_strings() {
    let update = SettingsUpdate {
        voices: Some(serde_json::json!({"en-US": "en-US-JennyNeural"})),
        dialects: Some(serde_json::Value::String(r#"{"en":"en-US"}"#.into())),
        ..Default::default()
    };
    let body = serde_json::to_value(&update).unwrap();
    assert_eq!(body["voices"], r#"{"en-US":"en-US-JennyNeural"}"#);
    assert_eq!(body["dialects"], r#"{"en":"en-US"}"#);
    assert!(serde_json::to_value(SettingsUpdate::default())
        .unwrap()
        .get("voices")
        .is_none());
}
