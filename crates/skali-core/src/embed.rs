//! What Basecamp makes of a link: a preview of the page (an
//! `opengraph-embed` attachment in rich text), a video player, or a post
//! (a tweet...) whose embed Basecamp keeps a copy of; the last two are a
//! `<figure>` around an `<iframe>`, even in a plain-text chat line.
//! Skali shows none of them as Basecamp does; each is read into an
//! [`Embed`] for a card of Skali's own, and its pictures and posts are
//! fetched like a browser would, without the sign-in.

use crate::richtext::{ATTRIBUTE, LINK, decode_entities, plain_text};
use std::sync::LazyLock;

use regex::Regex;

/// The `content-type` of a page preview's `<bc-attachment>`.
pub const OPENGRAPH: &str = "application/vnd.basecamp.opengraph-embed";
/// The most bytes a preview picture or thumbnail may be.
const MAX_BYTES: usize = 10 * 1024 * 1024;

static OPENGRAPH_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<bc-attachment\b([^>]*application/vnd\.basecamp\.opengraph-embed[^>]*)>")
        .unwrap()
});
static IFRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<iframe\b([^>]*)>\s*</iframe>").unwrap());
/// A player as Basecamp puts it in a plain-text line, figure and all.
static FIGURE_IFRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<figure\b[^>]*>\s*<iframe\b([^>]*)>\s*</iframe>\s*</figure>").unwrap()
});
static SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script\b.*?</script>").unwrap());
static PARAGRAPH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<p\b[^>]*>(.*?)</p>").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Embed {
    /// A web page: its title, description and picture (empty if none).
    Page {
        href: String,
        title: String,
        caption: String,
        image: String,
    },
    /// A video: its page, a still at an address the site keeps for it
    /// (empty if none), and the site's name.
    Video {
        href: String,
        thumbnail: String,
        site: String,
    },
    /// A post from another site (a tweet...), as the page Basecamp keeps
    /// of its embed; [`fetch_post`] reads it.
    Post { src: String },
}

impl Embed {
    /// Where a click on it goes.
    pub fn href(&self) -> &str {
        match self {
            Self::Page { href, .. } | Self::Video { href, .. } => href,
            Self::Post { src } => src,
        }
    }

    /// The site's oEmbed endpoint for this video, which knows its title.
    pub fn oembed_url(&self) -> Option<String> {
        let Self::Video { href, site, .. } = self else {
            return None;
        };
        let endpoint = match site.as_str() {
            "YouTube" => "https://www.youtube.com/oembed",
            "Vimeo" => "https://vimeo.com/api/oembed.json",
            _ => return None,
        };
        url::Url::parse_with_params(endpoint, &[("url", href.as_str()), ("format", "json")])
            .ok()
            .map(String::from)
    }
}

fn attribute(attributes: &str, name: &str) -> String {
    ATTRIBUTE
        .captures_iter(attributes)
        .find(|c| c[1].eq_ignore_ascii_case(name))
        .map(|c| decode_entities(&c[2]))
        .unwrap_or_default()
}

fn is_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// A page preview from its `<bc-attachment>`'s attributes.
pub(crate) fn page_of(attributes: &str) -> Option<Embed> {
    let href = attribute(attributes, "href");
    if !is_web(&href) {
        return None;
    }
    let image = attribute(attributes, "url");
    Some(Embed::Page {
        title: attribute(attributes, "title").trim().to_string(),
        caption: attribute(attributes, "caption").trim().to_string(),
        image: if is_web(&image) { image } else { String::new() },
        href,
    })
}

/// What an `<iframe>` shows, from its attributes: a post Basecamp keeps
/// the embed of, a video on YouTube or Vimeo by its own page, any other
/// player by its address.
pub(crate) fn frame_of(attributes: &str) -> Option<Embed> {
    let src = attribute(attributes, "src");
    let url = url::Url::parse(&src).ok()?;
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }
    let host = url.host_str()?.trim_start_matches("www.").to_string();
    if host.ends_with(".basecamp-static.com") && url.path().contains("/embeds/") {
        return Some(Embed::Post { src });
    }
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let video = |site: &str, href: String, thumbnail: String| Embed::Video {
        href,
        thumbnail,
        site: site.to_string(),
    };
    Some(match (host.as_str(), segments.next(), segments.next()) {
        ("youtube.com" | "youtube-nocookie.com", Some("embed"), Some(id)) => video(
            "YouTube",
            format!("https://www.youtube.com/watch?v={id}"),
            format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
        ),
        ("player.vimeo.com", Some("video"), Some(id)) => {
            video("Vimeo", format!("https://vimeo.com/{id}"), String::new())
        }
        _ => video(&host, src, String::new()),
    })
}

/// Every preview and player in rich text, in order.
pub fn embeds(html: &str) -> Vec<Embed> {
    let mut found: Vec<(usize, Embed)> = OPENGRAPH_TAG
        .captures_iter(html)
        .filter_map(|c| Some((c.get(0)?.start(), page_of(&c[1])?)))
        .chain(
            IFRAME
                .captures_iter(html)
                .filter_map(|c| Some((c.get(0)?.start(), frame_of(&c[1])?))),
        )
        .collect();
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, embed)| embed).collect()
}

