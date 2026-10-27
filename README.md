# Scribe API client

[![CI](https://github.com/PneumaSolutions/scribe-api-client/actions/workflows/ci.yml/badge.svg)](https://github.com/PneumaSolutions/scribe-api-client/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/scribe-client.svg)](https://crates.io/crates/scribe-client) [![PyPI](https://img.shields.io/pypi/v/scribe-client.svg)](https://pypi.org/project/scribe-client/) [![docs.rs](https://img.shields.io/docsrs/scribe-client)](https://docs.rs/scribe-client)

Clients for the [Scribe for Documents](https://scribeit.io) API by Pneuma Solutions, which converts documents into accessible formats: HTML, DAISY, EPUB, MP3 audio, and Braille.

The client signs in with OAuth 2.0 and PKCE, uploads documents, converts them, follows a conversion live over a WebSocket channel, and downloads the results. It is written once in Rust, and the other languages wrap that same code, so every client behaves the same way.

## Packages

| Package | For | Install |
| --- | --- | --- |
| [`scribe-client`](crates/scribe-client) | Rust | `cargo add scribe-client` |
| [`scribe-client`](crates/scribe-client-py) | Python 3.9 and later | `pip install scribe-client` |
| [`scribe-client-ffi`](crates/scribe-client-ffi) | Swift and Kotlin, through [UniFFI](https://mozilla.github.io/uniffi-rs/) | Not published; build it from this repository |

Each package's README has examples: [Rust](crates/scribe-client/README.md) and [Python](crates/scribe-client-py/README.md). The Rust API reference is on [docs.rs](https://docs.rs/scribe-client).

## Choosing a redirect URI

When you register an app, you give one or more redirect URIs. After the user signs in and approves access, the server sends their browser to one of these, with the authorization code in a `code` query parameter. The server never displays the code itself, so your app has to receive that redirect. The client libraries don't do this for you; they build the authorization URL and exchange the code, and leave the browser and the redirect to your app.

- **Desktop apps, command-line tools and scripts:** use a loopback address with a fixed port, such as `http://127.0.0.1:8765/callback`, and run a small HTTP server on it while the user signs in. The redirect must match a registered URI exactly, port included. The Python example, [`pkce_flow.py`](crates/scribe-client-py/examples/pkce_flow.py), does this.
- **Mobile apps:** use a custom URL scheme that your app handles, such as `myapp://callback`, through `ASWebAuthenticationSession` on iOS or a Custom Tab on Android.
- **Web apps:** use an HTTPS URL on your own site.

If nothing receives the redirect, the browser shows an error such as "can't connect to the server at 127.0.0.1" after the user approves access. Sign-in still worked, and the code is in the address bar, so in testing you can copy it from there. It works only once and expires after a few minutes.

## Development

You need a stable Rust toolchain. Formatting uses nightly rustfmt, because `rustfmt.toml` sets options that are still unstable.

```bash
cargo +nightly fmt --all
cargo clippy --workspace --exclude scribe-client-py --all-targets
cargo test --workspace --exclude scribe-client-py
```

The Python package is built with [maturin](https://www.maturin.rs/) and tested with pytest:

```bash
cd crates/scribe-client-py
uv venv
uv pip install maturin pytest
uv run --no-project maturin develop
uv run --no-project pytest
```

No test talks to a real server. The Rust tests use [wiremock](https://crates.io/crates/wiremock) for HTTP and a small local WebSocket server for the channel, and the Python tests run a small local HTTP and WebSocket server.

## Releasing

All three packages share the version in the workspace `Cargo.toml`. To release, change that version, commit it to `main`, and push a tag with the same number, such as `v0.2.0`. The [release workflow](.github/workflows/release.yml) builds the Python wheels for Linux, macOS and Windows, and then publishes the crate to crates.io and the wheels to PyPI. Both registries use trusted publishing, so the workflow needs no tokens.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option.
