//! Interactive demo of the OAuth 2.0 Authorization Code + PKCE flow against
//! a real Scribe server.
//!
//! Run with:
//!
//!     cargo run --example pkce_flow -- <base_url> <client_id> <redirect_uri>
//!
//! It prints a URL to open in a browser. After logging in and approving
//! access, the browser will be redirected to `redirect_uri` with a `code`
//! query parameter; paste that value back into the prompt to complete the
//! exchange.
use std::io::{self, Write};
use scribe_client::{AuthClient, PkceChallenge};

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
    println!("You'll be redirected to {redirect_uri}?code=... ; paste the code below.");
    print!("code: ");
    io::stdout().flush().expect("failed to flush stdout");
    let mut code = String::new();
    io::stdin()
        .read_line(&mut code)
        .expect("failed to read code from stdin");
    let code = code.trim();
    match auth
        .exchange_code(&redirect_uri, code, pkce.verifier())
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
