# scribe-client (Python)

Python bindings for the Scribe document conversion API. This package is a
thin, synchronous wrapper (via [PyO3](https://pyo3.rs)) around the Rust
`scribe-client` crate in this workspace — the same core that backs the
UniFFI (iOS/Android) bindings, so behavior is consistent across languages.

- **Synchronous by design.** Every method blocks until the request
  completes; there's no `asyncio` API. Under the hood each call runs on a
  shared Tokio runtime with the GIL released, so other Python threads keep
  running while a request is in flight.
- **Auto-refreshing tokens.** `ScribeClient` holds a `TokenSet` and
  refreshes it automatically when it's missing or about to expire, and
  again if a request unexpectedly comes back `401`.

## Installing

This package isn't published to PyPI yet. Build and install it from
source with [maturin](https://www.maturin.rs/):

```bash
cd crates/scribe-client-py
uv venv                      # or: python -m venv .venv
source .venv/bin/activate
uv pip install maturin
maturin develop              # builds the extension and installs it into the venv
```

## Authenticating

The server uses OAuth 2.0 Authorization Code + PKCE. There's no password
or client-credentials grant, so a human has to approve access once via a
browser; after that, `ScribeClient` handles refreshing the token for you.

```python
from scribe_client import AuthClient, PkceChallenge, ScribeClient

auth = AuthClient(base_url, client_id)
pkce = PkceChallenge()

# Send the user to this URL. After they log in and approve, they're
# redirected to redirect_uri with a `code` query parameter.
url = auth.authorization_url(redirect_uri, pkce)

# Exchange the code (and the verifier from the *same* PkceChallenge) for a
# token set.
tokens = auth.exchange_code(redirect_uri, code, pkce.verifier)

client = ScribeClient(base_url, client_id, tokens)
```

`tokens.access_token`, `tokens.refresh_token`, and `tokens.expires_at`
(Unix seconds, or `None`) are readable if you want to persist them
between runs — reconstruct a `TokenSet` later with
`TokenSet(access_token, refresh_token, expires_at)`.

See `examples/pkce_flow.py` for a runnable end-to-end version of this,
including creating a document and converting it.

## Documents

```python
# From a local file...
document_id = client.create_document_from_file("report.docx", data)
# ...or have the server fetch it from a URL.
document_id = client.create_document_from_url("https://example.com/report.pdf")
```

Creating a document automatically starts an `html_stream` conversion (a
fast preview), so the caller has something to show immediately even if
the account is out of page credits for a full conversion.

```python
documents = client.list_documents()
for doc in documents:
    # doc.title and doc.page_count are None until the server has
    # determined them (e.g. briefly, for a URL-sourced document).
    print(doc.id, doc.title, doc.page_count, doc.inserted_at)
    for output in doc.outputs:
        print(" ", output.format, output.stage, output.progress)

client.delete_document(document_id)
```

`list_outputs(document_id)` returns just the `Output` rows for one
document (what `list_documents()` embeds per-document, without the
document metadata); `get_settings`/`update_settings` read and partially
update a document's conversion settings (language, TTS voice, Braille
table, and so on — see `Settings`'s attributes for the full set).

```python
settings = client.get_settings(document_id)
client.update_settings(document_id, {"large_print": True})
```

`update_settings` takes a plain dict; only the keys you pass are changed
server-side.

## Converting to another format

Creating a document only starts the `html_stream` preview. Converting to
any other format — `pdf`, `epub`, `daisy`, `docx`, `brf`, `mp3`,
`offline_html`, `mobi` — happens over a **real-time channel**, not a REST
call, so the server can guarantee it's subscribed to progress on whatever
it just started converting.

```python
channel = client.open_document_channel(document_id)
try:
    output_id = channel.start_conversion("pdf")

    while True:
        event = channel.next_event()

        if event["type"] == "status":
            print(f"{event['format']}: {event['stage']} {event['progress']:.0%}")
        elif event["type"] == "conversion_complete" and event["format"] == "pdf":
            data = client.download_output(document_id, "pdf")
            break
        elif event["type"] == "error":
            raise RuntimeError(event["reason"])
finally:
    channel.close()
```

`start_conversion` is idempotent — calling it again for a format that's
already converting or complete just returns that output's existing id
without starting a duplicate conversion.

`next_event()` blocks until the next event arrives and returns it as a
dict tagged by `event["type"]`:

| `type`                | other keys                       | meaning                                    |
| ---------------------- | --------------------------------- | ------------------------------------------- |
| `"status"`              | `format`, `stage`, `progress`      | a conversion's stage or progress changed     |
| `"chunk"`               | `content`                          | a chunk of streamed HTML (`html_stream` only) |
| `"conversion_complete"` | `format`, `output_id`              | a format finished converting                 |
| `"error"`               | `reason`                           | an out-of-band error unrelated to a specific call |

`stage` is one of `"queue"`, `"start"`, `"convert"`,
`"add_image_descriptions"`, or `"complete"`.

Joining the channel also delivers events for any formats that were
already converting (or complete) when you joined — including incomplete
*parent* formats. (Most formats are rendered from the `html` output, and
`daisy` is additionally assembled from `mp3`; the server subscribes you
to those automatically when they're not finished yet.) Filter on
`event["format"]` if you only care about one format at a time, as in the
example above.

Always `close()` the channel when you're done with it (a `try`/`finally`
is the easiest way, as above) — it holds an open WebSocket connection.

## Errors

All exceptions are subclasses of `ScribeApiError`:

| Exception                      | Raised when                                                              |
| ------------------------------- | -------------------------------------------------------------------------- |
| `NotFoundError`                 | the document (or channel) doesn't exist                                    |
| `ForbiddenError`                | the document exists but isn't owned by the current user                   |
| `ConversionNotCompleteError`    | `download_output` was called before that format finished converting       |
| `ConversionInProgressError`     | `start_conversion` was called while a different conversion is already running |
| `RateLimitedError`              | `start_conversion` was called too many times too quickly                   |
| `NeedsPurchaseError`            | the account doesn't have enough page credits for the requested conversion   |
| `InvalidGrantError`             | an OAuth code, refresh token, or PKCE verifier didn't check out            |
| `ScribeApiError` (base)         | anything else the server rejected, or a connection-level failure          |

```python
from scribe_client import NotFoundError

try:
    client.delete_document(document_id)
except NotFoundError:
    print("already gone")
```

## Running the tests

```bash
cd crates/scribe-client-py
maturin develop
pytest
```

Tests don't hit a real server: `tests/conftest.py` runs a small
dependency-free HTTP server (stdlib `http.server`) for REST calls, and a
hand-rolled WebSocket server (stdlib `socket`, doing the RFC 6455
handshake itself) for exercising `DocumentChannel`.
