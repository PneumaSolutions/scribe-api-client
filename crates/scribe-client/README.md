# scribe-client

A Rust client for the [Scribe for Documents](https://scribeit.io) API by Pneuma Solutions, which converts documents into accessible formats: HTML, DAISY, EPUB, MP3 audio, and Braille.

It covers signing in (OAuth 2.0 with PKCE), uploading documents, converting them, following a conversion live over a WebSocket channel, downloading the results, and the account and settings around them. The [`scribe-client`](https://pypi.org/project/scribe-client/) Python package is built on it.

## Example

```rust,no_run
use scribe_client::{reqwest, AuthClient, PkceChallenge, ScribeClient, Url};

#[tokio::main]
async fn main() -> Result<(), scribe_client::ScribeError> {
    let base_url: Url = "https://scribeit.io".parse().expect("a valid URL");
    let client_id = "your-client-id";
    let redirect_uri = "myapp://callback";
    let http = reqwest::Client::new();
    let auth = AuthClient::new(http.clone(), base_url.clone(), client_id);

    // Send the user's browser to this URL. Once they sign in, the server
    // redirects to `redirect_uri` with a `code` query parameter.
    let pkce = PkceChallenge::generate();
    println!("{}", auth.authorization_url(redirect_uri, &pkce));
    let code = "the code from the redirect";

    // Exchange the code for tokens, which `ScribeClient` then refreshes as
    // needed.
    let tokens = auth.exchange_code(redirect_uri, code, pkce.verifier()).await?;
    let client = ScribeClient::new(http, base_url, client_id, tokens);

    for document in client.list_documents().await?.documents {
        println!("{}: {}", document.id, document.title.as_deref().unwrap_or("(untitled)"));
    }
    Ok(())
}
```

`ScribeClient` and `AuthClient` take a `reqwest::Client` and a `Url`; use the `scribe_client::reqwest` and `scribe_client::Url` re-exports so the versions match.

## TLS

By default TLS is done by [rustls](https://crates.io/crates/rustls), with no system library to link against. To use the platform's own TLS library instead (Security.framework on Apple platforms, SChannel on Windows, OpenSSL elsewhere), turn off the default features and turn on `native-tls`:

```toml
scribe-client = { version = "0.1", default-features = false, features = ["native-tls"] }
```

## License

Licensed under either of the [Apache License, Version 2.0](https://github.com/PneumaSolutions/scribe-api-client/blob/main/LICENSE-APACHE) or the [MIT license](https://github.com/PneumaSolutions/scribe-api-client/blob/main/LICENSE-MIT), at your option.
