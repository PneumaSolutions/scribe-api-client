//! Guards the wire format of the TTS voice/dialect fields.
//!
//! The server's `document_settings.dialects` and `.voices` columns are strings
//! holding JSON, but `GET` returns them already decoded. Writing them back as
//! an object fails the Ecto cast, so these pin the string-on-write shape.

use scribe_client_core::SettingsUpdate as CoreSettingsUpdate;
use scribe_client_ffi::SettingsUpdate;

fn body(update: SettingsUpdate) -> serde_json::Value {
    serde_json::to_value(CoreSettingsUpdate::from(update)).expect("update should serialize")
}

#[test]
fn dialects_and_voices_go_out_as_json_strings() {
    let update = SettingsUpdate {
        dialects_json: Some(r#"{"en":"en-US"}"#.to_owned()),
        voices_json: Some(r#"{"en-US":"en-US-AriaNeural"}"#.to_owned()),
        ..Default::default()
    };

    let body = body(update);
    // A string, not an object: the column is `:string` on the server.
    assert_eq!(body["dialects"], serde_json::json!(r#"{"en":"en-US"}"#));
    assert_eq!(
        body["voices"],
        serde_json::json!(r#"{"en-US":"en-US-AriaNeural"}"#)
    );
}

#[test]
fn untouched_voice_fields_are_left_out_of_the_request() {
    let body = body(SettingsUpdate {
        tts_rate: Some(1.5),
        ..Default::default()
    });

    assert_eq!(body["tts_rate"], serde_json::json!(1.5));
    // `skip_serializing_if` keeps a partial update partial, so a screen that
    // only changed the rate cannot blank out someone's chosen voice.
    assert!(body.get("dialects").is_none());
    assert!(body.get("voices").is_none());
}
