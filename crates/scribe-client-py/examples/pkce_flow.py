#!/usr/bin/env python3
"""Interactive demo of the OAuth 2.0 Authorization Code + PKCE flow against a
real Scribe server, followed by a full document conversion round trip
(upload, start a conversion over the document channel, watch it finish,
download).

Analogous to crates/scribe-client/examples/pkce_flow.rs, but goes one step
further into the document endpoints since exercising those against a real
server is the point of this script.

Configure via environment variables:

    SCRIBE_BASE_URL       e.g. http://localhost:8083
    SCRIBE_CLIENT_ID      OAuth client_id registered on the server
    SCRIBE_REDIRECT_URI   must match one registered for that client_id
    SCRIBE_DOCUMENT_PATH  local file to upload (mutually exclusive with
                           SCRIBE_DOCUMENT_URL)
    SCRIBE_DOCUMENT_URL   have the server fetch the document instead
    SCRIBE_OUTPUT_FORMAT  output format to convert to and download
                           (default: pdf)

Run with:

    uv run --project crates/scribe-client-py examples/pkce_flow.py
"""

import os
import sys
from pathlib import Path

from scribe_client import AuthClient, PkceChallenge, ScribeClient


def env_or_exit(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        print(f"missing required environment variable: {name}", file=sys.stderr)
        sys.exit(1)
    return value


def do_pkce_login(base_url: str, client_id: str, redirect_uri: str):
    auth = AuthClient(base_url, client_id)
    pkce = PkceChallenge()

    authorize_url = auth.authorization_url(redirect_uri, pkce)

    print("Open this URL in a browser, log in, and approve access:\n")
    print(f"  {authorize_url}\n")
    print(f"You'll be redirected to {redirect_uri}?code=... ; paste the code below.")
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
