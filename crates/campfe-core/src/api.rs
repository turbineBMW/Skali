//! The HTTP client for one Basecamp account: the base URL, the bearer token,
//! the mandatory User-Agent, and the API's manners -- geared pagination by
//! `Link: rel="next"`, 429 with `Retry-After`, 5xx with a backoff.
//!
//! Runs on the tokio runtime; nothing here touches GTK.

use crate::USER_AGENT;
use crate::error::{ApiError, Result};
use reqwest::header::{HeaderMap, HeaderValue, LINK, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// How long one request may take end to end.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Retries for a transient failure (5xx, a dropped connection) before
/// giving up, with the backoff doubling from this.
const RETRIES: u32 = 3;
const BACKOFF: Duration = Duration::from_millis(500);
/// A 429 whose Retry-After is longer than this is reported, not waited out.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(15);

/// The access token, shared with whoever refreshes it.
pub type SharedToken = Arc<RwLock<String>>;

/// One page of a listing: its items and where the next page is, if any.
#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
    /// `X-Total-Count`, when the listing reports it.
    pub total: Option<u64>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    /// `https://3.basecampapi.com/<account id>`, no trailing slash.
    base_url: String,
    token: SharedToken,
    /// Whether this account may be written to. With `CAMPFE_WRITE_ACCOUNTS`
    /// set in the environment, only the account ids it lists (comma
    /// separated) may be; a development guard so a test post can't land in
    /// a real project. Unset, every account may.
    writes_allowed: bool,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// A reqwest client with the headers Basecamp insists on.
pub fn http_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/json"),
    );
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .timeout(TIMEOUT)
        .build()?)
}

impl Client {
    pub fn new(base_url: impl Into<String>, token: SharedToken) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let writes_allowed =
            writes_allowed_for(&base_url, std::env::var("CAMPFE_WRITE_ACCOUNTS").ok());
        if !writes_allowed {
            tracing::warn!("writes to {base_url} are off (CAMPFE_WRITE_ACCOUNTS)");
        }
        Ok(Self {
            http: http_client()?,
            base_url,
            token,
            writes_allowed,
        })
    }

    pub fn writes_allowed(&self) -> bool {
        self.writes_allowed
    }

    fn check_write(&self) -> Result<()> {
        if self.writes_allowed {
            Ok(())
        } else {
            Err(ApiError::ReadOnly)
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// A path like `/projects.json` becomes an account URL; a full URL (a
    /// `Link` header's, a recording's `url`) is used as is.
    pub fn url(&self, path_or_url: &str) -> String {
        if path_or_url.starts_with("https://") || path_or_url.starts_with("http://") {
            path_or_url.to_string()
        } else {
            format!("{}/{}", self.base_url, path_or_url.trim_start_matches('/'))
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.send(|| self.http.get(self.url(path))).await?;
        Ok(response.json().await?)
    }

    /// Any call, the body as text: for `campfe api`.
    pub async fn call_raw(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<String> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| ApiError::Auth(format!("not an HTTP method: {method}")))?;
        if method != reqwest::Method::GET {
            self.check_write()?;
        }
        let response = self
            .send(|| {
                let request = self.http.request(method.clone(), self.url(path));
                match body {
                    Some(body) => request.json(body),
                    None => request.header("Content-Length", "0"),
                }
            })
            .await?;
        Ok(response.text().await?)
    }

    /// One page of a listing. `path` is the first page's path, or the
    /// `next` of the page before.
    pub async fn get_page<T: DeserializeOwned>(&self, path: &str) -> Result<Page<T>> {
        let response = self.send(|| self.http.get(self.url(path))).await?;
        let next = next_link(response.headers());
        let total = response
            .headers()
            .get("X-Total-Count")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        let items = response.json().await?;
        Ok(Page { items, next, total })
    }

    /// Every page of a listing, in order. For listings known to be small;
    /// anything a user scrolls should page with `get_page`.
    pub async fn get_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut next = Some(path.to_string());
        while let Some(url) = next {
            let page = self.get_page::<T>(&url).await?;
            items.extend(page.items);
            next = page.next;
        }
        Ok(items)
    }

    pub async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.check_write()?;
        let response = self
            .send(|| self.http.post(self.url(path)).json(body))
            .await?;
        Ok(response.json().await?)
    }

    pub async fn put<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.check_write()?;
        let response = self
            .send(|| self.http.put(self.url(path)).json(body))
            .await?;
        Ok(response.json().await?)
    }

    /// Bytes behind an API URL that needs the sign-in: an attachment's
    /// `download_url`.
    pub async fn get_bytes(&self, path_or_url: &str) -> Result<Vec<u8>> {
        let response = self.send(|| self.http.get(self.url(path_or_url))).await?;
        Ok(response.bytes().await?.to_vec())
    }

    /// Upload a file for use in rich text: the reply's `attachable_sgid`
    /// goes into a `<bc-attachment>`.
    pub async fn upload_attachment(
        &self,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<serde_json::Value> {
        self.check_write()?;
        let path = format!(
            "/attachments.json?name={}",
            percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC)
        );
        let content_type = content_type.to_string();
        let response = self
            .send(|| {
                self.http
                    .post(self.url(&path))
                    .header("Content-Type", content_type.clone())
                    .header("Content-Length", bytes.len())
                    .body(bytes.clone())
            })
            .await?;
        Ok(response.json().await?)
    }

    /// A write whose reply doesn't matter (completions, pins, deletes).
    pub async fn post_empty(&self, path: &str) -> Result<()> {
        self.check_write()?;
        self.send(|| self.http.post(self.url(path)).header("Content-Length", "0"))
            .await?;
        Ok(())
    }

    /// A PUT with a body whose reply is empty (marking Hey! items read).
    pub async fn put_no_reply<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> Result<()> {
        self.check_write()?;
        self.send(|| self.http.put(self.url(path)).json(body))
            .await?;
        Ok(())
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.check_write()?;
        self.send(|| self.http.delete(self.url(path))).await?;
        Ok(())
    }

    /// Send with the current token, retrying what's worth retrying. The
    /// request is rebuilt for every try since a builder is consumed by send.
    async fn send(&self, build: impl Fn() -> reqwest::RequestBuilder) -> Result<Response> {
        let mut delay = BACKOFF;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.token.read().await.clone();
            let result = build().bearer_auth(&token).send().await;
            let retry_in = match result {
                Ok(response) => match classify(response).await {
                    Ok(response) => return Ok(response),
                    Err(ApiError::RateLimited(wait)) if wait <= MAX_RETRY_AFTER => wait,
                    Err(error) if error.is_transient() && attempt <= RETRIES => delay,
                    Err(error) => return Err(error),
                },
                Err(error) if attempt <= RETRIES && (error.is_connect() || error.is_timeout()) => {
                    tracing::debug!("request failed ({error}), retrying");
                    delay
                }
                Err(error) => return Err(error.into()),
            };
            tokio::time::sleep(retry_in).await;
            delay *= 2;
        }
    }
}

