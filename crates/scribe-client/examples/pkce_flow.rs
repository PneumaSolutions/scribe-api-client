//! Interactive demo of the OAuth 2.0 Authorization Code + PKCE flow against
//! a real Scribe server.
//!
//! Run with:
//!
//!     cargo run --example pkce_flow -- <base_url> <client_id> <redirect_uri>
//!
//! It prints a URL to open in a browser. After logging in and approving
//! access, the browser will be redirected to `redirect_uri` with `code` and
//! `state` query parameters; paste that whole address back into the prompt
//! to complete the exchange.
use scribe_client::{AuthClient, PkceChallenge, Url};
use std::io::{self, Write};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: pkce_flow <base_url> <client_id> <redirect_uri>";
    let base_url = args.next().expect(usage);
    let client_id = args.next().expect(usage);
    let redirect_uri = args.next().expect(usage);
    let base_url = base_url.parse().expect("base_url must be a valid URL");
    let http = reqwest::Client::new();
    let auth = AuthClient::new(http, base_url, client_id);
    let pkce = PkceChallenge::generate();
    let authorize_url = auth.authorization_url(&redirect_uri, &pkce);
    println!("Open this URL in a browser, log in, and approve access:\n");
    println!("  {authorize_url}\n");
    println!("You'll be redirected to {redirect_uri}?code=... ; paste that whole address below.");
    print!("address: ");
    io::stdout().flush().expect("failed to flush stdout");
    let mut address = String::new();
    io::stdin()
        .read_line(&mut address)
        .expect("failed to read the address from stdin");
    let address: Url = address.trim().parse().expect("the address must be a URL");
    let param = |name: &str| {
        address
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    // Refuse a redirect that didn't come from this sign-in.
    if param("state").as_deref() != Some(pkce.state()) {
        eprintln!("\nThat address's state doesn't match this sign-in.");
        std::process::exit(1);
    }
    let code = param("code").expect("the address has no code");
    match auth
        .exchange_code(&redirect_uri, &code, pkce.verifier())
        .await
    {
        Ok(tokens) => {
            println!("\nToken exchange succeeded:");
            println!("  access_token:  {}", tokens.access_token);
            println!("  refresh_token: {:?}", tokens.refresh_token);
            println!("  expires_at:    {:?}", tokens.expires_at);
        }
        Err(err) => {
            eprintln!("\nToken exchange failed: {err}");
            std::process::exit(1);
        }
    }
}