/// A plain-text chat line's words and the players and posts Basecamp put
/// in them (where the link was). Nothing else in the text is taken for
/// HTML.
pub fn split_text(content: &str) -> (String, Vec<Embed>) {
    let mut embeds = Vec::new();
    let text = FIGURE_IFRAME.replace_all(content, |c: &regex::Captures| {
        match frame_of(&c[1]) {
            Some(embed) => embeds.push(embed),
            None => return c[0].to_string(),
        }
        String::new()
    });
    (text.trim().to_string(), embeds)
}

/// A plain-text chat line for reading as text: its words, and each
/// player's address where Basecamp took the link out. A post's copy at
/// Basecamp means nothing to a reader, so it's left out.
pub fn plain_line_text(content: &str) -> String {
    let (text, embeds) = split_text(content);
    std::iter::once(text)
        .chain(
            embeds
                .iter()
                .filter(|e| !matches!(e, Embed::Post { .. }))
                .map(|e| e.href().to_string()),
        )
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("larger than {} MB", MAX_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

/// A preview's picture or a video's still, from the open web.
pub async fn fetch(url: &str) -> Result<Vec<u8>, FetchError> {
    let response = crate::web().get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|len| len as usize > MAX_BYTES)
    {
        return Err(FetchError::TooLarge);
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::TooLarge);
    }
    Ok(bytes.to_vec())
}

/// A post as its embed shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Post {
    /// Who wrote it, as the embed names them: `Name (@handle)`.
    pub author: String,
    pub text: String,
    /// The post on its own site; empty if the embed links nowhere.
    pub href: String,
    /// When, as the embed writes it.
    pub date: String,
}

/// Read the page Basecamp keeps of an embed: its content is the site's own
/// oEmbed HTML. A tweet is a `<blockquote>`: the text in a `<p>`, then
/// `— Name (@handle) <a href="…/status/…">date</a>`. Anything else gives
/// its text, and its last link as where it lives.
pub fn read_post(page: &str) -> Option<Post> {
    let start = page.find("embed_content")?;
    let content = &page[start..];
    let content = &content[content.find('>')? + 1..];
    let content = &content[..content.rfind("</div>").unwrap_or(content.len())];
    let content = SCRIPT.replace_all(content, "");
    let links: Vec<(usize, String, String)> = LINK
        .captures_iter(&content)
        .filter_map(|c| {
            let href = attribute(&c[1], "href");
            let at = c.get(0)?.start();
            is_web(&href).then(|| (at, href, plain_text(&c[2])))
        })
        .collect();
    let (link_at, href, date) = links.last().cloned().unwrap_or_default();
    let (text, author, date) = match PARAGRAPH.captures(&content) {
        Some(p) if content.contains("<blockquote") => {
            let after = p.get(0)?.end();
            let byline = if link_at > after {
                &content[after..link_at]
            } else {
                &content[after..]
            };
            let author = plain_text(byline)
                .trim_start_matches(['—', '-', ' '])
                .trim()
                .to_string();
            (plain_text(&tweet_text(&p[1])), author, date)
        }
        _ => (plain_text(&content), String::new(), String::new()),
    };
    Some(Post {
        author,
        text,
        href: without_tracking(&href),
        date,
    })
}

/// A tweet's text without the short links X adds for its own media.
fn tweet_text(html: &str) -> String {
    LINK.replace_all(html, |c: &regex::Captures| {
        let words = plain_text(&c[2]);
        if words.starts_with("https://t.co/")
            || words.starts_with("pic.twitter.com/")
            || words.starts_with("pic.x.com/")
        {
            String::new()
        } else {
            c[0].to_string()
        }
    })
    .into_owned()
}

/// A link without the `ref_src` X adds to say where it was embedded.
fn without_tracking(href: &str) -> String {
    let Ok(mut url) = url::Url::parse(href) else {
        return href.to_string();
    };
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "ref_src")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    url.into()
}

/// A post's embed, from the copy Basecamp keeps; None if it isn't one Skali
/// can read.
pub async fn fetch_post(src: &str) -> Result<Option<Post>, FetchError> {
    let bytes = fetch(src).await?;
    Ok(read_post(&String::from_utf8_lossy(&bytes)))
}

/// What an oEmbed endpoint says about a video.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct VideoInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub thumbnail_url: String,
}