/// Turn a non-success status into the error that names it.
async fn classify(response: Response) -> Result<Response> {
    let status = response.status();
    if status.is_success() || status == StatusCode::NOT_MODIFIED {
        return Ok(response);
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let wait = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(10));
        return Err(ApiError::RateLimited(wait));
    }
    let message = response.text().await.unwrap_or_default();
    let message = message.chars().take(300).collect::<String>();
    Err(ApiError::Http {
        status: status.as_u16(),
        message: if message.is_empty() {
            status.canonical_reason().unwrap_or("error").to_string()
        } else {
            message
        },
    })
}

/// Bytes from a URL that needs no sign-in (avatars on Basecamp's CDN).
/// No bearer token: the host isn't the API's.
pub async fn fetch_public_bytes(url: &str) -> Result<Vec<u8>> {
    let response = http_client()?.get(url).send().await?;
    let response = classify(response).await?;
    Ok(response.bytes().await?.to_vec())
}

/// The development guard: with an allowlist set, only its account ids may
/// be written to. The account id is the last path segment of the base URL.
fn writes_allowed_for(base_url: &str, allowlist: Option<String>) -> bool {
    let Some(allowlist) = allowlist else {
        return true;
    };
    let account_id = base_url.rsplit('/').next().unwrap_or("");
    allowlist
        .split(',')
        .map(str::trim)
        .any(|id| !id.is_empty() && id == account_id)
}

/// The `rel="next"` URL of a `Link` header, if there is one.
pub fn next_link(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(LINK)?.to_str().ok()?;
    parse_next_link(header)
}

fn parse_next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (url, params) = part.trim().split_once(';')?;
        let is_next = params
            .split(';')
            .any(|param| param.trim().replace('"', "") == "rel=next");
        is_next.then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_link_is_found_among_others() {
        let header = r#"<https://3.basecampapi.com/1/projects.json?page=1>; rel="first", <https://3.basecampapi.com/1/projects.json?page=4>; rel="next""#;
        assert_eq!(
            parse_next_link(header).as_deref(),
            Some("https://3.basecampapi.com/1/projects.json?page=4")
        );
        assert_eq!(parse_next_link(r#"<x>; rel="prev""#), None);
    }

    #[test]
    fn write_guard_follows_the_allowlist() {
        let base = "https://3.basecampapi.com/1000001";
        assert!(writes_allowed_for(base, None));
        assert!(writes_allowed_for(base, Some("1000001".into())));
        assert!(writes_allowed_for(base, Some(" 1, 1000001 ".into())));
        assert!(!writes_allowed_for(base, Some("1".into())));
        assert!(!writes_allowed_for(base, Some(String::new())));
    }

    #[test]
    fn paths_and_urls_resolve() {
        let client = Client::new(
            "https://3.basecampapi.com/1000001/",
            Arc::new(RwLock::new(String::new())),
        )
        .unwrap();
        assert_eq!(
            client.url("/projects.json"),
            "https://3.basecampapi.com/1000001/projects.json"
        );
        assert_eq!(
            client.url("projects.json"),
            "https://3.basecampapi.com/1000001/projects.json"
        );
        assert_eq!(client.url("https://x/y.json"), "https://x/y.json");
    }
}
