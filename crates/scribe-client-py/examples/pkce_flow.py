#!/usr/bin/env python3
"""Interactive demo of the OAuth 2.0 Authorization Code + PKCE flow against a
real Scribe server, followed by a full document conversion round trip
(upload, start a conversion over the document channel, watch it finish,
download).

Analogous to crates/scribe-client/examples/pkce_flow.rs, but goes one step
further into the document endpoints since exercising those against a real
server is the point of this script.

Configure via environment variables:

    SCRIBE_BASE_URL       e.g. https://scribeit.io
    SCRIBE_CLIENT_ID      OAuth client_id registered on the server
    SCRIBE_REDIRECT_URI   must match one registered for that client_id;
                           a loopback URI such as
                           http://127.0.0.1:8765/callback lets the script
                           catch the redirect itself
    SCRIBE_DOCUMENT_PATH  local file to upload (mutually exclusive with
                           SCRIBE_DOCUMENT_URL)
    SCRIBE_DOCUMENT_URL   have the server fetch the document instead
    SCRIBE_OUTPUT_FORMAT  output format to convert to and download
                           (default: pdf)

If SCRIBE_REDIRECT_URI is an http:// URI on 127.0.0.1, [::1] or localhost,
the script listens on that address, opens the authorization URL in a
browser, and reads the code from the redirect. Any other redirect URI (a
custom scheme like myapp://callback, say) can't be caught by a script, so it
asks you to paste the code from the address bar instead.

Run with:

    uv run --project crates/scribe-client-py examples/pkce_flow.py
"""

import os
import socket
import sys
import webbrowser
from html import escape
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from scribe_client import AuthClient, PkceChallenge, ScribeClient


def env_or_exit(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        print(f"missing required environment variable: {name}", file=sys.stderr)
        sys.exit(1)
    return value


LOOPBACK_HOSTS = {"127.0.0.1", "::1", "localhost"}


def is_loopback(redirect_uri: str) -> bool:
    parts = urlsplit(redirect_uri)
    return parts.scheme == "http" and parts.hostname in LOOPBACK_HOSTS


class IPv6HTTPServer(HTTPServer):
    address_family = socket.AF_INET6


def wait_for_redirect(redirect_uri: str, authorize_url: str) -> str:
    """Opens `authorize_url` in a browser, serves `redirect_uri` until the
    browser is redirected to it, and returns the authorization code from
    the query string.

    Listens on the port in `redirect_uri` itself, since the server only
    redirects to a URI that exactly matches a registered one."""
    parts = urlsplit(redirect_uri)
    expected_path = parts.path or "/"
    result = {}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            request = urlsplit(self.path)
            if request.path != expected_path:
                # Favicon requests and the like.
                self.send_error(404)
                return

            query = parse_qs(request.query)
            if "code" in query:
                result["code"] = query["code"][0]
                self.respond(200, "Signed in", "You can close this tab and return to the script.")
            else:
                error = query.get("error", ["no code in redirect"])[0]
                description = query.get("error_description", [""])[0]
                result["error"] = f"{error}: {description}" if description else error
                self.respond(400, "Sign-in failed", result["error"])

        def respond(self, status: int, title: str, message: str):
            body = (
                f"<!doctype html><html lang=en><meta charset=utf-8><title>{escape(title)}</title>"
                f"<h1>{escape(title)}</h1><p>{escape(message)}</p></html>"
            ).encode()
            self.send_response(status)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format, *args):
            pass

    server_class = IPv6HTTPServer if parts.hostname == "::1" else HTTPServer
    with server_class((parts.hostname, parts.port or 80), Handler) as server:
        # Listening before the browser opens, so a fast redirect can't
        # arrive before anything is there to receive it.
        webbrowser.open(authorize_url)
        while not result:
            server.handle_request()

    if "error" in result:
        print(f"authorization failed: {result['error']}", file=sys.stderr)
        sys.exit(1)
    return result["code"]


def do_pkce_login(base_url: str, client_id: str, redirect_uri: str):
    auth = AuthClient(base_url, client_id)
    pkce = PkceChallenge()

    authorize_url = auth.authorization_url(redirect_uri, pkce)

    if is_loopback(redirect_uri):
        print("Opening this URL in your browser; log in and approve access:\n")
        print(f"  {authorize_url}\n")
        print(f"Waiting for the redirect to {redirect_uri} ...")
        code = wait_for_redirect(redirect_uri, authorize_url)
    else:
        print("Open this URL in a browser, log in, and approve access:\n")
        print(f"  {authorize_url}\n")
        print(f"The browser will then go to {redirect_uri}?code=... . Nothing here can")
        print("receive that, so expect an error page. Copy the code parameter from the")
        print("address bar and paste it below. A code works once, and only for a few")
        print("minutes.")
        code = input("code: ").strip()

    return auth.exchange_code(redirect_uri, code, pkce.verifier)


def create_document(client: ScribeClient) -> str:
    document_path = os.environ.get("SCRIBE_DOCUMENT_PATH")
    document_url = os.environ.get("SCRIBE_DOCUMENT_URL")

    if document_path and document_url:
        print("set only one of SCRIBE_DOCUMENT_PATH or SCRIBE_DOCUMENT_URL", file=sys.stderr)
        sys.exit(1)

    if document_url:
        print(f"Creating document from URL: {document_url}")
        return client.create_document_from_url(document_url)

    if document_path:
        path = Path(document_path)
        print(f"Uploading {path}")
        return client.create_document_from_file(path.name, path.read_bytes())

    print("set SCRIBE_DOCUMENT_PATH or SCRIBE_DOCUMENT_URL to choose what to upload", file=sys.stderr)
    sys.exit(1)


def convert_and_wait(client: ScribeClient, document_id: str, output_format: str) -> bytes:
    """Starts converting to `output_format` over the document channel and
    watches for it to finish, printing progress along the way."""
    channel = client.open_document_channel(document_id)

    try:
        channel.start_conversion(output_format)

        while True:
            event = channel.next_event()

            if event["type"] == "status" and event["format"] == output_format:
                print(f"  {output_format}: stage={event['stage']} progress={event['progress']:.0%}")
            elif event["type"] == "conversion_complete" and event["format"] == output_format:
                return client.download_output(document_id, output_format)
            elif event["type"] == "error":
                print(f"conversion error: {event['reason']}", file=sys.stderr)
                sys.exit(1)
            # Events for other formats (e.g. an incomplete parent format
            # like html) are pushed too but aren't interesting here.
    finally:
        channel.close()


def main():
    base_url = env_or_exit("SCRIBE_BASE_URL")
    client_id = env_or_exit("SCRIBE_CLIENT_ID")
    redirect_uri = env_or_exit("SCRIBE_REDIRECT_URI")
    output_format = os.environ.get("SCRIBE_OUTPUT_FORMAT", "pdf")

    tokens = do_pkce_login(base_url, client_id, redirect_uri)
    print("\nToken exchange succeeded:")
    print(f"  access_token:  {tokens.access_token}")
    print(f"  refresh_token: {tokens.refresh_token}")
    print(f"  expires_at:    {tokens.expires_at}")

    client = ScribeClient(base_url, client_id, tokens)

    document_id = create_document(client)
    print(f"\nDocument created: {document_id}")

    print(f"\nConverting to {output_format!r}...")
    data = convert_and_wait(client, document_id, output_format)

    out_path = Path(f"{document_id}.{output_format}")
    out_path.write_bytes(data)
    print(f"\nDownloaded {len(data)} bytes to {out_path}")


if __name__ == "__main__":
    main()
