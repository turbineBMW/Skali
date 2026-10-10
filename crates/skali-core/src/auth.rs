//! Signing in through Launchpad (launchpad.37signals.com), the OAuth
//! server for every 37signals product: the authorization-code flow with
//! the browser sent back to a loopback listener, token exchange and
//! refresh, and `authorization.json`, which says whose token it is and
//! which Basecamp accounts it opens.
//!
//! Launchpad needs the client secret at exchange and refresh, so a desktop
//! app embeds its own, as 37signals' own CLI does. Access tokens last two
//! weeks; the refresh token lasts until revoked.

use crate::USER_AGENT;
use crate::error::{ApiError, Result};
use crate::models::Authorization;
use chrono::{DateTime, Duration, Utc};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const LAUNCHPAD: &str = "https://launchpad.37signals.com";
/// Basecamp's own authorization document: who a personal access token
/// (from `https://app.basecamp.com/my/access_tokens`) belongs to.
pub const BASECAMP_AUTHORIZATION: &str = "https://3.basecampapi.com/authorization.json";

/// The integration registered at launchpad.37signals.com/integrations.
#[derive(Clone)]
pub struct OAuthApp {
    pub client_id: String,
    pub client_secret: String,
    /// Must match a redirect URI the integration was registered with,
    /// exactly: `http://127.0.0.1:8976/callback`.
    pub redirect_uri: String,
}

impl std::fmt::Debug for OAuthApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthApp")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

impl OAuthApp {
    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }

    /// The port the redirect URI names, for the loopback listener.
    pub fn redirect_port(&self) -> Option<u16> {
        url::Url::parse(&self.redirect_uri).ok()?.port()
    }

    /// Where to send the browser. `state` comes back on the redirect and
    /// is checked against this one, so a stray callback can't sign us in.
    pub fn authorize_url(&self, state: &str) -> String {
        format!(
            "{LAUNCHPAD}/authorization/new?response_type=code&client_id={}&redirect_uri={}&state={}",
            encode(&self.client_id),
            encode(&self.redirect_uri),
            encode(state)
        )
    }
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// What a sign-in leaves behind. Its Debug hides the tokens: never print one.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("expires_at", &self.expires_at)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

impl TokenSet {
    /// A pasted personal access token: no refresh token, so it lasts until
    /// its expiry (if Basecamp reports one) and is then pasted again.
    pub fn personal(access_token: String, expires_at: Option<DateTime<Utc>>) -> Self {
        Self {
            access_token,
            refresh_token: None,
            expires_at,
        }
    }

    /// Refresh ahead of the expiry rather than at it, so a request started
    /// just before never carries a token that dies in flight. A token with
    /// nothing to refresh with never asks.
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        self.refresh_token.is_some()
            && self
                .expires_at
                .is_some_and(|expires| expires - Duration::minutes(5) <= now)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl TokenResponse {
    fn into_tokens(self, now: DateTime<Utc>, previous_refresh: Option<String>) -> TokenSet {
        TokenSet {
            access_token: self.access_token,
            // A refresh reply may omit the refresh token; the old one stays good.
            refresh_token: self.refresh_token.or(previous_refresh),
            expires_at: self
                .expires_in
                .map(|seconds| now + Duration::seconds(seconds)),
        }
    }
}

/// A plain client for Launchpad itself (no bearer token, no account).
pub fn launchpad_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// Trade the code the browser brought back for tokens.
pub async fn exchange_code(http: &reqwest::Client, app: &OAuthApp, code: &str) -> Result<TokenSet> {
    let response = http
        .post(format!("{LAUNCHPAD}/authorization/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", app.client_id.as_str()),
            ("client_secret", app.client_secret.as_str()),
            ("redirect_uri", app.redirect_uri.as_str()),
            ("code", code),
        ])
        .send()
        .await?;
    token_reply(response, None).await
}

/// A fresh access token from the refresh token.
pub async fn refresh(
    http: &reqwest::Client,
    app: &OAuthApp,
    tokens: &TokenSet,
) -> Result<TokenSet> {
    let Some(refresh_token) = tokens.refresh_token.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    let response = http
        .post(format!("{LAUNCHPAD}/authorization/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", app.client_id.as_str()),
            ("client_secret", app.client_secret.as_str()),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;
    token_reply(response, tokens.refresh_token.clone()).await
}

async fn token_reply(
    response: reqwest::Response,
    previous_refresh: Option<String>,
) -> Result<TokenSet> {
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::BAD_REQUEST {
        let body = response.text().await.unwrap_or_default();
        tracing::warn!("Launchpad refused the token request ({status}): {body}");
        return Err(ApiError::Unauthorized);
    }
    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Http {
            status: status.as_u16(),
            message,
        });
    }
    let reply: TokenResponse = response.json().await?;
    Ok(reply.into_tokens(Utc::now(), previous_refresh))
}

/// Whose token this is and the accounts it opens. The first call after
/// signing in; the `href` of a `bc3` account is the API base URL.
pub async fn authorization(http: &reqwest::Client, access_token: &str) -> Result<Authorization> {
    authorization_at(
        http,
        &format!("{LAUNCHPAD}/authorization.json"),
        access_token,
    )
    .await
}

/// `authorization` for a personal access token, which Basecamp itself
/// vouches for rather than Launchpad. The same document, with the token's
/// expiry when it has one.
pub async fn token_authorization(
    http: &reqwest::Client,
    access_token: &str,
) -> Result<Authorization> {
    authorization_at(http, BASECAMP_AUTHORIZATION, access_token).await
}

