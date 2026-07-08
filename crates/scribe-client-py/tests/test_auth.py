from urllib.parse import parse_qs, urlparse
import pytest
from scribe_client import AuthClient, InvalidGrantError, PkceChallenge, ScribeApiError


def test_authorization_url_includes_pkce_challenge(mock_server):
    client = AuthClient(mock_server.base_url, "test-client-id")
    pkce = PkceChallenge()
    url = urlparse(client.authorization_url("myapp://callback", pkce))
    query = parse_qs(url.query)
    assert url.path == "/oauth/authorize"
    assert query["code_challenge"] == [pkce.challenge]
    assert query["code_challenge_method"] == ["S256"]
    assert query["redirect_uri"] == ["myapp://callback"]
    assert query["client_id"] == ["test-client-id"]


def test_exchange_code_returns_token_set_on_success(mock_server):
    mock_server.add_json_route(
        "POST",
        "/oauth/token",
        200,
        {"access_token": "at-123", "refresh_token": "rt-456", "expires_in": 3600},
    )
    client = AuthClient(mock_server.base_url, "test-client-id")
    tokens = client.exchange_code("myapp://callback", "auth-code", "the-verifier")
    assert tokens.access_token == "at-123"
    assert tokens.refresh_token == "rt-456"
    assert tokens.expires_at is not None
    [request] = mock_server.recorded_requests
    assert b"code_verifier=the-verifier" in request["body"]


def test_exchange_code_raises_invalid_grant_error(mock_server):
    mock_server.add_json_route(
        "POST",
        "/oauth/token",
        400,
        {"error": "invalid_grant", "error_description": "PKCE verification failed"},
    )
    client = AuthClient(mock_server.base_url, "test-client-id")
    with pytest.raises(InvalidGrantError):
        client.exchange_code("myapp://callback", "auth-code", "wrong-verifier")


def test_exchange_code_raises_generic_api_error_for_unrecognized_error(mock_server):
    mock_server.add_json_route(
        "POST", "/oauth/token", 400, {"error": "unsupported_grant_type"}
    )
    client = AuthClient(mock_server.base_url, "test-client-id")
    with pytest.raises(ScribeApiError):
        client.exchange_code("myapp://callback", "auth-code", "the-verifier")


def test_refresh_returns_new_token_set(mock_server):
    mock_server.add_json_route(
        "POST",
        "/oauth/token",
        200,
        {"access_token": "at-new", "refresh_token": "rt-new", "expires_in": 3600},
    )
    client = AuthClient(mock_server.base_url, "test-client-id")
    tokens = client.refresh("rt-old")
    assert tokens.access_token == "at-new"
