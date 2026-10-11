//! A recording with a body and comments -- a message, a card, a document,
//! an upload -- as one document in a rich-text view, with the comment
//! composer along the bottom. Images are fetched with the sign-in and
//! inlined; the document is rendered again as they arrive, and as
//! comments do. Boosts sit under the body and each comment; the header
//! bookmarks the recording and subscribes to it.

use super::{avatar, count_label, shell_with_header, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::widgets::attachments::{self, AttachmentPicker};
use crate::widgets::richtext_view::{self, RichTextView};
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::links::PageAction;
use skali_core::models::{Attachment, BookmarkState, Boost, Comment, Person, Subscription};
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
    pub boosts_count: i64,
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
    /// Boosts by the recording they're on: the body's, each comment's.
    boosts: RefCell<HashMap<i64, Vec<Boost>>>,
    /// `RecordingDoc::extra_html`, as last set.
    extra_html: RefCell<String>,
    /// The comment a link to it asked to see, scrolled to on every render.
    focus: Cell<Option<i64>>,
    view: RichTextView,
    /// Pictures the document wanted but the avatar cache didn't have: the
    /// ones asked for (once each), and whether a redraw is due for them.
    faces_missing: RefCell<HashSet<String>>,
    faces_asked: RefCell<HashSet<String>>,
    redraw_due: Cell<bool>,
    this: std::rc::Weak<RecordingPage>,
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
        if let Some(window) = self.window.upgrade() {
            window.keep_for_search(&self.client, std::slice::from_ref(&comment));
        }
        self.fetch_images(&comment.content_attachments);
        self.comments.borrow_mut().push(comment);
        self.render();
    }

    /// A picture for the document, noting a miss so it's fetched.
    fn face(&self, url: &str) -> Option<String> {
        let face = richtext_view::face(url);
        if face.is_none() && !url.is_empty() {
            self.faces_missing.borrow_mut().insert(url.to_string());
        }
        face
    }

    /// Fetch the pictures the last render missed, and render again once
    /// they're in (once for all that arrive together).
    fn fetch_faces(&self) {
        let missing: Vec<String> = self.faces_missing.take().into_iter().collect();
        for url in missing {
            if !self.faces_asked.borrow_mut().insert(url.clone()) {
                continue;
            }
            let this = self.this.clone();
            crate::avatars::fetch(&url, move || {
                let Some(page) = this.upgrade() else {
                    return;
                };
                if page.redraw_due.replace(true) {
                    return;
                }
                glib::idle_add_local_once(move || {
                    page.redraw_due.set(false);
                    page.render();
                });
            });
        }
    }

    /// A person's picture for the document, from the avatar cache, or their
    /// initial on a disc when it isn't there yet.
    fn face_html(&self, person: &Person, class: &str) -> String {
        match self.face(&person.avatar_url) {
            Some(src) => format!("<img class=\"{class}\" src=\"{src}\" alt=\"\">"),
            None => format!(
                "<span class=\"{class} initials\">{}</span>",
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
        }
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
            richtext::expand_attachments(&self.doc.content, resolve, |url| self.face(url))
        );
        body.push_str(&self.boosts_html(self.doc.id));
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
            let avatar = self.face_html(person, "avatar");
            body.push_str(&format!(
                "<div class=\"comment\" id=\"c{}\"><div class=\"byline\">{avatar}<b>{}</b><time>{}</time></div><div class=\"body\">{}</div>{}</div>",
                comment.recording.id,
                richtext::escape(&person.name),
                richtext::escape(&time_label(comment.recording.created_at)),
                richtext::expand_attachments(&comment.content, resolve, |url| self.face(url)),
                self.boosts_html(comment.recording.id)
            ));
        }
        body.push_str("</section>");
        let anchor = self.focus.get().map(|id| format!("c{id}"));
        self.view.set_document(&body, anchor.as_deref());
        self.fetch_faces();
    }

    /// The boosts on one recording as chips: yours take the boost back,
    /// and the last one adds a boost.
    fn boosts_html(&self, recording: i64) -> String {
        let boosts = self.boosts.borrow();
        let list = boosts
            .get(&recording)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let can_write = self.client.writes_allowed();
        if list.is_empty() && !can_write {
            return String::new();
        }
        let me = self.me();
        let mut html = String::from("<div class=\"boosts\">");
        for boost in list {
            // The booster's face and what they boosted with, as Basecamp
            // shows it; the name is the tooltip.
            let content = richtext::escape(&boost.content);
            let face = self.face_html(&boost.booster, "face");
            if can_write && me == Some(boost.booster.id) {
                let action = PageAction::Unboost {
                    recording,
                    boost: boost.id,
                };
                html.push_str(&format!(
                    "<a class=\"boost mine\" href=\"{}\" title=\"{}\">{face}{content}</a>",
                    action.uri(),
                    richtext::escape(&gettext("Take your boost back"))
                ));
            } else {
                html.push_str(&format!(
                    "<span class=\"boost\" title=\"{}\">{face}{content}</span>",
                    richtext::escape(&boost.booster.name)
                ));
            }
        }
        if can_write {
            html.push_str(&format!(
                "<a class=\"boost add\" href=\"{}\">{}</a>",
                PageAction::Boost(recording).uri(),
                richtext::escape(&gettext("+ Boost"))
            ));
        }
        html.push_str("</div>");
        html
    }

    /// Your person id in this account, once the window knows it.
    fn me(&self) -> Option<i64> {
        self.window
            .upgrade()?
            .my_person_id(self.client.account_id())
    }

    /// Whether `recording` is the body or a comment shown here: a `skali:`
    /// link about anything else didn't come from this page.
    fn shows(&self, recording: i64) -> bool {
        recording == self.doc.id
            || self
                .comments
                .borrow()
                .iter()
                .any(|c| c.recording.id == recording)
    }

    fn fetch_boosts(self: &Rc<Self>, recording: i64) {
        let client = self.client.clone();
        let path = format!("/recordings/{recording}/boosts.json");
        rt::run(
            async move { client.get_all::<Boost>(&path).await },
            glib::clone!(
                #[strong(rename_to = page)]
                self,
                move |result| match result {
                    Ok(boosts) => {
                        page.boosts.borrow_mut().insert(recording, boosts);
                        page.render();
                    }
                    Err(error) =>
                        tracing::warn!("could not load the boosts on {recording}: {error}"),
                }
            ),
        );
    }

    /// A `skali:` link in the document was clicked.
    fn act(self: &Rc<Self>, uri: &str) {
        match PageAction::parse(uri) {
            Some(PageAction::Boost(recording)) if self.shows(recording) => {
                self.boost_dialog(recording)
            }
            Some(PageAction::Unboost { recording, boost })
                if self.is_my_boost(recording, boost) =>
            {
                self.unboost(recording, boost)
            }
            _ => tracing::warn!("ignored a page link: {uri}"),
        }
    }

    fn is_my_boost(&self, recording: i64, boost: i64) -> bool {
        let me = self.me();
        me.is_some()
            && self.boosts.borrow().get(&recording).is_some_and(|boosts| {
                boosts
                    .iter()
                    .any(|b| b.id == boost && Some(b.booster.id) == me)
            })
    }

    /// Ask what to boost with, then boost.
    fn boost_dialog(self: &Rc<Self>, recording: i64) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let page = Rc::downgrade(self);
        crate::widgets::boosts::ask(&window, move |content| {
            if let Some(page) = page.upgrade() {
                page.boost(recording, content);
            }
        });
    }

    fn boost(self: &Rc<Self>, recording: i64, content: String) {
        let client = self.client.clone();
        let path = format!("/recordings/{recording}/boosts.json");
        rt::run(
            async move {
                client
                    .post::<Boost, _>(&path, &serde_json::json!({ "content": content }))
                    .await
            },
            glib::clone!(
                #[strong(rename_to = page)]
                self,
                move |result| match result {
                    Ok(boost) => {
                        page.boosts
                            .borrow_mut()
                            .entry(recording)
                            .or_default()
                            .push(boost);
                        page.show_after_boost(recording);
                    }
                    Err(error) => {
                        if let Some(window) = page.window.upgrade() {
                            window.report(&format!("could not boost {recording}"), &error);
                        }
                    }
                }
            ),
        );
    }

    fn unboost(self: &Rc<Self>, recording: i64, boost: i64) {
        let client = self.client.clone();
        rt::run(
            async move { client.delete(&format!("/boosts/{boost}.json")).await },
            glib::clone!(
                #[strong(rename_to = page)]
                self,
                move |result| match result {
                    Ok(()) => {
                        if let Some(boosts) = page.boosts.borrow_mut().get_mut(&recording) {
                            boosts.retain(|b| b.id != boost);
                        }
                        page.show_after_boost(recording);
                    }
                    Err(error) => {
                        if let Some(window) = page.window.upgrade() {
                            window.report(&format!("could not take back boost {boost}"), &error);
                        }
                    }
                }
            ),
        );
    }

    /// Draw again, staying at the comment that changed rather than going
    /// back to the top.
    fn show_after_boost(&self, recording: i64) {
        if recording != self.doc.id {
            self.focus.set(Some(recording));
        }
        self.render();
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
    let boosts_count = doc.boosts_count;
    let recording_id = doc.id;
    let page_title = doc.title.clone();
    let extra_html = std::mem::take(&mut doc.extra_html);
    let page = Rc::new_cyclic(|this| RecordingPage {
        window: window.downgrade(),
        client: client.clone(),
        doc,
        comments: RefCell::new(Vec::new()),
        images: RefCell::new(HashMap::new()),
        fetching: RefCell::new(HashSet::new()),
        boosts: RefCell::new(HashMap::new()),
        extra_html: RefCell::new(extra_html),
        focus: Cell::new(None),
        view,
        faces_missing: RefCell::new(HashSet::new()),
        faces_asked: RefCell::new(HashSet::new()),
        redraw_due: Cell::new(false),
        this: this.clone(),
    });
    window.watch_recording(&page);
    page.view.connect_action(glib::clone!(
        #[weak]
        page,
        move |uri| page.act(uri)
    ));
    page.fetch_images(&attachments);
    page.render();
    if boosts_count > 0 {
        page.fetch_boosts(recording_id);
    }

    if comments_count > 0 {
        let path = format!("/recordings/{recording_id}/comments.json");
        rt::run(
            async move { client.get_all::<Comment>(&path).await },
            glib::clone!(
                #[strong]
                page,
                move |result| match result {
                    Ok(comments) => {
                        if let Some(window) = page.window.upgrade() {
                            window.keep_for_search(&page.client, &comments);
                        }
                        for comment in &comments {
                            page.fetch_images(&comment.content_attachments);
                            if comment.recording.boosts_count > 0 {
                                page.fetch_boosts(comment.recording.id);
                            }
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
    header_toggles(&page, &header);
    (nav_page, header, page)
}

/// Bookmark and subscribe, as toggles that follow Basecamp: each starts
/// from the recording's state and puts itself back if a change fails.
fn header_toggles(page: &Rc<RecordingPage>, header: &adw::HeaderBar) {
    let id = page.doc.id;
    let bookmark = RemoteToggle {
        path: format!("/recordings/{id}/bookmark.json"),
        on_icon: "starred-symbolic",
        off_icon: "non-starred-symbolic",
        on_tip: gettext("Bookmarked"),
        off_tip: gettext("Bookmark"),
        what: "bookmark",
    };
    let subscribe = RemoteToggle {
        path: format!("/recordings/{id}/subscription.json"),
        on_icon: "preferences-system-notifications-symbolic",
        off_icon: "notifications-disabled-symbolic",
        on_tip: gettext("Subscribed: you're notified of new comments"),
        off_tip: gettext("Not subscribed"),
        what: "subscription",
    };
    header.pack_end(&subscribe.build(page, |json: &str| {
        serde_json::from_str::<Subscription>(json).map(|s| s.subscribed)
    }));
    header.pack_end(&bookmark.build(page, |json: &str| {
        serde_json::from_str::<BookmarkState>(json).map(|b| b.bookmarked)
    }));
}

/// A header toggle for a yes/no that lives in Basecamp at `path`: GET
/// reads it, POST turns it on, DELETE off.
struct RemoteToggle {
    path: String,
    on_icon: &'static str,
    off_icon: &'static str,
    on_tip: String,
    off_tip: String,
    /// For the log: "bookmark", "subscription".
    what: &'static str,
}

impl RemoteToggle {
    fn build(
        self,
        page: &Rc<RecordingPage>,
        read: fn(&str) -> serde_json::Result<bool>,
    ) -> gtk::ToggleButton {
        let toggle = Rc::new(self);
        let button = gtk::ToggleButton::builder()
            .icon_name(toggle.off_icon)
            .tooltip_text(&toggle.off_tip)
            .sensitive(false)
            .build();
        let writes_allowed = page.client.writes_allowed();
        let client = page.client.clone();
        let path = toggle.path.clone();
        rt::run(
            async move { client.call_raw("GET", &path, None).await },
            glib::clone!(
                #[weak]
                button,
                #[strong]
                toggle,
                move |result| match result.map(|json| read(&json)) {
                    Ok(Ok(on)) => {
                        button.set_active(on);
                        toggle.show(&button);
                        button.set_sensitive(writes_allowed);
                        if !writes_allowed {
                            button.set_tooltip_text(Some(&gettext(
                                "Writes to this account are turned off.",
                            )));
                        }
                    }
                    Ok(Err(error)) => tracing::warn!("could not read the {}: {error}", toggle.what),
                    Err(error) => tracing::warn!("could not load the {}: {error}", toggle.what),
                }
            ),
        );
        let client = page.client.clone();
        let window = page.window.clone();
        button.connect_toggled(move |button| {
            if !button.is_sensitive() {
                return;
            }
            let on = button.is_active();
            toggle.show(button);
            button.set_sensitive(false);
            let client = client.clone();
            let path = toggle.path.clone();
            rt::run(
                async move {
                    if on {
                        client.post_empty(&path).await
                    } else {
                        client.delete(&path).await
                    }
                },
                glib::clone!(
                    #[weak]
                    button,
                    #[strong]
                    toggle,
                    #[strong]
                    window,
                    move |result| {
                        if let Err(error) = result {
                            if let Some(window) = window.upgrade() {
                                window.report(
                                    &format!(
                                        "could not change the {} at {}",
                                        toggle.what, toggle.path
                                    ),
                                    &error,
                                );
                            }
                            button.set_active(!on);
                            toggle.show(&button);
                        }
                        button.set_sensitive(true);
                    }
                ),
            );
        });
        button
    }

    fn show(&self, button: &gtk::ToggleButton) {
        let on = button.is_active();
        button.set_icon_name(if on { self.on_icon } else { self.off_icon });
        button.set_tooltip_text(Some(if on { &self.on_tip } else { &self.off_tip }));
    }
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
