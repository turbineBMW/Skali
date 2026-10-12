//! GIF search over KLIPY's API (https://docs.klipy.com), ported from Bubo.
//! Tenor's API closed on 2026-06-30; KLIPY is what's left that's free.
//!
//! Each user brings their own free API key from partner.klipy.com, set in
//! Preferences: a key can't ship in the source, and a test key is capped at
//! 100 searches an hour, which one person fits but a shared key wouldn't.
//! The GIFs you sent are remembered locally (`meta`), never reported.

use crate::db::Database;
use std::collections::HashMap;

const API: &str = "https://api.klipy.com/api/v1";
/// The largest GIF sent; a bigger rendition is passed over for a smaller one.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// KLIPY allows 8–50 a page; the picker grid shows three columns of 15.
const PER_PAGE: u32 = 45;
/// How many recently sent GIFs the picker remembers.
const HISTORY_LEN: usize = 45;
/// The `meta` key the recently sent GIFs are kept under, as JSON.
const HISTORY_KEY: &str = "gif-history";

#[derive(Debug, thiserror::Error)]
pub enum GifError {
    #[error("no KLIPY API key")]
    NoKey,
    #[error("KLIPY's hourly limit was reached")]
    RateLimited,
    /// KLIPY's own words, e.g. an invalid key.
    #[error("KLIPY: {0}")]
    Klipy(String),
    #[error("the GIF is larger than {} MB", MAX_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("the file is not a GIF")]
    NotAGif,
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Gif {
    /// The animated GIF that gets sent.
    pub url: String,
    /// A small animated GIF for the picker grid.
    pub preview: String,
    /// A still of the preview, shown while it loads; empty if KLIPY had none.
    #[serde(default)]
    pub still: String,
}

#[derive(serde::Deserialize)]
struct Response {
    #[serde(default)]
    result: bool,
    data: Option<Data>,
    errors: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct Data {
    data: Vec<Item>,
}

#[derive(serde::Deserialize)]
struct Item {
    #[serde(default, rename = "type")]
    kind: String,
    /// Size (`hd`, `md`, `sm`, `xs`) → format (`gif`, `jpg`, …) → file.
    #[serde(default)]
    file: HashMap<String, HashMap<String, File>>,
}

#[derive(serde::Deserialize)]
struct File {
    url: String,
    #[serde(default)]
    size: u64,
}

impl Item {
    fn file(&self, size: &str, format: &str) -> Option<&File> {
        self.file
            .get(size)?
            .get(format)
            .filter(|f| f.url.starts_with("https://"))
    }

    /// The biggest rendition still small enough to send, a small animated
    /// preview (`sm` is ~220px, sharp in a 112px tile on HiDPI) and its
    /// still. Ads (`type: "ad"`) are skipped.
    fn to_gif(&self) -> Option<Gif> {
        if self.kind == "ad" {
            return None;
        }
        let url = ["hd", "md", "sm", "xs"]
            .iter()
            .filter_map(|s| self.file(s, "gif"))
            .find(|f| f.size as usize <= MAX_BYTES)?
            .url
            .clone();
        let preview = ["sm", "xs"]
            .iter()
            .find_map(|s| self.file(s, "gif"))
            .map_or_else(|| url.clone(), |f| f.url.clone());
        let still = ["sm", "xs"]
            .iter()
            .find_map(|s| self.file(s, "jpg"))
            .map(|f| f.url.clone())
            .unwrap_or_default();
        Some(Gif {
            url,
            preview,
            still,
        })
    }
}

/// KLIPY's `{"errors": {"message": ["…"]}}`, flattened to one line.
fn error_message(errors: &serde_json::Value) -> Option<String> {
    let messages: Vec<&str> = match errors {
        serde_json::Value::Object(map) => map
            .values()
            .flat_map(|v| {
                v.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str())
                    .chain(v.as_str())
            })
            .collect(),
        serde_json::Value::String(s) => vec![s.as_str()],
        _ => vec![],
    };
    (!messages.is_empty()).then(|| messages.join(" "))
}

/// The GIFs a search finds, best first; `page` is zero-based.
pub async fn search(api_key: &str, query: &str, page: u32) -> Result<Vec<Gif>, GifError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(vec![]);
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(GifError::NoKey);
    }
    let response = crate::web()
        .get(format!("{API}/{api_key}/gifs/search"))
        .query(&[
            ("q", query),
            ("page", &(page + 1).to_string()),
            ("per_page", &PER_PAGE.to_string()),
            ("format_filter", "gif,jpg"),
        ])
        .send()
        .await
        // The key is in the URL, and an error would print it.
        .map_err(reqwest::Error::without_url)?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(GifError::RateLimited);
    }
    let body: Response = match response.json().await {
        Ok(body) => body,
        Err(_) => return Err(GifError::Klipy(format!("HTTP {status}"))),
    };
    if !status.is_success() || !body.result {
        let message = body
            .errors
            .as_ref()
            .and_then(error_message)
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(GifError::Klipy(message));
    }
    Ok(body
        .data
        .map(|d| d.data.iter().filter_map(Item::to_gif).collect())
        .unwrap_or_default())
}

