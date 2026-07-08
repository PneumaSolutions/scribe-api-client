"""A minimal threaded HTTP server for exercising the PyO3 bindings against
real HTTP requests, mirroring the wiremock-based Rust integration tests in
crates/scribe-client/tests/.

We can't mock the Rust HTTP client from Python, so tests need something on
the wire; stdlib http.server keeps this dependency-free.
"""

import base64
import hashlib
import http.server
import json
import socket
import struct
import threading

import pytest

_WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


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

    def do_DELETE(self):
        self._dispatch("DELETE")


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

    def add_empty_route(self, method, path, status):
        def handler(req, _req_body):
            req.send_response(status)
            req.send_header("Content-Length", "0")
            req.end_headers()

        self.routes[(method, path)] = handler

    def shutdown(self):
        self._httpd.shutdown()
        self._thread.join()


@pytest.fixture
def mock_server():
    server = MockServer()
    yield server
    server.shutdown()


class _FakeWsConn:
    """One accepted, already-upgraded WebSocket connection, server side.
    Only implements enough of RFC 6455 to exchange the small JSON text
    frames the Phoenix channel protocol uses in tests."""

    def __init__(self, sock):
        self._sock = sock
        self._buf = b""

    def _recv_exact(self, n):
        while len(self._buf) < n:
            chunk = self._sock.recv(4096)
            if not chunk:
                raise ConnectionError("socket closed while reading a frame")
            self._buf += chunk
        data, self._buf = self._buf[:n], self._buf[n:]
        return data

    def recv_json(self):
        first2 = self._recv_exact(2)
        length = first2[1] & 0x7F
        masked = first2[1] & 0x80
        if length == 126:
            length = struct.unpack(">H", self._recv_exact(2))[0]
        elif length == 127:
            length = struct.unpack(">Q", self._recv_exact(8))[0]
        mask_key = self._recv_exact(4) if masked else b""
        payload = self._recv_exact(length)
        if masked:
            payload = bytes(b ^ mask_key[i % 4] for i, b in enumerate(payload))
        return json.loads(payload.decode())

    def send_json(self, value):
        payload = json.dumps(value).encode()
        header = bytes([0x81])  # FIN + text frame opcode
        length = len(payload)
        if length < 126:
            header += bytes([length])
        elif length < 65536:
            header += bytes([126]) + struct.pack(">H", length)
        else:
            header += bytes([127]) + struct.pack(">Q", length)
        self._sock.sendall(header + payload)


def _ws_handshake(sock):
    data = b""
    while b"\r\n\r\n" not in data:
        data += sock.recv(4096)
    headers = data.split(b"\r\n\r\n", 1)[0]

    key = None
    for line in headers.split(b"\r\n")[1:]:
        name, _, value = line.partition(b":")
        if name.strip().lower() == b"sec-websocket-key":
            key = value.strip().decode()

    accept = base64.b64encode(hashlib.sha1((key + _WS_GUID).encode()).digest()).decode()
    response = (
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
    )
    sock.sendall(response.encode())


class FakeChannelServer:
    """A `/socket/websocket` stand-in: accepts exactly one connection and
    hands it to `script(conn)` on a background thread."""

    def __init__(self, script):
        self._sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._sock.bind(("127.0.0.1", 0))
        self._sock.listen(1)
        self._thread = threading.Thread(target=self._serve, args=(script,), daemon=True)
        self._thread.start()

    def _serve(self, script):
        conn, _addr = self._sock.accept()
        try:
            _ws_handshake(conn)
            script(_FakeWsConn(conn))
        finally:
            conn.close()

    @property
    def base_url(self):
        _host, port = self._sock.getsockname()
        return f"http://127.0.0.1:{port}"

    def join(self, timeout=5):
        self._thread.join(timeout)


@pytest.fixture
def fake_channel_server():
    servers = []

    def start(script):
        server = FakeChannelServer(script)
        servers.append(server)
        return server

    yield start
