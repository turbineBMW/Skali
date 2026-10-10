//! Basecamp's rich text in a WebKit view: the one way to show it as
//! written, images and all. JavaScript is off, nothing is fetched from
//! the network (images arrive as `data:` URIs), a link click opens a
//! Basecamp item on Skali's own page or goes to the browser, and in dark
//! mode the document is adapted to the scheme.

use crate::accent;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::gdk;
use gtk::glib;
use skali_core::darkmode;
use webkit::prelude::*;

const EXTERNAL_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

pub struct RichTextView {
    view: webkit::WebView,
}

impl Default for RichTextView {
    fn default() -> Self {
        Self::new()
    }
}

impl RichTextView {
    pub fn new() -> Self {
        let view = webkit::WebView::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_javascript(false);
            settings.set_enable_page_cache(false);
            settings.set_enable_media(false);
            settings.set_enable_webaudio(false);
            settings.set_enable_webgl(false);
            settings.set_enable_back_forward_navigation_gestures(false);
        }
        // The window's own background shows through.
        view.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
        view.connect_decide_policy(|view, decision, _| decide_policy(view, decision));
        Self { view }
    }

    pub fn widget(&self) -> &webkit::WebView {
        &self.view
    }

    /// Show `body`, already expanded (no `<bc-attachment>` left in it),
    /// scrolled to the element with the id `anchor` if there is one.
    pub fn set_document(&self, body: &str, anchor: Option<&str>) {
        // The document's address carries the fragment WebKit scrolls to.
        let base = anchor.map(|anchor| format!("about:blank#{anchor}"));
        self.view.load_html(&document(body), base.as_deref());
    }
}

fn is_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

/// The page around a body: a stylesheet in the app's colours and type,
/// a policy that blocks every remote fetch, and the dark-mode rewrite.
fn document(body: &str) -> String {
    let accent = accent::accent_hex();
    let (text, dim, border, code_bg) = if is_dark() {
        let scheme = accent::reader_scheme();
        (
            scheme.text_css(),
            "rgba(255,255,255,0.55)".to_string(),
            "rgba(255,255,255,0.12)".to_string(),
            "rgba(255,255,255,0.08)".to_string(),
        )
    } else {
        (
            "#2e3436".to_string(),
            "rgba(0,0,0,0.55)".to_string(),
            "rgba(0,0,0,0.12)".to_string(),
            "rgba(0,0,0,0.06)".to_string(),
        )
    };
    let body = if is_dark() {
        darkmode::adapt_with(body, &accent::reader_scheme())
    } else {
        body.to_string()
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src 'none'\">\
         <style>\
         :root {{ color-scheme: {scheme}; }}\
         html, body {{ margin: 0; padding: 0; background: transparent; }}\
         body {{ color: {text}; font-family: system-ui, -apple-system, sans-serif; font-size: 15px; line-height: 1.5; padding: 0 2px 24px; word-wrap: break-word; }}\
         a {{ color: {accent}; }}\
         img {{ max-width: 100%; height: auto; border-radius: 8px; }}\
         figure.attachment {{ margin: 12px 0; }}\
         figcaption {{ color: {dim}; font-size: 0.9em; }}\
         a.file {{ display: inline-block; padding: 6px 10px; border: 1px solid {border}; border-radius: 8px; text-decoration: none; margin: 4px 0; }}\
         .mention {{ font-weight: 600; }}\
         blockquote {{ border-left: 3px solid {border}; margin: 8px 0; padding: 2px 12px; color: {dim}; }}\
         pre, code {{ background: {code_bg}; border-radius: 6px; font-family: monospace; }}\
         pre {{ padding: 10px 12px; overflow-x: auto; }} code {{ padding: 1px 4px; }}\
         h1 {{ font-size: 1.5em; }} h2 {{ font-size: 1.25em; }} h3 {{ font-size: 1.1em; }}\
         table {{ border-collapse: collapse; }} td, th {{ border: 1px solid {border}; padding: 4px 8px; }}\
         section.comments {{ margin-top: 32px; border-top: 1px solid {border}; padding-top: 12px; }}\
         section.comments h2 {{ font-size: 1em; margin: 0 0 12px; }}\
         .comment {{ margin: 0 0 20px; }}\
         .byline {{ display: flex; align-items: center; gap: 10px; margin-bottom: 4px; }}\
         .byline time {{ color: {dim}; font-size: 0.9em; }}\
         .avatar, .initials {{ width: 32px; height: 32px; border-radius: 50%; flex: none; object-fit: cover; }}\
         .initials {{ display: inline-flex; align-items: center; justify-content: center; background: {accent}; color: white; font-weight: 600; }}\
         .comment .body {{ margin-left: 42px; }}\
         .comment:target {{ background: {code_bg}; border-radius: 8px; padding: 8px; margin: 0 -8px 20px; }}\
         </style></head><body>{body}</body></html>",
        scheme = if is_dark() { "dark" } else { "light" },
    )
}

fn navigation_of(decision: &webkit::PolicyDecision) -> Option<(webkit::NavigationType, String)> {
    let navigation = decision.downcast_ref::<webkit::NavigationPolicyDecision>()?;
    let action = navigation.navigation_action()?;
    let uri = action
        .request()
        .and_then(|request| request.uri())
        .map(|uri| uri.to_string())
        .unwrap_or_default();
    Some((action.navigation_type(), uri))
}

fn scheme_of(uri: &str) -> String {
    uri.split(':').next().unwrap_or("").to_lowercase()
}

/// The view renders one document and never navigates: a link click opens
/// in Skali or the browser, anything else is refused. load_html's own
/// document arrives as about:blank (with the anchor), or with no URI.
fn decide_policy(view: &webkit::WebView, decision: &webkit::PolicyDecision) -> bool {
    let Some((kind, uri)) = navigation_of(decision) else {
        return false;
    };
    let scheme = scheme_of(&uri);
    if kind != webkit::NavigationType::LinkClicked {
        if scheme == "about" || scheme.is_empty() {
            return false;
        }
        decision.ignore();
        tracing::warn!("blocked navigation from rich text to {uri}");
        return true;
    }
    decision.ignore();
    if !EXTERNAL_SCHEMES.contains(&scheme.as_str()) {
        tracing::warn!("blocked a rich-text link with scheme {scheme:?}");
        return true;
    }
    if let Some(window) = view.root().and_downcast::<SkaliWindow>()
        && window.open_app_url(&uri)
    {
        return true;
    }
    let window = view.root().and_downcast::<gtk::Window>();
    gtk::UriLauncher::new(&uri).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            tracing::warn!("could not open a link: {error}");
        }
    });
    true
}

/// Bytes as a `data:` URI for an `<img>`.
pub fn data_uri(content_type: &str, bytes: &[u8]) -> String {
    format!("data:{content_type};base64,{}", glib::base64_encode(bytes))
}