async fn authorization_at(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<Authorization> {
    let response = http.get(url).bearer_auth(access_token).send().await?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if !response.status().is_success() {
        return Err(ApiError::Http {
            status: response.status().as_u16(),
            message: response.text().await.unwrap_or_default(),
        });
    }
    let body = response.text().await?;
    serde_json::from_str(&body).map_err(|error| {
        // No secret in this document: an identity and account names.
        tracing::debug!("authorization.json did not parse ({error}): {body}");
        ApiError::Decode(error)
    })
}

/// The page the browser shows once it has been sent back to us.
const CALLBACK_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Skali</title>\
<style>body{font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0;background:#1e1e1e;color:#eee}\
div{text-align:center}h1{font-weight:500}</style></head><body><div><h1>Signed in to Skali</h1><p>You can close this tab and go back to the app.</p></div></body></html>";
const ERROR_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Skali</title></head><body style=\"font-family:system-ui,sans-serif\"><h1>Sign-in failed</h1><p>Skali didn't get an authorization code. Go back to the app and try again.</p></body></html>";

/// Listen on the loopback port for the browser's redirect and return the
/// authorization code it carries. Only a request whose `state` matches
/// counts; anything else gets an error page and the wait goes on.
pub async fn await_callback(port: u16, expected_state: &str) -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| {
            ApiError::Auth(format!("could not listen on 127.0.0.1:{port}: {error}"))
        })?;
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut buffer = vec![0u8; 8192];
        let read = socket.read(&mut buffer).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
        match code_from_request(&request, expected_state) {
            Ok(code) => {
                respond(&mut socket, "200 OK", CALLBACK_PAGE).await;
                return Ok(code);
            }
            Err(CallbackError::Denied(reason)) => {
                respond(&mut socket, "200 OK", ERROR_PAGE).await;
                return Err(ApiError::Auth(reason));
            }
            Err(CallbackError::Ignore) => {
                respond(&mut socket, "404 Not Found", ERROR_PAGE).await;
            }
        }
    }
}

async fn respond(socket: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(reply.as_bytes()).await;
    let _ = socket.shutdown().await;
}

enum CallbackError {
    /// Not our redirect: a favicon request, a wrong state.
    Ignore,
    /// Our redirect, but Launchpad reported an error (the user said no).
    Denied(String),
}

/// The `code` of `GET /callback?code=...&state=...`, when the state is ours.
fn code_from_request(
    request: &str,
    expected_state: &str,
) -> std::result::Result<String, CallbackError> {
    let request_line = request.lines().next().unwrap_or("");
    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or(CallbackError::Ignore)?;
    let parsed =
        url::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| CallbackError::Ignore)?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(CallbackError::Ignore);
    }
    if let Some(error) = error {
        return Err(CallbackError::Denied(error));
    }
    code.filter(|code| !code.is_empty())
        .ok_or(CallbackError::Ignore)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> OAuthApp {
        OAuthApp {
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "http://127.0.0.1:8976/callback".into(),
        }
    }

    #[test]
    fn authorize_url_carries_everything_encoded() {
        let url = app().authorize_url("st ate");
        assert!(url.starts_with(
            "https://launchpad.37signals.com/authorization/new?response_type=code&client_id=id"
        ));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127%2E0%2E0%2E1%3A8976%2Fcallback"));
        assert!(url.ends_with("&state=st%20ate"));
        assert_eq!(app().redirect_port(), Some(8976));
    }

    #[test]
    fn callback_request_yields_the_code_for_our_state() {
        let request = "GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            code_from_request(request, "xyz").ok().as_deref(),
            Some("abc")
        );
        assert!(matches!(
            code_from_request(request, "other"),
            Err(CallbackError::Ignore)
        ));
        assert!(matches!(
            code_from_request("GET /favicon.ico HTTP/1.1\r\n", "xyz"),
            Err(CallbackError::Ignore)
        ));
        let denied = "GET /callback?error=access_denied&state=xyz HTTP/1.1\r\n";
        assert!(
            matches!(code_from_request(denied, "xyz"), Err(CallbackError::Denied(e)) if e == "access_denied")
        );
    }

    #[test]
    fn refresh_is_due_five_minutes_early_and_keeps_the_old_refresh_token() {
        let now = Utc::now();
        let tokens = TokenResponse {
            access_token: "a".into(),
            refresh_token: None,
            expires_in: Some(240),
        }
        .into_tokens(now, Some("old".into()));
        assert_eq!(tokens.refresh_token.as_deref(), Some("old"));
        assert!(tokens.needs_refresh(now));
        let fresh = TokenSet {
            access_token: "SECRET-TOKEN".into(),
            refresh_token: None,
            expires_at: Some(now + Duration::days(14)),
        };
        assert!(!fresh.needs_refresh(now));
        assert!(!format!("{fresh:?}").contains("SECRET-TOKEN"));
    }

    #[test]
    fn a_personal_token_never_asks_for_a_refresh() {
        let now = Utc::now();
        let expiring = TokenSet::personal("t".into(), Some(now + Duration::minutes(1)));
        assert!(expiring.refresh_token.is_none());
        assert!(!expiring.needs_refresh(now));
        assert!(!TokenSet::personal("t".into(), None).needs_refresh(now));
    }
}
