//! A recording with a body and comments -- a message, a card, a document,
//! an upload -- as one document in a rich-text view, with the comment
//! composer along the bottom. Images are fetched with the sign-in and
//! inlined; the document is rendered again as they arrive, and as
//! comments do.

use super::{avatar, count_label, shell_with_header, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::widgets::attachments::{self, AttachmentPicker};
use crate::widgets::richtext_view::RichTextView;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{Attachment, Comment, Person};
use skali_core::richtext;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What the page shows, whatever kind of recording it came from.
pub struct RecordingDoc {
    pub id: i64,
    pub title: String,
    pub creator: Person,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Rich text (HTML), `<bc-attachment>`s and all.
    pub content: String,
    pub attachments: Vec<Attachment>,
    pub comments_count: i64,
    /// HTML shown between the body and the comments: a card's steps.
    pub extra_html: String,
}

pub struct RecordingPage {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    doc: RecordingDoc,
    comments: RefCell<Vec<Comment>>,
    /// `data:` URIs by attachment sgid, once fetched.
    images: RefCell<HashMap<String, String>>,
    fetching: RefCell<HashSet<String>>,
    /// `RecordingDoc::extra_html`, as last set.
    extra_html: RefCell<String>,
    /// The comment a link to it asked to see, scrolled to on every render.
    focus: Cell<Option<i64>>,
    view: RichTextView,
}

impl RecordingPage {
    pub fn recording_id(&self) -> i64 {
        self.doc.id
    }

    /// Scroll to this comment, now and whenever the document is redrawn.
    pub fn focus_comment(&self, comment_id: i64) {
        self.focus.set(Some(comment_id));
        self.render();
    }

    /// Replace what's shown between the body and the comments.
    pub fn set_extra_html(&self, html: String) {
        *self.extra_html.borrow_mut() = html;
        self.render();
    }

    /// A comment that arrived (posted here, or live). Dropped if shown.
    pub fn add_comment(self: &Rc<Self>, comment: Comment) {
        if self
            .comments
            .borrow()
            .iter()
            .any(|c| c.recording.id == comment.recording.id)
        {
            return;
        }
        self.fetch_images(&comment.content_attachments);
        self.comments.borrow_mut().push(comment);
        self.render();
    }

    fn render(&self) {
        let images = self.images.borrow();
        let resolve = |sgid: &str| {
            images.get(sgid).map(|src| richtext::Expanded::Image {
                src: src.clone(),
                alt: String::new(),
            })
        };
        let mut body = format!(
            "<article>{}</article>",
            richtext::expand_attachments(&self.doc.content, resolve)
        );
        body.push_str(&self.extra_html.borrow());
        let comments = self.comments.borrow();
        let count = comments.len().max(self.doc.comments_count as usize);
        body.push_str(&format!(
            "<section class=\"comments\"><h2>{}</h2>",
            richtext::escape(&count_label(
                count as i64,
                &gettext("comment"),
                &gettext("comments")
            ))
        ));
        for comment in comments.iter() {
            let person = &comment.recording.creator;
            let avatar = match crate::avatars::cached_bytes(&person.avatar_url) {
                Some(bytes) => format!(
                    "<img class=\"avatar\" src=\"{}\" alt=\"\">",
                    crate::widgets::richtext_view::data_uri("image/*", &bytes)
                ),
                None => format!(
                    "<span class=\"initials\">{}</span>",
                    richtext::escape(
                        &person
                            .name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string()
                    )
                ),
            };
            body.push_str(&format!(
                "<div class=\"comment\" id=\"c{}\"><div class=\"byline\">{avatar}<b>{}</b><time>{}</time></div><div class=\"body\">{}</div></div>",
                comment.recording.id,
                richtext::escape(&person.name),
                richtext::escape(&time_label(comment.recording.created_at)),
                richtext::expand_attachments(&comment.content, resolve)
            ));
        }
        body.push_str("</section>");
        let anchor = self.focus.get().map(|id| format!("c{id}"));
        self.view.set_document(&body, anchor.as_deref());
    }

    /// Fetch the images among `attachments` with the sign-in, once each,
    /// and render again as each one lands.
    fn fetch_images(self: &Rc<Self>, attachments: &[Attachment]) {
        for attachment in attachments.iter().filter(|a| a.is_image()) {
            let sgid = attachment.sgid.clone();
            if sgid.is_empty()
                || self.images.borrow().contains_key(&sgid)
                || !self.fetching.borrow_mut().insert(sgid.clone())
            {
                continue;
            }
            let client = self.client.clone();
            let url = attachment.download_url.clone();
            let content_type = attachment.content_type.clone();
            rt::run(
                async move { client.get_bytes(&url).await },
                glib::clone!(
                    #[strong(rename_to = page)]
                    self,
                    move |result| {
                        page.fetching.borrow_mut().remove(&sgid);
                        match result {
                            Ok(bytes) => {
                                let uri =
                                    crate::widgets::richtext_view::data_uri(&content_type, &bytes);
                                page.images.borrow_mut().insert(sgid, uri);
                                page.render();
                            }
                            Err(error) => tracing::warn!("could not fetch an image: {error}"),
                        }
                    }
                ),
            );
        }
    }
}

/// The page: a native title and byline, the document, the composer.
pub fn recording_page(
    window: &SkaliWindow,
    client: Client,
    doc: RecordingDoc,
) -> adw::NavigationPage {
    recording_page_parts(window, client, doc).0
}

/// `recording_page`, with its header bar for the page's own buttons and the
/// page itself, for whoever changes what it shows.
pub fn recording_page_parts(
    window: &SkaliWindow,
    client: Client,
    mut doc: RecordingDoc,
) -> (adw::NavigationPage, adw::HeaderBar, Rc<RecordingPage>) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_margin_start(12);
    column.set_margin_end(12);
    column.set_margin_top(12);
    let title = gtk::Label::builder()
        .label(&doc.title)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["title-1"])
        .build();
    column.append(&title);
    column.append(&byline(&doc.creator, doc.created_at));
    let view = RichTextView::new();
    column.append(view.widget());
    let clamp = adw::Clamp::builder()
        .maximum_size(800)
        .tightening_threshold(600)
        .child(&column)
        .build();

    let attachments = doc.attachments.clone();
    let comments_count = doc.comments_count;
    let recording_id = doc.id;
    let page_title = doc.title.clone();
    let extra_html = std::mem::take(&mut doc.extra_html);
    let page = Rc::new(RecordingPage {
        window: window.downgrade(),
        client: client.clone(),
        doc,
        comments: RefCell::new(Vec::new()),
        images: RefCell::new(HashMap::new()),
        fetching: RefCell::new(HashSet::new()),
        extra_html: RefCell::new(extra_html),
        focus: Cell::new(None),
        view,
    });
    window.watch_recording(&page);
    page.fetch_images(&attachments);
    page.render();

    if comments_count > 0 {
        let path = format!("/recordings/{recording_id}/comments.json");
        rt::run(
            async move { client.get_all::<Comment>(&path).await },
            glib::clone!(
                #[strong]
                page,
                move |result| match result {
                    Ok(comments) => {
                        for comment in &comments {
                            page.fetch_images(&comment.content_attachments);
                        }
                        page.comments.borrow_mut().extend(comments);
                        page.render();
                    }
                    Err(error) => {
                        if let Some(window) = page.window.upgrade() {
                            window.report("could not load the comments", &error);
                        }
                    }
                }
            ),
        );
    }

    let (nav_page, header) = shell_with_header(&page_title, &clamp);
    if let Some(toolbar) = nav_page.child().and_downcast::<adw::ToolbarView>() {
        let composer = comment_composer(&page);
        let composer_clamp = adw::Clamp::builder()
            .maximum_size(800)
            .tightening_threshold(600)
            .child(&composer)
            .build();
        toolbar.add_bottom_bar(&composer_clamp);
    }
    (nav_page, header, page)
}

