import pytest

from scribe_client import ForbiddenError, NotFoundError, ScribeClient, TokenSet


def valid_tokens():
    return TokenSet("at-valid")


def settings_body(**overrides):
    body = {
        "language": "en",
        "dialects": {},
        "voices": {},
        "tts_gender": None,
        "tts_rate": 1.0,
        "braille_translation_table": "en-us-g2.ctb",
        "braille_cells_per_line": 40,
        "braille_split_into_pages": True,
        "braille_lines_per_page": 25,
        "large_print": False,
        "add_image_descriptions": True,
        "math": False,
        "notify_when_complete": False,
    }
    body.update(overrides)
    return body


def test_get_settings_returns_current_document_settings(mock_server):
    mock_server.add_json_route(
        "GET", "/api/documents/doc-1/settings", 200, settings_body()
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    settings = client.get_settings("doc-1")
    assert settings.language == "en"
    assert settings.braille_translation_table == "en-us-g2.ctb"
    assert settings.add_image_descriptions is True
    assert settings.large_print is False
    assert settings.dialects == {}
    assert settings.tts_rate == 1.0


def test_get_settings_raises_not_found_error(mock_server):
    mock_server.add_json_route(
        "GET", "/api/documents/missing/settings", 404, {"error": "not_found"}
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    with pytest.raises(NotFoundError):
        client.get_settings("missing")


def test_update_settings_sends_only_provided_fields(mock_server):
    mock_server.add_json_route(
        "PATCH",
        "/api/documents/doc-1/settings",
        200,
        settings_body(large_print=True, braille_translation_table="en-gb-g1.utb"),
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    settings = client.update_settings(
        "doc-1", {"large_print": True, "braille_translation_table": "en-gb-g1.utb"}
    )
    assert settings.large_print is True
    assert settings.braille_translation_table == "en-gb-g1.utb"
    [request] = mock_server.recorded_requests
    import json
    body = json.loads(request["body"])
    assert body["settings"] == {
        "large_print": True,
        "braille_translation_table": "en-gb-g1.utb",
    }


def test_update_settings_raises_forbidden_error(mock_server):
    mock_server.add_json_route(
        "PATCH", "/api/documents/doc-1/settings", 403, {"error": "forbidden"}
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    with pytest.raises(ForbiddenError):
        client.update_settings("doc-1", {"large_print": True})


def test_languages_returns_name_code_pairs(mock_server):
    mock_server.add_json_route(
        "GET",
        "/api/settings/languages",
        200,
        {"languages": [["English", "en"], ["French", "fr"]]},
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    languages = client.languages()
    assert languages == [("English", "en"), ("French", "fr")]


def test_dialects_returns_map_of_lists(mock_server):
    mock_server.add_json_route(
        "GET",
        "/api/settings/dialects",
        200,
        {"dialects": {"en": [["English (United States)", "en-US"]]}},
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    dialects = client.dialects()
    assert dialects == {"en": [("English (United States)", "en-US")]}


def test_braille_tables_returns_name_id_pairs(mock_server):
    mock_server.add_json_route(
        "GET",
        "/api/settings/braille_tables",
        200,
        {"braille_tables": [["English (U.S.) grade 2", "en-us-g2.ctb"]]},
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    tables = client.braille_tables()
    assert tables == [("English (U.S.) grade 2", "en-us-g2.ctb")]


def test_voices_returns_map_of_lists(mock_server):
    mock_server.add_json_route(
        "GET",
        "/api/settings/voices",
        200,
        {"voices": {"en-US": [["Jenny (Female)", "en-US-JennyNeural", True]]}},
    )
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    voices = client.voices()
    assert voices == {"en-US": [("Jenny (Female)", "en-US-JennyNeural", True)]}
