//! Basecamp's rich text is HTML. Two readings of it: Pango markup for a
//! label (chat lines, comments, excerpts), and plain text for a one-line
//! preview. Both decode entities, keep what Pango can show, and turn
//! `<bc-attachment>` into something readable: a mention's name, a file's
//! name.

use std::sync::LazyLock;

use regex::Regex;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static ATTACHMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<bc-attachment\b([^>]*)>(.*?)</bc-attachment>").unwrap());
pub(crate) static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([a-zA-Z-]+)="([^"]*)""#).unwrap());
static FIGCAPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<figcaption[^>]*>(.*?)</figcaption>").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\r\n]+").unwrap());
static IMG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<img\b[^>]*>").unwrap());
static FIGURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<figure\b[^>]*>\s*<iframe\b.*?</figure>").unwrap());
pub(crate) static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<a\b([^>]*)>(.*?)</a>").unwrap());

/// What `markup_with_mentions` leaves where each mention was: the object
/// replacement character, for a label to put a widget on.
pub const OBJECT: char = '\u{FFFC}';

/// A mention in rich text: the name as written and the picture Basecamp
/// put beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// Usually the first name.
    pub name: String,
    /// From the picture's alt text; the name as written if there is none.
    pub full_name: String,
    /// Empty if the mention came without a picture.
    pub avatar_url: String,
}

/// The mention a `<bc-attachment>` stands for, if it is one.
fn mention_of(attributes: &str, inner: &str) -> Option<Mention> {
    ATTRIBUTE
        .captures_iter(attributes)
        .find(|c| &c[1] == "content-type" && &c[2] == "application/vnd.basecamp.mention")?;
    let name = FIGCAPTION
        .captures(inner)
        .map(|c| plain_text(&c[1]).trim().to_string())
        .unwrap_or_default();
    let mut full_name = String::new();
    let mut avatar_url = String::new();
    if let Some(img) = IMG.find(inner) {
        for capture in ATTRIBUTE.captures_iter(img.as_str()) {
            match &capture[1] {
                "alt" => full_name = decode_entities(&capture[2]).trim().to_string(),
                "src" => avatar_url = decode_entities(&capture[2]),
                _ => {}
            }
        }
    }
    let name = if name.is_empty() {
        full_name.clone()
    } else {
        name
    };
    if full_name.is_empty() {
        full_name = name.clone();
    }
    Some(Mention {
        name,
        full_name,
        avatar_url,
    })
}

/// What a `<bc-attachment>` stood for.
fn attachment_text(attributes: &str, inner: &str) -> String {
    let mut content_type = String::new();
    let mut filename = String::new();
    for capture in ATTRIBUTE.captures_iter(attributes) {
        match &capture[1] {
            "content-type" => content_type = capture[2].to_string(),
            "filename" => filename = capture[2].to_string(),
            _ => {}
        }
    }
    if content_type == "application/vnd.basecamp.mention" {
        let name = FIGCAPTION
            .captures(inner)
            .map(|c| plain_text(&c[1]))
            .unwrap_or_default();
        return format!("@{}", name.trim());
    }
    // A link's preview: the link is in the text already.
    if content_type == crate::embed::OPENGRAPH {
        return String::new();
    }
    if filename.is_empty() {
        let caption = FIGCAPTION
            .captures(inner)
            .map(|c| plain_text(&c[1]))
            .unwrap_or_default();
        return format!(
            "[{}]",
            if caption.trim().is_empty() {
                "attachment"
            } else {
                caption.trim()
            }
        );
    }
    format!("[{filename}]")
}