pub fn byline(
    person: &skali_core::models::Person,
    at: Option<chrono::DateTime<chrono::Utc>>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&avatar(person, 32));
    let name = gtk::Label::builder()
        .label(&person.name)
        .css_classes(["heading"])
        .build();
    let time = gtk::Label::builder()
        .label(time_label(at))
        .css_classes(["dim-label"])
        .build();
    row.append(&name);
    row.append(&time);
    row
}

/// A box to type a comment in, and the button that posts it. The comment
/// joins the document once Basecamp has it.
fn comment_composer(page: &Rc<RecordingPage>) -> gtk::Box {
    let writes_allowed = page.client.writes_allowed();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .sensitive(writes_allowed)
        .build();
    let frame = gtk::ScrolledWindow::builder()
        .child(&text)
        .focusable(false)
        .min_content_height(60)
        .max_content_height(200)
        .propagate_natural_height(true)
        .css_classes(["card"])
        .build();
    let post = gtk::Button::builder()
        .label(gettext("Post comment"))
        .halign(gtk::Align::End)
        .sensitive(writes_allowed)
        .css_classes(["suggested-action", "pill"])
        .tooltip_text(if writes_allowed {
            String::new()
        } else {
            gettext("Writes to this account are turned off.")
        })
        .build();
    let picker = AttachmentPicker::new();
    picker.set_sensitive(writes_allowed);
    post.connect_clicked(glib::clone!(
        #[strong]
        page,
        #[weak]
        text,
        #[strong]
        picker,
        move |button| {
            let buffer = text.buffer();
            let typed = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let html = richtext::text_to_html(&typed);
            let files = picker.files();
            if html.is_empty() && files.is_empty() {
                return;
            }
            button.set_sensitive(false);
            text.set_sensitive(false);
            picker.set_sensitive(false);
            let client = page.client.clone();
            let path = format!("/recordings/{}/comments.json", page.recording_id());
            rt::run(
                async move {
                    let tags = attachments::upload(client.clone(), files).await?;
                    let body = serde_json::json!({ "content": html + &tags.concat() });
                    client.post::<Comment, _>(&path, &body).await
                },
                glib::clone!(
                    #[strong]
                    page,
                    #[weak]
                    text,
                    #[weak]
                    button,
                    #[strong]
                    picker,
                    move |result| {
                        button.set_sensitive(true);
                        text.set_sensitive(true);
                        picker.set_sensitive(true);
                        match result {
                            Ok(comment) => {
                                text.buffer().set_text("");
                                picker.clear();
                                page.add_comment(comment);
                            }
                            Err(error) => {
                                if let Some(window) = page.window.upgrade() {
                                    window.report("could not post the comment", &error);
                                }
                            }
                        }
                    }
                ),
            );
        }
    ));
    // Ctrl+Return posts, as in the chat.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        post,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, modifiers| {
            let is_enter = matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            if is_enter && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                post.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    ));
    text.add_controller(keys);
    // A developer hook: focus the box as soon as the page is on screen.
    if std::env::var_os("SKALI_DEBUG_FOCUS").is_some() {
        text.connect_map(|text| {
            text.grab_focus();
        });
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    picker.widget().set_hexpand(true);
    picker.widget().set_valign(gtk::Align::Center);
    post.set_valign(gtk::Align::Center);
    actions.append(picker.widget());
    actions.append(&post);
    column.append(&frame);
    column.append(&actions);
    column
}
