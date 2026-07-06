import pytest

from scribe_client import (
    ConversionNotCompleteError,
    ForbiddenError,
    NotFoundError,
    ScribeClient,
    TokenSet,
)


def valid_tokens():
    return TokenSet("at-valid")


def test_create_document_from_file_returns_document_id(mock_server):
    mock_server.add_json_route(
        "POST", "/api/documents", 200, {"document_id": "doc-1"}
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    document_id = client.create_document_from_file("report.docx", b"pretend docx bytes")

    assert document_id == "doc-1"

    [request] = mock_server.recorded_requests
    assert request["headers"]["authorization"] == "Bearer at-valid"
    assert b"report.docx" in request["body"]


def test_create_document_from_url_returns_document_id(mock_server):
    mock_server.add_json_route(
        "POST", "/api/documents", 200, {"document_id": "doc-2"}
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    document_id = client.create_document_from_url("https://example.com/report.pdf")

    assert document_id == "doc-2"


def test_list_outputs_parses_in_progress_and_complete_rows(mock_server):
    mock_server.add_json_route(
        "GET",
        "/api/documents/doc-1/outputs",
        200,
        {
            "outputs": [
                {
                    "format": "html_stream",
                    "stage": "convert",
                    "progress": 0.5,
                    "estimated_time_remaining": 10,
                    "is_preview": True,
                },
                {
                    "format": "pdf",
                    "stage": "complete",
                    "progress": 1.0,
                    "estimated_time_remaining": None,
                    "is_preview": False,
                },
            ]
        },
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    outputs = client.list_outputs("doc-1")

    assert len(outputs) == 2
    assert outputs[0].format == "html_stream"
    assert outputs[0].stage == "convert"
    assert outputs[0].progress == 0.5
    assert outputs[0].estimated_time_remaining == 10
    assert outputs[0].is_preview is True
    assert outputs[1].format == "pdf"
    assert outputs[1].stage == "complete"
    assert outputs[1].estimated_time_remaining is None


def test_list_outputs_raises_not_found_error(mock_server):
    mock_server.add_json_route(
        "GET", "/api/documents/missing/outputs", 404, {"error": "not_found"}
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())

    with pytest.raises(NotFoundError):
        client.list_outputs("missing")


def test_list_outputs_raises_forbidden_error(mock_server):
    mock_server.add_json_route(
        "GET", "/api/documents/other-users-doc/outputs", 403, {"error": "forbidden"}
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())

    with pytest.raises(ForbiddenError):
        client.list_outputs("other-users-doc")


def test_download_output_returns_bytes_when_complete(mock_server):
    mock_server.add_bytes_route(
        "GET", "/api/documents/doc-1/outputs/pdf/download", 200, b"%PDF-1.4 fake"
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())
    data = client.download_output("doc-1", "pdf")

    assert data == b"%PDF-1.4 fake"


def test_download_output_raises_conversion_not_complete_error(mock_server):
    mock_server.add_json_route(
        "GET",
        "/api/documents/doc-1/outputs/pdf/download",
        409,
        {"error": "conversion_not_complete"},
    )

    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())

    with pytest.raises(ConversionNotCompleteError):
        client.download_output("doc-1", "pdf")


def test_download_output_rejects_unrecognized_format(mock_server):
    client = ScribeClient(mock_server.base_url, "test-client-id", valid_tokens())

    with pytest.raises(ValueError):
        client.download_output("doc-1", "not_a_format")


def test_a_401_triggers_refresh_and_retries_once(mock_server):
    call_count = {"outputs": 0}

    def outputs_handler(req, _body):
        call_count["outputs"] += 1
        if req.headers["authorization"] == "Bearer at-stale":
            req.send_response(401)
            req.end_headers()
        else:
            payload = b'{"outputs": []}'
            req.send_response(200)
            req.send_header("Content-Type", "application/json")
            req.send_header("Content-Length", str(len(payload)))
            req.end_headers()
            req.wfile.write(payload)

    mock_server.routes[("GET", "/api/documents/doc-1/outputs")] = outputs_handler
    mock_server.add_json_route(
        "POST",
        "/oauth/token",
        200,
        {"access_token": "at-fresh", "refresh_token": "rt-fresh", "expires_in": 3600},
    )

    stale_tokens = TokenSet("at-stale", "rt-stale")
    client = ScribeClient(mock_server.base_url, "test-client-id", stale_tokens)
    outputs = client.list_outputs("doc-1")

    assert outputs == []
    assert call_count["outputs"] == 2
