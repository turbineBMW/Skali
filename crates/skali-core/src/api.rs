//! The HTTP client for one Basecamp account: the base URL, the bearer token,
//! the mandatory User-Agent, and the API's manners -- geared pagination by
//! `Link: rel="next"`, 429 with `Retry-After`, 5xx with a backoff, and
//! conditional GETs: a JSON GET sends the `ETag` it last saw as
//! `If-None-Match`, and a 304 answers with the body kept from then.
//!
//! Runs on the tokio runtime; nothing here touches GTK.

use crate::USER_AGENT;
use crate::error::{ApiError, Result};
use reqwest::header::{ETAG, HeaderMap, HeaderValue, IF_NONE_MATCH, LINK, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
/// The ETag cache keeps at most this many responses, and this many bytes of
/// them, dropping the least recently used first.
const CACHE_ENTRIES: usize = 512;
const CACHE_BYTES: usize = 32 * 1024 * 1024;

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
    /// Whether this account may be written to. With `SKALI_WRITE_ACCOUNTS`
    /// set in the environment, only the account ids it lists (comma
    /// separated) may be; a development guard so a test post can't land in
    /// a real project. Unset, every account may.
    writes_allowed: bool,
    /// Responses by URL with their ETags, shared by every clone of this
    /// account's client. In memory only: the session's pages and polls.
    cache: Arc<Mutex<EtagCache>>,
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
            writes_allowed_for(&base_url, std::env::var("SKALI_WRITE_ACCOUNTS").ok());
        if !writes_allowed {
            tracing::warn!("writes to {base_url} are off (SKALI_WRITE_ACCOUNTS)");
        }
        Ok(Self {
            http: http_client()?,
            base_url,
            token,
            writes_allowed,
            cache: Arc::default(),
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
        let cached = self.get_cached(path).await?;
        Ok(serde_json::from_slice(&cached.body)?)
    }

    /// A GET by way of the ETag cache: with an ETag on hand the request is
    /// conditional, and a 304 gives back what was kept with it.
    async fn get_cached(&self, path: &str) -> Result<Cached> {
        let url = self.url(path);
        let known = self.cache.lock().unwrap().get(&url);
        let response = self
            .send(|| {
                let request = self.http.get(&url);
                match &known {
                    Some(known) => request.header(IF_NONE_MATCH, known.etag.as_str()),
                    None => request,
                }
            })
            .await?;
        if response.status() == StatusCode::NOT_MODIFIED
            && let Some(known) = known
        {
            tracing::trace!("not modified: {url}");
            return Ok(known);
        }
        let headers = response.headers();
        let etag = headers
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let next = next_link(headers);
        let total = headers
            .get("X-Total-Count")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        let cached = Cached {
            etag: etag.unwrap_or_default(),
            body: response.bytes().await?.to_vec().into(),
            next,
            total,
        };
        let mut cache = self.cache.lock().unwrap();
        if cached.etag.is_empty() {
            cache.remove(&url);
        } else {
            cache.insert(url, cached.clone());
        }
        Ok(cached)
    }

    /// Any call, the body as text: for `skali api`.
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
        let cached = self.get_cached(path).await?;
        Ok(Page {
            items: serde_json::from_slice(&cached.body)?,
            next: cached.next,
            total: cached.total,
        })
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

/// A response as the ETag cache keeps it: the body and the headers a
/// listing needs, since a 304 may not repeat them.
#[derive(Debug, Clone)]
struct Cached {
    etag: String,
    body: Arc<[u8]>,
    next: Option<String>,
    total: Option<u64>,
}

/// Responses by URL, bounded by count and bytes, least recently used out.
#[derive(Debug, Default)]
struct EtagCache {
    entries: HashMap<String, (Cached, u64)>,
    bytes: usize,
    /// Bumped on every use; an entry's stamp says when it was last used.
    clock: u64,
}

impl EtagCache {
    fn get(&mut self, url: &str) -> Option<Cached> {
        self.clock += 1;
        let (cached, used) = self.entries.get_mut(url)?;
        *used = self.clock;
        Some(cached.clone())
    }

    fn insert(&mut self, url: String, cached: Cached) {
        self.remove(&url);
        if cached.body.len() > CACHE_BYTES {
            return;
        }
        self.clock += 1;
        self.bytes += cached.body.len();
        self.entries.insert(url, (cached, self.clock));
        while self.entries.len() > CACHE_ENTRIES || self.bytes > CACHE_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(url, _)| url.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
    }

    fn remove(&mut self, url: &str) {
        if let Some((cached, _)) = self.entries.remove(url) {
            self.bytes -= cached.body.len();
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

/// The path of a search: the words, optionally one kind of recording
/// (a `key` of `/searches/metadata.json`), best match or newest first.
pub fn search_path(query: &str, type_name: Option<&str>, newest_first: bool) -> String {
    let encode = |s: &str| {
        percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
    };
    let mut path = format!("/search.json?q={}", encode(query.trim()));
    if let Some(type_name) = type_name {
        path.push_str(&format!("&type_names%5B%5D={}", encode(type_name)));
    }
    if newest_first {
        path.push_str("&sort=recency");
    }
    path
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
    fn search_paths_encode_their_words() {
        assert_eq!(
            search_path(" fish & chips ", None, false),
            "/search.json?q=fish%20%26%20chips"
        );
        assert_eq!(
            search_path("x", Some("Kanban::Card"), true),
            "/search.json?q=x&type_names%5B%5D=Kanban%3A%3ACard&sort=recency"
        );
    }

    fn cached(etag: &str, len: usize) -> Cached {
        Cached {
            etag: etag.into(),
            body: vec![0; len].into(),
            next: None,
            total: None,
        }
    }

    #[test]
    fn etag_cache_replaces_and_counts_bytes() {
        let mut cache = EtagCache::default();
        cache.insert("a".into(), cached("1", 10));
        cache.insert("a".into(), cached("2", 4));
        assert_eq!(cache.get("a").unwrap().etag, "2");
        assert_eq!(cache.bytes, 4);
        cache.remove("a");
        assert!(cache.get("a").is_none());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn etag_cache_drops_the_least_recently_used() {
        let mut cache = EtagCache::default();
        for i in 0..CACHE_ENTRIES {
            cache.insert(i.to_string(), cached("e", 1));
        }
        // Touch the oldest, so the second oldest is the one to go.
        cache.get("0");
        cache.insert("new".into(), cached("e", 1));
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert!(cache.get("0").is_some());
        assert!(cache.get("1").is_none());

        cache.insert("big".into(), cached("e", CACHE_BYTES - 10));
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(cache.get("big").is_some());
        cache.insert("huge".into(), cached("e", CACHE_BYTES + 1));
        assert!(cache.get("huge").is_none());
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
