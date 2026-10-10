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
static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([a-zA-Z-]+)="([^"]*)""#).unwrap());
static FIGCAPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<figcaption[^>]*>(.*?)</figcaption>").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\r\n]+").unwrap());

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

fn decode_entities(text: &str) -> String {
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

/// One line for a list row: the plain text, whitespace collapsed, cut at
/// `max` characters with an ellipsis.
pub fn excerpt(html: &str, max: usize) -> String {
    let text = WHITESPACE
        .replace_all(&plain_text(html), " ")
        .trim()
        .to_string();
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Pango markup for a label: bold, italic, strike, code, links and line
/// breaks survive; everything else is text. Safe to hand to `set_markup`.
pub fn pango_markup(html: &str) -> String {
    let html = ATTACHMENT.replace_all(html, |c: &regex::Captures| {
        format!("<b>{}</b>", escape(&attachment_text(&c[1], &c[2])))
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
}