/// A GIF's bytes, checked to be one and not too big to send.
pub async fn download(url: &str) -> Result<Vec<u8>, GifError> {
    let response = crate::web().get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|len| len as usize > MAX_BYTES)
    {
        return Err(GifError::TooLarge);
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_BYTES {
        return Err(GifError::TooLarge);
    }
    if !bytes.starts_with(b"GIF8") {
        return Err(GifError::NotAGif);
    }
    Ok(bytes.to_vec())
}

/// A grid tile's preview or still (any image format).
pub async fn thumbnail(url: &str) -> Result<Vec<u8>, GifError> {
    Ok(crate::web()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

/// The GIFs sent from this computer, newest first.
pub fn history(db: &Database) -> Vec<Gif> {
    db.meta(HISTORY_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Put a sent GIF at the front of the history.
pub fn remember(db: &Database, gif: &Gif) {
    let mut list = history(db);
    push_recent(&mut list, gif);
    let json = serde_json::to_string(&list).expect("strings serialize");
    if let Err(error) = db.set_meta(HISTORY_KEY, &json) {
        tracing::warn!("could not save the GIF history: {error}");
    }
}

/// Move `gif` to the front, dropping an older copy and the oldest past the cap.
fn push_recent(list: &mut Vec<Gif>, gif: &Gif) {
    list.retain(|g| g.url != gif.url);
    list.insert(0, gif.clone());
    list.truncate(HISTORY_LEN);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_sendable_gif_and_skips_ads() {
        let body: Response = serde_json::from_str(
            r#"{"result":true,"data":{"data":[
            {"id":1,"slug":"a","type":"gif","file":{
                "hd":{"gif":{"url":"https://x/hd.gif","width":498,"height":498,"size":9999999}},
                "md":{"gif":{"url":"https://x/md.gif","width":220,"height":220,"size":500000}},
                "sm":{"gif":{"url":"https://x/sm.gif","size":100000},"jpg":{"url":"https://x/sm.jpg","size":5000}}}},
            {"id":2,"type":"ad","file":{"md":{"gif":{"url":"https://ad/md.gif","size":1}}}}
        ],"current_page":1,"per_page":45,"has_next":true}}"#,
        )
        .unwrap();
        let gifs: Vec<Gif> = body
            .data
            .unwrap()
            .data
            .iter()
            .filter_map(Item::to_gif)
            .collect();
        assert_eq!(gifs.len(), 1);
        assert_eq!(gifs[0].url, "https://x/md.gif");
        assert_eq!(gifs[0].preview, "https://x/sm.gif");
        assert_eq!(gifs[0].still, "https://x/sm.jpg");
    }

    #[test]
    fn history_is_newest_first_without_duplicates() {
        let gif = |n: usize| Gif {
            url: format!("https://x/{n}.gif"),
            preview: String::new(),
            still: String::new(),
        };
        let mut list = Vec::new();
        for n in 0..HISTORY_LEN + 5 {
            push_recent(&mut list, &gif(n));
        }
        assert_eq!(list.len(), HISTORY_LEN);
        assert_eq!(list[0], gif(HISTORY_LEN + 4));
        push_recent(&mut list, &gif(HISTORY_LEN));
        assert_eq!(list[0], gif(HISTORY_LEN));
        assert_eq!(list.iter().filter(|g| **g == gif(HISTORY_LEN)).count(), 1);
    }

    #[test]
    fn history_survives_in_the_database() {
        let db = Database::open_in_memory().unwrap();
        assert!(history(&db).is_empty());
        let gif = Gif {
            url: "https://x/a.gif".into(),
            preview: "https://x/a-sm.gif".into(),
            still: String::new(),
        };
        remember(&db, &gif);
        assert_eq!(history(&db), vec![gif]);
    }

    #[test]
    fn flattens_error_messages() {
        let body: Response = serde_json::from_str(
            r#"{"result":false,"errors":{"message":["The provided API key is invalid."]}}"#,
        )
        .unwrap();
        assert_eq!(
            error_message(&body.errors.unwrap()).as_deref(),
            Some("The provided API key is invalid.")
        );
    }
}