pub async fn video_info(oembed_url: &str) -> Result<VideoInfo, FetchError> {
    Ok(crate::web()
        .get(oembed_url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YOUTUBE: &str = r#"Please welcome Brandon: <figure dir="auto">
    <iframe width="600" height="338" src="https://www.youtube.com/embed/WJqRIeTjUbI?feature=oembed" sandbox="allow-scripts" class="embed embed--video" allowfullscreen="allowfullscreen" scrolling="no"></iframe>

</figure>"#;

    #[test]
    fn a_plain_line_gives_up_its_player() {
        let (text, embeds) = split_text(YOUTUBE);
        assert_eq!(text, "Please welcome Brandon:");
        assert_eq!(
            embeds,
            vec![Embed::Video {
                href: "https://www.youtube.com/watch?v=WJqRIeTjUbI".into(),
                thumbnail: "https://i.ytimg.com/vi/WJqRIeTjUbI/hqdefault.jpg".into(),
                site: "YouTube".into(),
            }]
        );
        assert_eq!(
            plain_line_text(YOUTUBE),
            "Please welcome Brandon: https://www.youtube.com/watch?v=WJqRIeTjUbI"
        );
        // Anything else that looks like HTML is what was typed.
        let typed = "a <figure>x</figure> <iframe src=\"javascript:x\"></iframe>";
        assert_eq!(split_text(typed), (typed.to_string(), vec![]));
    }

    #[test]
    fn players_are_known_by_site() {
        let video = |src: &str| frame_of(&format!("src=\"{src}\""));
        assert_eq!(
            video("https://player.vimeo.com/video/76979871?h=1"),
            Some(Embed::Video {
                href: "https://vimeo.com/76979871".into(),
                thumbnail: String::new(),
                site: "Vimeo".into(),
            })
        );
        assert_eq!(
            video("https://example.com/player/1"),
            Some(Embed::Video {
                href: "https://example.com/player/1".into(),
                thumbnail: String::new(),
                site: "example.com".into(),
            })
        );
        assert_eq!(video("javascript:alert(1)"), None);
        assert!(
            video("https://www.youtube-nocookie.com/embed/abc")
                .unwrap()
                .oembed_url()
                .unwrap()
                .starts_with("https://www.youtube.com/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dabc")
        );
    }

    #[test]
    fn a_tweet_is_read_from_basecamps_copy() {
        let line = r#"Welcome: <figure dir="auto">
    <iframe width="550" height="550" src="https://bc3-production-assets-cdn.basecamp-static.com/1/embeds/BAh7--54/full" class="embed embed--rich"></iframe>

</figure>"#;
        let src = "https://bc3-production-assets-cdn.basecamp-static.com/1/embeds/BAh7--54/full";
        assert_eq!(
            split_text(line),
            ("Welcome:".into(), vec![Embed::Post { src: src.into() }])
        );
        assert_eq!(plain_line_text(line), "Welcome:");
        let page = r#"<!DOCTYPE html><html><head><script nonce="x">var a = "<p>no</p>";</script></head>
<body class="embed-iframe">
      <div class="formatted_content embed_content" data-window-identifier="embed_4180117">
      <blockquote class="twitter-tweet" data-cards="hidden"><p lang="en" dir="ltr">I run Omarchy on Macs &amp; more.<br><br>Really happy to… <a href="https://t.co/5EK">https://t.co/5EK</a> <a href="https://t.co/Q9G">pic.twitter.com/Q9G</a></p>&mdash; Kazuhide Takahashi (@kazuhidet) <a href="https://x.com/kazuhidet/status/2101801432953569756?ref_src=twsrc%5Etfw">September 20, 2026</a></blockquote>
<script async src="https://platform.x.com/widgets.js" charset="utf-8"></script>
    </div>
</body></html>"#;
        assert_eq!(
            read_post(page),
            Some(Post {
                author: "Kazuhide Takahashi (@kazuhidet)".into(),
                text: "I run Omarchy on Macs & more.\n\nReally happy to…".into(),
                href: "https://x.com/kazuhidet/status/2101801432953569756".into(),
                date: "September 20, 2026".into(),
            })
        );
        // Anything else: its words, and its last link.
        let other = r#"<div class="embed_content"><div>A <b>song</b> <a href="https://s.example/t/1">listen</a></div></div>"#;
        assert_eq!(
            read_post(other),
            Some(Post {
                author: String::new(),
                text: "A song listen".into(),
                href: "https://s.example/t/1".into(),
                date: String::new(),
            })
        );
        assert_eq!(read_post("<html>nothing</html>"), None);
    }

    #[test]
    fn rich_text_gives_up_its_previews() {
        let html = r#"<p><a href="https://omarchy.org/teams/">https://omarchy.org/teams/</a><bc-attachment content-type="application/vnd.basecamp.opengraph-embed" url="https://omarchy.org/social.png" href="https://omarchy.org/teams/" caption="The people &amp; teams." title="Teams - Omarchy"></bc-attachment></p><figure><iframe src="https://www.youtube.com/embed/x"></iframe></figure>"#;
        assert_eq!(
            embeds(html),
            vec![
                Embed::Page {
                    href: "https://omarchy.org/teams/".into(),
                    title: "Teams - Omarchy".into(),
                    caption: "The people & teams.".into(),
                    image: "https://omarchy.org/social.png".into(),
                },
                Embed::Video {
                    href: "https://www.youtube.com/watch?v=x".into(),
                    thumbnail: "https://i.ytimg.com/vi/x/hqdefault.jpg".into(),
                    site: "YouTube".into(),
                },
            ]
        );
    }
}
