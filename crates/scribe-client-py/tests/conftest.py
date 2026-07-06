"""A minimal threaded HTTP server for exercising the PyO3 bindings against
real HTTP requests, mirroring the wiremock-based Rust integration tests in
crates/scribe-client/tests/.

We can't mock the Rust HTTP client from Python, so tests need something on
the wire; stdlib http.server keeps this dependency-free.
"""

import http.server
import json
import threading

import pytest


class _Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):  # noqa: A002 - matches base class signature
        pass

    def _dispatch(self, method):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length else b""

        self.server.recorded_requests.append(
            {
                "method": method,
                "path": self.path,
                "headers": dict(self.headers),
                "body": body,
            }
        )

        handler = self.server.routes.get((method, self.path))
        if handler is None:
            self.send_response(404)
            self.end_headers()
            return
        handler(self, body)

    def do_GET(self):
        self._dispatch("GET")

    def do_POST(self):
        self._dispatch("POST")

    def do_PATCH(self):
        self._dispatch("PATCH")


class MockServer:
    def __init__(self):
        self.routes = {}
        self.recorded_requests = []
        self._httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        self._httpd.routes = self.routes
        self._httpd.recorded_requests = self.recorded_requests
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)
        self._thread.start()

    @property
    def base_url(self):
        host, port = self._httpd.server_address
        return f"http://{host}:{port}"

    def add_json_route(self, method, path, status, body):
        payload = json.dumps(body).encode()

        def handler(req, _req_body):
            req.send_response(status)
            req.send_header("Content-Type", "application/json")
            req.send_header("Content-Length", str(len(payload)))
            req.end_headers()
            req.wfile.write(payload)

        self.routes[(method, path)] = handler

    def add_bytes_route(self, method, path, status, data, content_type="application/octet-stream"):
        def handler(req, _req_body):
            req.send_response(status)
            req.send_header("Content-Type", content_type)
            req.send_header("Content-Length", str(len(data)))
            req.end_headers()
            req.wfile.write(data)

        self.routes[(method, path)] = handler

    def shutdown(self):
        self._httpd.shutdown()
        self._thread.join()


@pytest.fixture
def mock_server():
    server = MockServer()
    yield server
    server.shutdown()