pub(crate) fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|&end| end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| {
                    number
                        .strip_prefix('x')
                        .or_else(|| number.strip_prefix('X'))
                        .map_or_else(
                            || number.parse().ok(),
                            |hex| u32::from_str_radix(hex, 16).ok(),
                        )
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Plain text: tags gone, block boundaries as newlines, entities decoded,
/// attachments named. Runs of blank lines collapse.
pub fn plain_text(html: &str) -> String {
    let text = ATTACHMENT.replace_all(html, |c: &regex::Captures| attachment_text(&c[1], &c[2]));
    let text = TAG.replace_all(&text, |c: &regex::Captures| {
        let tag = c[0].to_ascii_lowercase();
        let name: String = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        match name.as_str() {
            "br" => "\n".to_string(),
            "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "pre"
            | "tr" | "figure" => {
                if tag.starts_with("</") {
                    "\n".to_string()
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    });
    let text = decode_entities(&text);
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last() == Some(&"") {
        lines.pop();
    }
    lines.join("\n")
}

/// `plain_text` with each link's address kept: `words (url)`, or just the
/// url when that is what the words were. For reading by something that
/// may follow them (a summary).
pub fn text_with_links(html: &str) -> String {
    let text = LINK.replace_all(html, |c: &regex::Captures| {
        let href = ATTRIBUTE
            .captures_iter(&c[1])
            .find(|a| a[1].eq_ignore_ascii_case("href"))
            .map(|a| decode_entities(&a[2]));
        let words = plain_text(&c[2]);
        match href {
            Some(href) if href.starts_with("http") => {
                let words = words.trim();
                if words.is_empty() || words == href || href.contains(words.trim_end_matches('…'))
                {
                    escape(&href)
                } else {
                    format!("{} ({})", escape(words), escape(&href))
                }
            }
            _ => escape(&words),
        }
    });
    plain_text(&text)
}

/// One line for a list row: the plain text, whitespace collapsed, cut at
/// `max` characters with an ellipsis.
pub fn excerpt(html: &str, max: usize) -> String {
    shorten(&plain_text(html), max)
}

/// Plain text on one line, cut to `max` characters with an ellipsis.
pub fn shorten(text: &str, max: usize) -> String {
    let text = WHITESPACE.replace_all(text, " ").trim().to_string();
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Pango markup for a label: bold, italic, strike, code, links and line
/// breaks survive; everything else is text. Safe to hand to `set_markup`.
pub fn pango_markup(html: &str) -> String {
    markup(html, None)
}

/// Pango markup as `pango_markup` makes it, but with an [`OBJECT`] where
/// each mention was, and the mentions in the order they appear.
pub fn markup_with_mentions(html: &str) -> (String, Vec<Mention>) {
    let mut mentions = Vec::new();
    let markup = markup(html, Some(&mut mentions));
    (markup, mentions)
}

fn markup(html: &str, mut mentions: Option<&mut Vec<Mention>>) -> String {
    // An OBJECT in the text itself would take a mention's place.
    let html = html.replace(OBJECT, "");
    let html = ATTACHMENT.replace_all(&html, |c: &regex::Captures| {
        if let Some(found) = mentions.as_deref_mut()
            && let Some(mention) = mention_of(&c[1], &c[2])
        {
            found.push(mention);
            return OBJECT.to_string();
        }
        let text = attachment_text(&c[1], &c[2]);
        if text.is_empty() {
            return text;
        }
        format!("<b>{}</b>", escape(&text))
    });
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    let mut link_stack: Vec<bool> = Vec::new();
    for tag in TAG.find_iter(&html) {
        out.push_str(&escape(&decode_entities(&html[last..tag.start()])));
        last = tag.end();
        let raw = tag.as_str();
        let lower = raw.to_ascii_lowercase();
        let closing = lower.starts_with("</");
        let name: String = lower
            .trim_start_matches('<')
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        match (name.as_str(), closing) {
            ("b" | "strong", false) => out.push_str("<b>"),
            ("b" | "strong", true) => out.push_str("</b>"),
            ("i" | "em", false) => out.push_str("<i>"),
            ("i" | "em", true) => out.push_str("</i>"),
            ("s" | "strike" | "del", false) => out.push_str("<s>"),
            ("s" | "strike" | "del", true) => out.push_str("</s>"),
            ("u", false) => out.push_str("<u>"),
            ("u", true) => out.push_str("</u>"),
            ("code" | "pre" | "kbd" | "tt", false) => out.push_str("<tt>"),
            ("code" | "pre" | "kbd" | "tt", true) => out.push_str("</tt>"),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => out.push_str("<b>"),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => out.push_str("</b>\n"),
            ("a", false) => {
                let href = ATTRIBUTE
                    .captures_iter(raw)
                    .find(|c| c[1].eq_ignore_ascii_case("href"))
                    .map(|c| c[2].to_string());
                match href {
                    Some(href) if href.starts_with("http") || href.starts_with("mailto:") => {
                        out.push_str(&format!("<a href=\"{}\">", escape(&decode_entities(&href))));
                        link_stack.push(true);
                    }
                    _ => link_stack.push(false),
                }
            }
            ("a", true) => {
                if link_stack.pop().unwrap_or(false) {
                    out.push_str("</a>");
                }
            }
            ("br", _) => out.push('\n'),
            ("li", false) => out.push_str("• "),
            ("p" | "div" | "li" | "blockquote" | "tr" | "figure", true) => out.push('\n'),
            _ => {}
        }
    }
    out.push_str(&escape(&decode_entities(&html[last..])));
    // Any link left open (malformed HTML) is closed so Pango accepts it.
    for open in link_stack.into_iter().rev() {
        if open {
            out.push_str("</a>");
        }
    }
    let mut lines: Vec<&str> = Vec::new();
    for line in out.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() && lines.last().is_none_or(|last| last.trim().is_empty()) {
            continue;
        }
        lines.push(trimmed);
    }
    while lines.last().is_some_and(|last| last.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// What stands in for a `<bc-attachment>` in a rendered document.
pub enum Expanded {
    /// An image we have the bytes of, as a `data:` URI, and the link a
    /// click on it follows (empty for none).
    Image {
        src: String,
        alt: String,
        href: String,
    },
    /// A file (or an image not fetched yet): a link the app handles.
    File { href: String, filename: String },
}

/// Rich text for a browser view: every `<bc-attachment>` becomes HTML the
/// view can show. A mention is the person's picture (what `face` gives
/// for its URL, if anything) and name; an image is `<img>` with whatever
/// `resolve` gives for its sgid; a link's preview and a video player
/// (which the view could not load) are cards that link to the page; any
/// other file is a link. The rest of the HTML passes through as Basecamp
/// sent it.
pub fn expand_attachments(
    html: &str,
    resolve: impl Fn(&str) -> Option<Expanded>,
    face: impl Fn(&str) -> Option<String>,
) -> String {
    let html = FIGURE.replace_all(html, |c: &regex::Captures| {
        match crate::embed::embeds(&c[0]).first() {
            Some(embed) => embed_card(embed),
            None => c[0].to_string(),
        }
    });
    ATTACHMENT
        .replace_all(&html, |c: &regex::Captures| {
            let attributes = &c[1];
            let inner = &c[2];
            let mut sgid = String::new();
            let mut filename = String::new();
            let mut href = String::new();
            for capture in ATTRIBUTE.captures_iter(attributes) {
                match &capture[1] {
                    "sgid" => sgid = capture[2].to_string(),
                    "filename" => filename = capture[2].to_string(),
                    "href" => href = capture[2].to_string(),
                    _ => {}
                }
            }
            if let Some(mention) = mention_of(attributes, inner) {
                let picture = face(&mention.avatar_url)
                    .map(|src| format!("<img class=\"face\" src=\"{}\" alt=\"\">", escape(&src)))
                    .unwrap_or_default();
                return format!(
                    "<span class=\"mention\" title=\"{}\">{picture}{}</span>",
                    escape(&mention.full_name),
                    escape(&mention.name)
                );
            }
            if let Some(embed) = crate::embed::page_of(attributes)
                .filter(|_| attributes.contains(crate::embed::OPENGRAPH))
            {
                return embed_card(&embed);
            }
            let caption = FIGCAPTION
                .captures(inner)
                .map(|c| plain_text(&c[1]))
                .unwrap_or_default();
            match resolve(&sgid) {
                Some(Expanded::Image { src, alt, href }) => format!(
                    "<figure class=\"attachment\">{}{}</figure>",
                    if href.is_empty() {
                        format!("<img src=\"{}\" alt=\"{}\">", escape(&src), escape(&alt))
                    } else {
                        format!(
                            "<a class=\"image\" href=\"{}\"><img src=\"{}\" alt=\"{}\"></a>",
                            escape(&href),
                            escape(&src),
                            escape(&alt)
                        )
                    },
                    if caption.trim().is_empty() {
                        String::new()
                    } else {
                        format!("<figcaption>{}</figcaption>", escape(caption.trim()))
                    }
                ),
                Some(Expanded::File { href, filename }) => format!(
                    "<a class=\"file\" href=\"{}\">{}</a>",
                    escape(&href),
                    escape(&filename)
                ),
                None => {
                    let name = if filename.is_empty() {
                        "attachment".to_string()
                    } else {
                        filename
                    };
                    format!(
                        "<a class=\"file\" href=\"{}\">{}</a>",
                        escape(&href),
                        escape(&name)
                    )
                }
            }
        })
        .into_owned()
}

/// A preview or a player as a card linking to its page: no picture, as
/// the view fetches nothing.
fn embed_card(embed: &crate::embed::Embed) -> String {
    let (title, caption) = match embed {
        crate::embed::Embed::Page { title, caption, .. } => (title.as_str(), caption.as_str()),
        crate::embed::Embed::Video { site, href, .. } => (site.as_str(), href.as_str()),
        crate::embed::Embed::Post { .. } => ("Embedded post", ""),
    };
    let title = if title.is_empty() {
        embed.href()
    } else {
        title
    };
    format!(
        "<a class=\"embed\" href=\"{}\"><b>{}</b>{}</a>",
        escape(embed.href()),
        escape(title),
        if caption.is_empty() {
            String::new()
        } else {
            format!("<span>{}</span>", escape(caption))
        }
    )
}

/// Whether `html` has more than paragraphs and line breaks: what editing it
/// as plain text (`plain_text`, then `text_to_html`) would lose.
pub fn has_formatting(html: &str) -> bool {
    TAG.find_iter(html).any(|tag| {
        let name: String = tag
            .as_str()
            .trim_start_matches('<')
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        !matches!(name.as_str(), "div" | "p" | "br")
    })
}

/// Typed text as Basecamp rich text: one `<div>` per paragraph, `<br>`
/// for a line break inside one, everything escaped.
pub fn text_to_html(text: &str) -> String {
    let paragraphs: Vec<String> = text
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| {
            let lines: Vec<String> = paragraph.lines().map(escape).collect();
            format!("<div>{}</div>", lines.join("<br>"))
        })
        .collect();
    paragraphs.join("")
}

/// Whether `html` mentions the person whose `attachable_sgid` is `sgid`.
pub fn mentions(html: &str, sgid: &str) -> bool {
    !sgid.is_empty()
        && ATTACHMENT.captures_iter(html).any(|attachment| {
            let mut is_mention = false;
            let mut is_them = false;
            for capture in ATTRIBUTE.captures_iter(&attachment[1]) {
                match &capture[1] {
                    "content-type" => {
                        is_mention = &capture[2] == "application/vnd.basecamp.mention"
                    }
                    "sgid" => is_them = &capture[2] == sgid,
                    _ => {}
                }
            }
            is_mention && is_them
        })
}

/// A file uploaded to `/attachments.json`, placed in rich text by its
/// `attachable_sgid`; Basecamp renders it on the way back.
pub fn attachment_tag(sgid: &str) -> String {
    format!("<bc-attachment sgid=\"{}\"></bc-attachment>", escape(sgid))
}

/// Escape for Pango markup (which is XML).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_strips_and_breaks() {
        let html = "<div>Hello <strong>there</strong><br>second</div><div>&amp; third &#8217;s</div><div></div>";
        assert_eq!(plain_text(html), "Hello there\nsecond\n& third ’s");
        assert_eq!(
            plain_text("a &mdash; b&hellip; &ldquo;c&rdquo;"),
            "a — b… “c”"
        );
    }

    #[test]
    fn links_keep_their_addresses() {
        let html = r#"<div>See <a href="https://github.com/o/r/pull/7?a=1&amp;b=2">the fix</a>, and
            <a href="https://example.com/x">https://example.com/x</a> or
            <a href="https://github.com/o/r/pull/9">github.com/o/r/pull/9</a><br><a href="mailto:a@b.c">mail</a></div>"#;
        assert_eq!(
            text_with_links(html),
            "See the fix (https://github.com/o/r/pull/7?a=1&b=2), and\nhttps://example.com/x or\nhttps://github.com/o/r/pull/9\nmail"
        );
    }

    #[test]
    fn mentions_of_one_person_are_found() {
        let html = r#"<div><bc-attachment sgid="ann" content-type="application/vnd.basecamp.mention"><figure><figcaption>Ann</figcaption></figure></bc-attachment> and <bc-attachment sgid="bob" content-type="image/png"></bc-attachment></div>"#;
        assert!(mentions(html, "ann"));
        // Bob is in an attachment, not a mention.
        assert!(!mentions(html, "bob"));
        assert!(!mentions(html, "cy"));
        assert!(!mentions(html, ""));
    }

    #[test]
    fn mentions_and_files_are_named() {
        let html = r#"Thanks <bc-attachment sgid="x" content-type="application/vnd.basecamp.mention"><figure><img src="a"><figcaption>Victor Cooper</figcaption></figure></bc-attachment> see <bc-attachment sgid="y" content-type="image/png" filename="shot.png" width="1" height="1"></bc-attachment>"#;
        assert_eq!(plain_text(html), "Thanks @Victor Cooper see [shot.png]");
        assert_eq!(
            pango_markup(html),
            "Thanks <b>@Victor Cooper</b> see <b>[shot.png]</b>"
        );
    }

    #[test]
    fn attachments_expand_for_a_browser() {
        let html = r#"<div>Hi <bc-attachment sgid="m" content-type="application/vnd.basecamp.mention"><figure><img src="x"><figcaption>Ann Lee</figcaption></figure></bc-attachment></div><bc-attachment sgid="i" content-type="image/png" filename="a.png" href="https://d/a.png"><figure><img src="https://preview/x"><figcaption>A caption</figcaption></figure></bc-attachment><bc-attachment sgid="f" content-type="application/pdf" filename="b.pdf" href="https://d/b.pdf"></bc-attachment>"#;
        let out = expand_attachments(
            html,
            |sgid| match sgid {
                "i" => Some(Expanded::Image {
                    src: "data:image/png;base64,AAAA".into(),
                    alt: "a.png".into(),
                    href: "skali:image/0".into(),
                }),
                _ => None,
            },
            |url| (url == "x").then(|| "data:image/png;base64,FACE".into()),
        );
        assert_eq!(
            out,
            r#"<div>Hi <span class="mention" title="Ann Lee"><img class="face" src="data:image/png;base64,FACE" alt="">Ann Lee</span></div><figure class="attachment"><a class="image" href="skali:image/0"><img src="data:image/png;base64,AAAA" alt="a.png"></a><figcaption>A caption</figcaption></figure><a class="file" href="https://d/b.pdf">b.pdf</a>"#
        );
    }

    #[test]
    fn previews_and_players_become_cards() {
        let html = r#"<p><a href="https://o.org/t">https://o.org/t</a><bc-attachment content-type="application/vnd.basecamp.opengraph-embed" url="https://o.org/s.png" href="https://o.org/t" caption="People &amp; teams" title="Teams"></bc-attachment></p><figure dir="auto">
    <iframe src="https://www.youtube.com/embed/x?feature=oembed"></iframe>
</figure>"#;
        assert_eq!(
            expand_attachments(html, |_| None, |_| None),
            r#"<p><a href="https://o.org/t">https://o.org/t</a><a class="embed" href="https://o.org/t"><b>Teams</b><span>People &amp; teams</span></a></p><a class="embed" href="https://www.youtube.com/watch?v=x"><b>YouTube</b><span>https://www.youtube.com/watch?v=x</span></a>"#
        );
        // The link is in the text; the preview adds nothing to it.
        assert_eq!(plain_text(html), "https://o.org/t");
        assert_eq!(
            pango_markup(html),
            r#"<a href="https://o.org/t">https://o.org/t</a>"#
        );
    }

    #[test]
    fn mentions_leave_objects_in_the_markup() {
        let html = "<div dir=\"auto\">\n<bc-attachment sgid=\"s\" content-type=\"application/vnd.basecamp.mention\" content='&lt;bc-mention gid=\"p\"&gt;&lt;img src=\"wrong\"&gt;'><figure>\n  <img alt=\"Brandon Williams\" class=\"avatar\" src=\"https://cdn/people/B=/avatar?a=1&amp;b=2\" width=\"20\">\n  <figcaption>\n    Brandon\n  </figcaption>\n</figure></bc-attachment> thanks \u{FFFC}<b>x</b> <bc-attachment sgid=\"f\" content-type=\"image/png\" filename=\"a.png\"></bc-attachment></div>";
        let (markup, mentions) = markup_with_mentions(html);
        assert_eq!(markup, "\u{FFFC} thanks <b>x</b> <b>[a.png]</b>");
        assert_eq!(
            mentions,
            vec![Mention {
                name: "Brandon".into(),
                full_name: "Brandon Williams".into(),
                avatar_url: "https://cdn/people/B=/avatar?a=1&b=2".into(),
            }]
        );
        // Without a picture or a caption, one name stands for the other.
        let bare = r#"<bc-attachment content-type="application/vnd.basecamp.mention"><figure><figcaption>Ann</figcaption></figure></bc-attachment>"#;
        let (_, mentions) = markup_with_mentions(bare);
        assert_eq!(mentions[0].full_name, "Ann");
        assert_eq!(mentions[0].avatar_url, "");
    }

    #[test]
    fn typed_text_becomes_paragraphs() {
        assert_eq!(
            text_to_html("One <two>\nstill one\n\nThree & four\n"),
            "<div>One &lt;two&gt;<br>still one</div><div>Three &amp; four</div>"
        );
        assert_eq!(text_to_html("  \n"), "");
    }

    #[test]
    fn attachment_tag_quotes_the_sgid() {
        assert_eq!(
            attachment_tag("BAh7==--1a\""),
            r#"<bc-attachment sgid="BAh7==--1a&quot;"></bc-attachment>"#
        );
    }

    #[test]
    fn shorten_keeps_what_looks_like_tags() {
        assert_eq!(
            shorten("With <b>tags</b>\n & more", 100),
            "With <b>tags</b> & more"
        );
        assert_eq!(shorten("abcdef", 4), "abc…");
    }

    #[test]
    fn excerpt_is_one_line_and_cut() {
        let html = "<p>One</p><p>two   three</p>";
        assert_eq!(excerpt(html, 100), "One two three");
        assert_eq!(excerpt(html, 6), "One t…");
    }

    #[test]
    fn markup_keeps_what_pango_shows() {
        let html = r#"<div>A <b>bold</b>, <em>it</em>, <a href="https://x.y/?a=1&amp;b=2">link</a> &lt;tag&gt; <span style="x">plain</span></div><ul><li>one</li><li>two</li></ul>"#;
        assert_eq!(
            pango_markup(html),
            "A <b>bold</b>, <i>it</i>, <a href=\"https://x.y/?a=1&amp;b=2\">link</a> &lt;tag&gt; plain\n• one\n• two"
        );
        // javascript: links become plain text; an unclosed link is closed.
        assert_eq!(pango_markup(r#"<a href="javascript:x">no</a>"#), "no");
        assert_eq!(
            pango_markup(r#"<a href="https://a">open"#),
            "<a href=\"https://a\">open</a>"
        );
    }

    #[test]
    fn formatting_is_anything_past_paragraphs() {
        assert!(!has_formatting(""));
        assert!(!has_formatting("<div>One<br>two</div><p>Three</p>"));
        assert!(!has_formatting(&text_to_html("a & b\n\nc")));
        assert!(has_formatting("<div><strong>Bold</strong></div>"));
        assert!(has_formatting(r#"<div><a href="https://x">link</a></div>"#));
        assert!(has_formatting("<ul><li>x</li></ul>"));
        assert!(has_formatting(
            r#"<bc-attachment sgid="s"></bc-attachment>"#
        ));
    }
}
