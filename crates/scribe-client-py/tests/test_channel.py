from scribe_client import NotFoundError, PasswordRequiredError, ScribeClient, TokenSet


def valid_tokens():
    return TokenSet("at-valid")


def test_open_document_channel_start_conversion_and_events(fake_channel_server):
    def script(conn):
        join_ref, ref, topic, event, _payload = conn.recv_json()
        assert event == "phx_join"
        conn.send_json([join_ref, ref, topic, "phx_reply", {"status": "ok", "response": {}}])
        _join_ref, ref, topic, event, payload = conn.recv_json()
        assert event == "start_conversion"
        assert payload["format"] == "pdf"
        conn.send_json(
            [join_ref, ref, topic, "phx_reply", {"status": "ok", "response": {"output_id": "out-1"}}]
        )
        conn.send_json(
            [join_ref, None, topic, "status", {"format": "pdf", "stage": "convert", "progress": 0.5}]
        )
        conn.send_json(
            [join_ref, None, topic, "conversion_complete", {"format": "pdf", "output_id": "out-1"}]
        )

    server = fake_channel_server(script)
    client = ScribeClient(server.base_url, "test-client-id", valid_tokens())
    channel = client.open_document_channel("doc-1")
    try:
        output_id = channel.start_conversion("pdf")
        assert output_id == "out-1"
        status_event = channel.next_event()
        assert status_event == {
            "type": "status",
            "format": "pdf",
            "stage": "convert",
            "progress": 0.5,
        }
        complete_event = channel.next_event()
        assert complete_event == {
            "type": "conversion_complete",
            "format": "pdf",
            "output_id": "out-1",
        }
    finally:
        channel.close()
    server.join()


def test_open_document_channel_join_error_raises_not_found(fake_channel_server):
    def script(conn):
        join_ref, ref, topic, _event, _payload = conn.recv_json()
        conn.send_json(
            [join_ref, ref, topic, "phx_reply", {"status": "error", "response": {"reason": "not_found"}}]
        )

    server = fake_channel_server(script)
    client = ScribeClient(server.base_url, "test-client-id", valid_tokens())
    try:
        client.open_document_channel("missing")
        raise AssertionError("expected NotFoundError")
    except NotFoundError:
        pass
    server.join()


def test_start_conversion_sends_the_password_and_password_required_surfaces(
    fake_channel_server,
):
    def script(conn):
        join_ref, ref, topic, _event, _payload = conn.recv_json()
        conn.send_json([join_ref, ref, topic, "phx_reply", {"status": "ok", "response": {}}])
        conn.send_json([join_ref, None, topic, "password_required", {}])
        _join_ref, ref, topic, event, payload = conn.recv_json()
        assert event == "start_conversion"
        assert payload["password"] == "hunter2"
        conn.send_json(
            [join_ref, ref, topic, "phx_reply", {"status": "ok", "response": {"output_id": "out-1"}}]
        )

    server = fake_channel_server(script)
    client = ScribeClient(server.base_url, "test-client-id", valid_tokens())
    channel = client.open_document_channel("doc-1")
    try:
        assert channel.next_event() == {"type": "password_required"}
        assert channel.start_conversion("pdf", "hunter2") == "out-1"
    finally:
        channel.close()


def test_start_conversion_raises_password_required_for_a_locked_document(
    fake_channel_server,
):
    def script(conn):
        join_ref, ref, topic, _event, _payload = conn.recv_json()
        conn.send_json([join_ref, ref, topic, "phx_reply", {"status": "ok", "response": {}}])
        _join_ref, ref, topic, _event, _payload = conn.recv_json()
        conn.send_json(
            [
                join_ref,
                ref,
                topic,
                "phx_reply",
                {"status": "error", "response": {"reason": "password_required"}},
            ]
        )

    server = fake_channel_server(script)
    client = ScribeClient(server.base_url, "test-client-id", valid_tokens())
    channel = client.open_document_channel("doc-1")
    try:
        import pytest

        with pytest.raises(PasswordRequiredError):
            channel.start_conversion("pdf")
    finally:
        channel.close()
