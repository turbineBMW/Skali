//! A Campfire: its lines, oldest at the top, newest at the bottom where
//! the eye lands. The API pages newest-first; each page is reversed for
//! display, and scrolling to the top fetches the next (older) page and
//! prepends it, keeping what was on screen where it was. Each line is a
//! bubble, yours on the right; boosts sit inside it, and hovering a line
//! shows the button that adds one. An upload line shows its images (GIFs
//! playing; a click opens one at the window's size) and names its other
//! files. A link's preview and a video player, which Basecamp puts in the
//! line, are cards that open the page. The composer has an emoji picker
//! and a GIF picker; a GIF goes up as a Campfire upload.

use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::widgets::inline_label::InlineLabel;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::embed::{self, Embed};
use skali_core::error::ApiError;
use skali_core::gif::{self, Gif, GifError};
use skali_core::models::{Boost, ChatAttachment, ChatLine, Person};
use skali_core::richtext;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The most older pages fetched looking for a line a link points at.
const FOCUS_PAGES: usize = 20;
/// How close to the top (in pixels) asks for the older page.
const SCROLLBACK_THRESHOLD: f64 = 120.0;
/// A line's avatar, and the gap left for one in a run.
const AVATAR_SIZE: i32 = 40;
/// The widest a bubble gets.
const BUBBLE_WIDTH: i32 = 680;
/// The room a bubble leaves on the side it isn't on.
const LANE_INDENT: i32 = 64;
/// The box an uploaded image is scaled down to fit.
const IMAGE_WIDTH: i32 = 360;
const IMAGE_HEIGHT: i32 = 320;
/// A page preview's picture, beside its title.
const PREVIEW_WIDTH: i32 = 128;
const PREVIEW_HEIGHT: i32 = 84;

/// Why a GIF didn't make it: KLIPY's side, or Basecamp's.
enum GifSendError {
    Gif(GifError),
    Api(ApiError),
}

/// What the page remembers between fetches, shared by the scroll handler,
/// the fetch callbacks and the live feed.
pub struct ChatView {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    title: String,
    lines_box: gtk::Box,
    scroller: gtk::ScrolledWindow,
    stack: gtk::Stack,
    /// The next (older) page's URL; None once the oldest page is shown.
    next: RefCell<Option<String>>,
    loading: Cell<bool>,
    /// Lines shown so far, to tell an empty chat from a short one.
    shown: Cell<usize>,
    chat_id: i64,
    /// The creator and time of the newest line, so a posted line joins
    /// its run.
    newest: RefCell<Option<(i64, chrono::DateTime<chrono::Utc>)>>,
    /// Every line on screen, so a live wake-up for one we posted (or
    /// already received) is dropped.
    line_ids: RefCell<HashSet<i64>>,
    /// Each line's row of boost chips and the boosts in it, by line id.
    boosts: RefCell<HashMap<i64, (gtk::Box, Vec<Boost>)>>,
    /// You in this account: from the window, or fetched by the page.
    me: RefCell<Option<Person>>,
    /// Whether the newest line is in view, so the page follows it when
    /// the lines grow: a live line, boosts or an image that load late.
    pinned: Cell<bool>,
    /// Each line's row, by line id, to scroll to one.
    rows: RefCell<HashMap<i64, gtk::Widget>>,
    /// A line to scroll to once it's loaded, and how many older pages
    /// have been fetched looking for it.
    focus: Cell<Option<i64>>,
    focus_pages: Cell<usize>,
}

impl ChatView {
    /// Post what was typed, and show it once Basecamp has it.
    fn send(self: &Rc<Self>, entry: &gtk::Entry) {
        let text = entry.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        entry.set_sensitive(false);
        let client = self.client.clone();
        let path = format!("/chats/{}/lines.json", self.chat_id);
        let body = serde_json::json!({ "content": text });
        rt::run(
            async move { client.post::<ChatLine, _>(&path, &body).await },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                #[weak]
                entry,
                move |result| {
                    entry.set_sensitive(true);
                    match result {
                        Ok(line) => {
                            entry.set_text("");
                            view.append_line(&line);
                            view.stack.set_visible_child_name("lines");
                            let adjustment = view.scroller.vadjustment();
                            glib::idle_add_local_once(move || {
                                adjustment.set_value(adjustment.upper() - adjustment.page_size());
                            });
                        }
                        Err(error) => {
                            if let Some(window) = view.window.upgrade() {
                                window.report(
                                    &format!("could not post to the chat {}", view.title),
                                    &error,
                                );
                            }
                            entry.grab_focus();
                        }
                    }
                }
            ),
        );
    }

    /// Fetch a GIF from KLIPY and post it to the chat as an upload. Once
    /// Basecamp has it, it joins the picker's recently sent.
    fn send_gif(self: &Rc<Self>, gif: Gif, button: &gtk::MenuButton) {
        button.set_sensitive(false);
        let client = self.client.clone();
        let chat_id = self.chat_id;
        let url = gif.url.clone();
        // KLIPY's file name, unless it has none worth keeping.
        let name = url
            .rsplit('/')
            .next()
            .filter(|name| name.ends_with(".gif"))
            .unwrap_or("animation.gif")
            .to_string();
        rt::run(
            async move {
                let bytes = gif::download(&url).await.map_err(GifSendError::Gif)?;
                client
                    .upload_to_chat(chat_id, &name, "image/gif", bytes)
                    .await
                    .map_err(GifSendError::Api)
            },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                #[weak]
                button,
                move |result| {
                    button.set_sensitive(true);
                    let Some(window) = view.window.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(line) => {
                            gif::remember(&window.db().borrow(), &gif);
                            view.pinned.set(true);
                            view.append_live(&line);
                            view.stack.set_visible_child_name("lines");
                        }
                        Err(GifSendError::Api(error)) => window.report(
                            &format!("could not post a GIF to the chat {}", view.title),
                            &error,
                        ),
                        Err(GifSendError::Gif(error)) => {
                            tracing::error!("could not fetch the GIF {}: {error}", gif.url);
                            window.toast(&crate::i18n::gif_failure_message(&error));
                        }
                    }
                }
            ),
        );
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// Scroll to a line, fetching older pages until it's there (as far
    /// back as `FOCUS_PAGES`), and mark it for a moment.
    pub fn focus_line(self: &Rc<Self>, line_id: i64) {
        self.focus.set(Some(line_id));
        self.focus_pages.set(0);
        self.seek_focus();
    }

    /// The next step towards the line asked for: scroll to it if it's
    /// shown, or fetch the page before. Runs again as each page arrives.
    fn seek_focus(self: &Rc<Self>) {
        let Some(line_id) = self.focus.get() else {
            return;
        };
        let row = self.rows.borrow().get(&line_id).cloned();
        if let Some(row) = row {
            self.focus.set(None);
            self.scroll_to(&row);
            return;
        }
        // The first page comes by itself; a fetch in flight calls back.
        if self.shown.get() == 0 || self.loading.get() {
            return;
        }
        if self.next.borrow().is_none() || self.focus_pages.get() >= FOCUS_PAGES {
            self.focus.set(None);
            if let Some(window) = self.window.upgrade() {
                window.toast(&gettext("That line is too far back to show here."));
            }
            return;
        }
        self.focus_pages.set(self.focus_pages.get() + 1);
        self.load_older();
    }

    /// Bring a row to the upper part of the view and outline it briefly.
    fn scroll_to(self: &Rc<Self>, row: &gtk::Widget) {
        self.pinned.set(false);
        row.add_css_class("focused");
        let place = glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[weak]
            row,
            move || {
                let Some(viewport) = view.scroller.child() else {
                    return;
                };
                let Some(point) = row.compute_point(&viewport, &gtk::graphene::Point::zero())
                else {
                    return;
                };
                let adjustment = view.scroller.vadjustment();
                let target =
                    adjustment.value() + f64::from(point.y()) - adjustment.page_size() / 4.0;
                adjustment
                    .set_value(target.clamp(0.0, adjustment.upper() - adjustment.page_size()));
            }
        );
        // Once laid out, and again when late pictures above have settled.
        glib::idle_add_local_once(place.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), place);
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(2500),
            glib::clone!(
                #[weak]
                row,
                move || row.remove_css_class("focused")
            ),
        );
    }

    /// A line that just arrived over the feed. Dropped if it's already
    /// shown (we posted it, or the feed repeated itself).
    pub fn append_live(self: &Rc<Self>, line: &ChatLine) {
        if self.line_ids.borrow().contains(&line.recording.id) {
            return;
        }
        // At the bottom, the page follows it down (see `pinned`).
        self.append_line(line);
    }

    /// A new line at the bottom: ours just posted, or one arriving live.
    fn append_line(self: &Rc<Self>, line: &ChatLine) {
        if let Some(window) = self.window.upgrade() {
            window.keep_for_search(&self.client, std::slice::from_ref(line));
        }
        self.line_ids.borrow_mut().insert(line.recording.id);
        let previous = *self.newest.borrow();
        self.lines_box
            .append(&self.line_row(line, joins(previous, line)));
        *self.newest.borrow_mut() = line
            .recording
            .created_at
            .map(|at| (line.recording.creator.id, at));
        self.shown.set(self.shown.get() + 1);
    }

    /// Fetch the next older page, if there is one and none is in flight.
    fn load_older(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let Some(next) = self.next.borrow_mut().take() else {
            return;
        };
        self.loading.set(true);
        let adjustment = self.scroller.vadjustment();
        let before = adjustment.upper();
        let client = self.client.clone();
        rt::run(
            async move { client.get_page::<ChatLine>(&next).await },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                move |result| {
                    view.loading.set(false);
                    match result {
                        Ok(page) => {
                            if let Some(window) = view.window.upgrade() {
                                window.keep_for_search(&view.client, &page.items);
                            }
                            *view.next.borrow_mut() = page.next;
                            let first_page = view.shown.get() == 0;
                            if first_page && page.items.is_empty() {
                                view.stack.set_visible_child_name("empty");
                                return;
                            }
                            if first_page {
                                *view.newest.borrow_mut() = page.items.first().and_then(|line| {
                                    line.recording
                                        .created_at
                                        .map(|at| (line.recording.creator.id, at))
                                });
                            }
                            view.shown.set(view.shown.get() + page.items.len());
                            view.line_ids
                                .borrow_mut()
                                .extend(page.items.iter().map(|line| line.recording.id));
                            view.prepend_lines(page.items);
                            view.stack.set_visible_child_name("lines");
                            // Once laid out: the first page starts at the
                            // bottom, a later one keeps the old top line in
                            // place, and a viewport still not full asks for more.
                            glib::idle_add_local_once(glib::clone!(
                                #[strong]
                                view,
                                move || {
                                    let adjustment = view.scroller.vadjustment();
                                    if first_page {
                                        adjustment
                                            .set_value(adjustment.upper() - adjustment.page_size());
                                    } else {
                                        let grown = adjustment.upper() - before;
                                        adjustment.set_value(adjustment.value() + grown.max(0.0));
                                    }
                                    if view.focus.get().is_some() {
                                        view.seek_focus();
                                    } else if adjustment.upper() <= adjustment.page_size() {
                                        view.load_older();
                                    }
                                }
                            ));
                        }
                        Err(error) => {
                            if let Some(window) = view.window.upgrade() {
                                window.report(
                                    &format!("could not load the chat {}", view.title),
                                    &error,
                                );
                            }
                            if view.shown.get() == 0 {
                                view.stack.set_visible_child_name("empty");
                            }
                        }
                    }
                }
            ),
        );
    }

    /// Put a newest-first page of lines above what's shown, oldest first.
    /// A run by one person that straddles two pages keeps a header on each
    /// side; simpler than merging, and rare.
    fn prepend_lines(self: &Rc<Self>, mut lines: Vec<ChatLine>) {
        lines.reverse();
        let mut rows: Vec<gtk::Widget> = Vec::with_capacity(lines.len());
        let mut previous: Option<(i64, chrono::DateTime<chrono::Utc>)> = None;
        for line in &lines {
            rows.push(self.line_row(line, joins(previous, line)));
            previous = line
                .recording
                .created_at
                .map(|at| (line.recording.creator.id, at));
        }
        for row in rows.into_iter().rev() {
            self.lines_box.prepend(&row);
        }
    }

    /// Your person id in this account, once known.
    fn me(&self) -> Option<i64> {
        self.me.borrow().as_ref().map(|person| person.id)
    }

    /// The first page, once it's known who you are: your lines sit on the
    /// other side, and lines mentioning you stand out. A chat opened at
    /// launch can beat the window's own profile fetch; then the page asks.
    fn load_first(self: &Rc<Self>) {
        let known = self
            .window
            .upgrade()
            .and_then(|window| window.my_person(self.client.account_id()));
        if known.is_some() {
            *self.me.borrow_mut() = known;
            self.load_older();
            return;
        }
        let client = self.client.clone();
        rt::run(
            async move { client.get::<Person>("/my/profile.json").await },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                move |result| {
                    match result {
                        Ok(person) => *view.me.borrow_mut() = Some(person),
                        Err(error) => tracing::warn!(
                            "could not load your profile for the chat {}: {error}",
                            view.title
                        ),
                    }
                    view.load_older();
                }
            ),
        );
    }

    fn fetch_boosts(self: &Rc<Self>, line_id: i64) {
        let client = self.client.clone();
        let path = format!("/recordings/{line_id}/boosts.json");
        rt::run(
            async move { client.get_all::<Boost>(&path).await },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                move |result| match result {
                    Ok(boosts) => {
                        if let Some((_, shown)) = view.boosts.borrow_mut().get_mut(&line_id) {
                            *shown = boosts;
                        }
                        view.show_boosts(line_id);
                    }
                    Err(error) => {
                        tracing::warn!("could not load the boosts on line {line_id}: {error}")
                    }
                }
            ),
        );
    }

    /// Lay out a line's chips again from what's known.
    fn show_boosts(self: &Rc<Self>, line_id: i64) {
        let Some((chips, boosts)) = self.boosts.borrow().get(&line_id).cloned() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let on_add = {
            let weak = weak.clone();
            move |add: &gtk::Button| {
                if let Some(view) = weak.upgrade() {
                    view.ask_boost(line_id, add);
                }
            }
        };
        crate::widgets::boosts::fill_chips(
            &chips,
            &boosts,
            self.me(),
            self.client.writes_allowed(),
            move |boost_id| {
                if let Some(view) = weak.upgrade() {
                    view.unboost(line_id, boost_id);
                }
            },
            on_add,
        );
    }

    /// Ask what to boost a line with, then boost it.
    fn ask_boost(self: &Rc<Self>, line_id: i64, button: &gtk::Button) {
        let weak = Rc::downgrade(self);
        crate::widgets::boosts::ask(button, move |content| {
            if let Some(view) = weak.upgrade() {
                view.boost(line_id, content);
            }
        });
    }

    fn boost(self: &Rc<Self>, line_id: i64, content: String) {
        let client = self.client.clone();
        let path = format!("/recordings/{line_id}/boosts.json");
        rt::run(
            async move {
                client
                    .post::<Boost, _>(&path, &serde_json::json!({ "content": content }))
                    .await
            },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                move |result| match result {
                    Ok(boost) => {
                        if let Some((_, boosts)) = view.boosts.borrow_mut().get_mut(&line_id) {
                            boosts.push(boost);
                        }
                        view.show_boosts(line_id);
                    }
                    Err(error) => {
                        if let Some(window) = view.window.upgrade() {
                            window.report(&format!("could not boost chat line {line_id}"), &error);
                        }
                    }
                }
            ),
        );
    }

    fn unboost(self: &Rc<Self>, line_id: i64, boost_id: i64) {
        let client = self.client.clone();
        rt::run(
            async move { client.delete(&format!("/boosts/{boost_id}.json")).await },
            glib::clone!(
                #[strong(rename_to = view)]
                self,
                move |result| {
                    match result {
                        Ok(()) => {
                            if let Some((_, boosts)) = view.boosts.borrow_mut().get_mut(&line_id) {
                                boosts.retain(|b| b.id != boost_id);
                            }
                        }
                        Err(error) => {
                            if let Some(window) = view.window.upgrade() {
                                window.report(
                                    &format!("could not take back boost {boost_id}"),
                                    &error,
                                );
                            }
                        }
                    }
                    // Either way: the chip comes back clickable, or goes.
                    view.show_boosts(line_id);
                }
            ),
        );
    }

    /// A file in an upload line: an image shown (fetched with the sign-in,
    /// a GIF playing), anything else by name, opening Basecamp's page for
    /// it in the browser.
    fn attachment(&self, line: &ChatLine, attachment: &ChatAttachment) -> gtk::Widget {
        if !attachment.is_image() {
            let button = gtk::Button::builder()
                .child(
                    &adw::ButtonContent::builder()
                        .icon_name("mail-attachment-symbolic")
                        .label(&attachment.filename)
                        .build(),
                )
                .tooltip_text(gettext("Open in the browser"))
                .halign(gtk::Align::Start)
                .css_classes(["flat"])
                .build();
            let app_url = line.recording.app_url.clone();
            button.connect_clicked(move |button| {
                if let Some(window) = button.root().and_downcast::<SkaliWindow>() {
                    window.open_in_browser(&app_url);
                }
            });
            return button.upcast();
        }
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .overflow(gtk::Overflow::Hidden)
            .halign(gtk::Align::Start)
            .alternative_text(&attachment.filename)
            .tooltip_text(&attachment.filename)
            .css_classes(["chat-image"])
            .build();
        // The bytes, once here, for the viewer a click opens.
        let shown: Rc<RefCell<Option<Vec<u8>>>> = Rc::default();
        let click = gtk::GestureClick::new();
        click.connect_released(glib::clone!(
            #[strong]
            shown,
            #[strong(rename_to = filename)]
            attachment.filename,
            move |gesture, _, _, _| {
                let (Some(picture), Some(bytes)) = (gesture.widget(), shown.borrow().clone())
                else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                crate::widgets::image_viewer::show(&picture, &filename, &bytes);
            }
        ));
        picture.add_controller(click);
        let client = self.client.clone();
        let url = attachment.download_url.clone();
        let filename = attachment.filename.clone();
        rt::run(
            async move { client.get_bytes(&url).await },
            glib::clone!(
                #[weak]
                picture,
                move |result| match result {
                    Ok(bytes) => match crate::widgets::gifs::show_image(&picture, &bytes) {
                        Some((width, height)) => {
                            picture.set_cursor_from_name(Some("zoom-in"));
                            *shown.borrow_mut() = Some(bytes);
                            let (width, height) = (width.max(1) as f64, height.max(1) as f64);
                            let scale = (IMAGE_WIDTH as f64 / width)
                                .min(IMAGE_HEIGHT as f64 / height)
                                .min(1.0);
                            picture.set_size_request(
                                (width * scale).round() as i32,
                                (height * scale).round() as i32,
                            );
                        }
                        None => tracing::warn!("the chat image {filename} didn't decode"),
                    },
                    Err(error) => {
                        tracing::warn!("could not fetch the chat image {filename}: {error}")
                    }
                }
            ),
        );
        picture.upcast()
    }

    /// A link's preview, a video or a post under the line's text, as a
    /// card; a click opens the page (on Skali's own page, if it has one).
    /// Its pictures come from the open web, a video's title from the
    /// site's oEmbed, and a post from the copy Basecamp keeps of it.
    fn embed_card(embed: &Embed) -> gtk::Widget {
        let href = embed.href().to_string();
        let card = gtk::Button::builder()
            .tooltip_text(&href)
            .halign(gtk::Align::Start)
            .css_classes(["embed-card"])
            .build();
        // A post's own address comes with the post.
        let target = Rc::new(RefCell::new(href.clone()));
        card.connect_clicked(glib::clone!(
            #[strong]
            target,
            move |card| {
                let href = target.borrow().clone();
                if let Some(window) = card.root().and_downcast::<SkaliWindow>()
                    && !window.open_app_url(&href)
                {
                    window.open_in_browser(&href);
                }
            }
        ));
        let text_label = |text: &str, classes: &[&str], lines: i32| {
            gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(lines)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(44)
                .visible(!text.is_empty())
                .css_classes(classes)
                .build()
        };
        let picture = |width: i32, height: i32| {
            gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .can_shrink(true)
                .overflow(gtk::Overflow::Hidden)
                .width_request(width)
                .height_request(height)
                .valign(gtk::Align::Center)
                .build()
        };
        match embed {
            Embed::Page {
                title,
                caption,
                image,
                ..
            } => {
                let words = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(2)
                    .hexpand(true)
                    .valign(gtk::Align::Center)
                    .build();
                let host = host_of(&href);
                words.append(&text_label(
                    if title.is_empty() { &host } else { title },
                    &["heading"],
                    2,
                ));
                words.append(&text_label(caption, &["dim-label"], 3));
                words.append(&text_label(&host, &["dim-label", "caption"], 1));
                let row = gtk::Box::builder().spacing(12).build();
                row.append(&words);
                if !image.is_empty() {
                    let picture = picture(PREVIEW_WIDTH, PREVIEW_HEIGHT);
                    picture.set_visible(false);
                    row.append(&picture);
                    fetch_picture(&picture, image.clone());
                }
                card.set_child(Some(&row));
            }
            Embed::Video {
                thumbnail, site, ..
            } => {
                let still = picture(IMAGE_WIDTH, IMAGE_WIDTH * 9 / 16);
                let play = gtk::Image::builder()
                    .icon_name("media-playback-start-symbolic")
                    .pixel_size(24)
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .css_classes(["embed-play"])
                    .build();
                let overlay = gtk::Overlay::builder().child(&still).build();
                overlay.add_overlay(&play);
                let title = text_label("", &["heading"], 2);
                let byline = text_label(site, &["dim-label", "caption"], 1);
                let column = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(4)
                    .build();
                column.append(&overlay);
                column.append(&title);
                column.append(&byline);
                card.set_child(Some(&column));
                if !thumbnail.is_empty() {
                    fetch_picture(&still, thumbnail.clone());
                }
                if let Some(oembed) = embed.oembed_url() {
                    let site = site.clone();
                    let fetch_still = thumbnail.is_empty();
                    rt::run(
                        async move { embed::video_info(&oembed).await },
                        glib::clone!(
                            #[weak]
                            title,
                            #[weak]
                            byline,
                            #[weak]
                            still,
                            move |result| match result {
                                Ok(info) => {
                                    title.set_label(&info.title);
                                    title.set_visible(!info.title.is_empty());
                                    if !info.author_name.is_empty() {
                                        byline.set_label(&format!("{site} · {}", info.author_name));
                                    }
                                    if fetch_still && !info.thumbnail_url.is_empty() {
                                        fetch_picture(&still, info.thumbnail_url);
                                    }
                                }
                                Err(error) => {
                                    tracing::debug!("no oEmbed for a {site} video: {error}")
                                }
                            }
                        ),
                    );
                }
            }
            Embed::Post { src } => {
                let author = text_label("", &["heading"], 2);
                let words = text_label(&gettext("Embedded post"), &["dim-label"], 12);
                let byline = text_label("", &["dim-label", "caption"], 1);
                let column = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(6)
                    .build();
                column.append(&author);
                column.append(&words);
                column.append(&byline);
                card.set_child(Some(&column));
                let src = src.clone();
                rt::run(
                    {
                        let src = src.clone();
                        async move { embed::fetch_post(&src).await }
                    },
                    glib::clone!(
                        #[weak]
                        card,
                        move |result| match result {
                            Ok(Some(post)) => {
                                author.set_label(&post.author);
                                author.set_visible(!post.author.is_empty());
                                words.set_label(&post.text);
                                words.remove_css_class("dim-label");
                                words.set_visible(!post.text.is_empty());
                                let site = host_of(&post.href);
                                let line = [site.as_str(), post.date.as_str()]
                                    .into_iter()
                                    .filter(|part| !part.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" · ");
                                byline.set_label(&line);
                                byline.set_visible(!line.is_empty());
                                if !post.href.is_empty() {
                                    card.set_tooltip_text(Some(&post.href));
                                    *target.borrow_mut() = post.href;
                                }
                            }
                            Ok(None) => tracing::debug!("the embed at {src} isn't one Skali reads"),
                            Err(error) =>
                                tracing::debug!("could not fetch the embed {src}: {error}"),
                        }
                    ),
                );
            }
        }
        card.upcast()
    }

    /// A click on someone's face or name opens what they're up to in the
    /// project, from its AI summary; a ping has no project, so no page.
    fn open_person_on_click(&self, widget: &impl IsA<gtk::Widget>, line: &ChatLine) {
        let bucket = &line.recording.bucket;
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if bucket.kind != "Project" || super::summary::backend(&window).is_none() {
            return;
        }
        let person = line.recording.creator.clone();
        widget.set_tooltip_text(Some(&crate::i18n::format(
            &gettext("What {name} is up to"),
            &[("name", &person.name)],
        )));
        widget.set_cursor_from_name(Some("pointer"));
        let click = gtk::GestureClick::new();
        let client = self.client.clone();
        let project_id = bucket.id;
        click.connect_released(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(window) = gesture
                .widget()
                .and_then(|w| w.root())
                .and_downcast::<SkaliWindow>()
            {
                window.open_person_in(client.clone(), project_id, person.clone());
            }
        });
        widget.add_controller(click);
    }

    /// One line, as a bubble: yours on the right, everyone else's on the
    /// left. In a run of lines by the same person within minutes, only the
    /// first carries the avatar, name and time.
    fn line_row(self: &Rc<Self>, line: &ChatLine, same_run: bool) -> gtk::Widget {
        let me = self.me.borrow();
        let mine = me
            .as_ref()
            .is_some_and(|me| me.id == line.recording.creator.id);
        // Basecamp's highlight for a line that mentions you.
        let mentioned = !mine
            && line.is_rich_text()
            && me
                .as_ref()
                .is_some_and(|me| richtext::mentions(&line.content, &me.attachable_sgid));
        drop(me);
        let side = if mine {
            gtk::Align::End
        } else {
            gtk::Align::Start
        };
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .margin_top(if same_run { 2 } else { 12 })
            .css_classes(["chat-line"])
            .build();
        let face: gtk::Widget = if same_run {
            gtk::Box::builder()
                .width_request(AVATAR_SIZE)
                .build()
                .upcast()
        } else {
            let avatar = avatar(&line.recording.creator, AVATAR_SIZE);
            avatar.set_valign(gtk::Align::Start);
            self.open_person_on_click(&avatar, line);
            avatar.upcast()
        };
        // The bubble and its boost button, pushed to the line's side; the
        // other side keeps a margin so the two read as a conversation.
        let lane = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .hexpand(true)
            .halign(side)
            .build();
        if mine {
            lane.set_margin_start(LANE_INDENT);
        } else {
            lane.set_margin_end(LANE_INDENT);
        }
        let bubble = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["chat-bubble", if mine { "mine" } else { "theirs" }])
            .build();
        if mentioned {
            bubble.add_css_class("mentioned");
        }
        if !same_run {
            let header = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .halign(side)
                .build();
            let name = gtk::Label::builder()
                .label(if mine {
                    gettext("Me")
                } else {
                    line.recording.creator.name.clone()
                })
                .css_classes(["chat-name"])
                .build();
            self.open_person_on_click(&name, line);
            let time = gtk::Label::builder()
                .label(time_label(line.recording.created_at))
                .css_classes(["dim-label", "caption"])
                .valign(gtk::Align::Baseline)
                .build();
            if mine {
                header.append(&time);
                header.append(&name);
            } else {
                header.append(&name);
                header.append(&time);
            }
            bubble.append(&header);
        }
        // Mentions as Basecamp shows them: a face and a name in the text.
        // A plain line's player is taken out of its words.
        let (markup, mentions, embeds) = if line.is_rich_text() {
            let (markup, mentions) = richtext::markup_with_mentions(&line.content);
            (markup, mentions, embed::embeds(&line.content))
        } else {
            let (text, embeds) = embed::split_text(&line.content);
            (richtext::escape(&text), Vec::new(), embeds)
        };
        let body = InlineLabel::with_mentions(&markup, &mentions);
        body.set_visible(!markup.is_empty());
        body.label().set_selectable(true);
        body.label().set_focusable(false);
        // A link to a Basecamp item opens on Skali's page for it.
        body.label().connect_activate_link(|label, uri| {
            match label.root().and_downcast::<SkaliWindow>() {
                Some(window) if window.open_app_url(uri) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        bubble.append(&body);
        for embed in &embeds {
            bubble.append(&Self::embed_card(embed));
        }
        for attachment in &line.attachments {
            bubble.append(&self.attachment(line, attachment));
        }
        let line_id = line.recording.id;
        let chips = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .visible(false)
            .build();
        bubble.append(&chips);
        self.boosts
            .borrow_mut()
            .insert(line_id, (chips, Vec::new()));
        if line.recording.boosts_count > 0 {
            self.fetch_boosts(line_id);
        }
        let add = self.client.writes_allowed().then(|| {
            let add = gtk::Button::builder()
                .icon_name("face-smile-big-symbolic")
                .tooltip_text(gettext("Boost"))
                .valign(gtk::Align::Center)
                .css_classes(["flat", "circular", "boost-add"])
                .build();
            add.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |add| view.ask_boost(line_id, add)
            ));
            add
        });
        // The width cap is a clamp, not the label's max-width-chars: a
        // wrapping label is measured at the width it's offered, so it must
        // be offered the width it's drawn at, or long lines overflow.
        let bubble = adw::Clamp::builder()
            .maximum_size(BUBBLE_WIDTH)
            .tightening_threshold(BUBBLE_WIDTH)
            .child(&bubble)
            .build();
        if mine {
            if let Some(add) = &add {
                lane.append(add);
            }
            lane.append(&bubble);
            row.append(&lane);
            row.append(&face);
        } else {
            lane.append(&bubble);
            if let Some(add) = &add {
                lane.append(add);
            }
            row.append(&face);
            row.append(&lane);
        }
        self.rows
            .borrow_mut()
            .insert(line.recording.id, row.clone().upcast());
        row.upcast()
    }
}

pub fn page(
    window: &SkaliWindow,
    client: Client,
    title: &str,
    chat_id: i64,
) -> adw::NavigationPage {
    let lines_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .valign(gtk::Align::End)
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    let scroller = super::scrolled_clamp(&lines_box, 900);
    if let Some(clamp) = scroller.child() {
        clamp.set_valign(gtk::Align::End);
    }
    stack.add_named(&scroller, Some("lines"));
    stack.add_named(
        &super::empty(
            &gettext("Nothing said here yet."),
            "user-available-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(title, &stack);
    page.set_tag(Some(&format!("chat-{chat_id}")));

    let writes_allowed = client.writes_allowed();
    let view = Rc::new(ChatView {
        window: window.downgrade(),
        client,
        title: title.to_string(),
        lines_box,
        scroller: scroller.clone(),
        stack,
        next: RefCell::new(Some(format!("/chats/{chat_id}/lines.json"))),
        loading: Cell::new(false),
        shown: Cell::new(0),
        chat_id,
        newest: RefCell::new(None),
        line_ids: RefCell::new(HashSet::new()),
        boosts: RefCell::new(HashMap::new()),
        me: RefCell::new(None),
        pinned: Cell::new(true),
        rows: RefCell::new(HashMap::new()),
        focus: Cell::new(None),
        focus_pages: Cell::new(0),
    });
    window.watch_chat(&view);
    view.load_first();

    // The composer, along the bottom.
    let entry = gtk::Entry::builder()
        .placeholder_text(gettext("Say something…"))
        .hexpand(true)
        .sensitive(writes_allowed)
        .tooltip_text(if writes_allowed {
            String::new()
        } else {
            gettext("Writes to this account are turned off.")
        })
        .build();
    let send = gtk::Button::builder()
        .icon_name("document-send-symbolic")
        .tooltip_text(gettext("Send"))
        .sensitive(writes_allowed)
        .css_classes(["suggested-action", "circular"])
        .build();
    entry.connect_activate(glib::clone!(
        #[strong]
        view,
        move |entry| view.send(entry)
    ));
    send.connect_clicked(glib::clone!(
        #[strong]
        view,
        #[weak]
        entry,
        move |_| view.send(&entry)
    ));
    // GTK's emoji chooser, typing the one picked where the cursor is.
    let emoji_chooser = gtk::EmojiChooser::new();
    emoji_chooser.connect_emoji_picked(glib::clone!(
        #[weak]
        entry,
        move |_, emoji| {
            entry.delete_selection();
            let mut position = entry.position();
            entry.insert_text(emoji, &mut position);
            entry.set_position(position);
        }
    ));
    emoji_chooser.connect_closed(glib::clone!(
        #[weak]
        entry,
        move |_| {
            entry.grab_focus_without_selecting();
        }
    ));
    let emoji = gtk::MenuButton::builder()
        .icon_name("face-smile-symbolic")
        .tooltip_text(gettext("Insert an emoji"))
        .popover(&emoji_chooser)
        .sensitive(writes_allowed)
        .css_classes(["circular"])
        .build();
    let gif_button = crate::widgets::gifs::picker_button(window, {
        let view = Rc::downgrade(&view);
        move |button, gif| {
            if let Some(view) = view.upgrade() {
                view.send_gif(gif, button);
            }
        }
    });
    gif_button.set_sensitive(writes_allowed);
    let composer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    composer.append(&gif_button);
    composer.append(&emoji);
    composer.append(&entry);
    composer.append(&send);
    // The whole width of the pane, as in Bubo: the entry takes what the
    // buttons leave.
    if let Some(toolbar) = page.child().and_downcast::<adw::ToolbarView>() {
        toolbar.add_bottom_bar(&composer);
    }
    // Focus once the page is on screen; before that the sidebar keeps it.
    if writes_allowed {
        page.connect_shown(move |_| {
            entry.grab_focus();
        });
    }

    // Near the top: the older page goes in above.
    scroller.vadjustment().connect_value_changed(glib::clone!(
        #[strong]
        view,
        move |adjustment| {
            view.pinned
                .set(adjustment.value() + adjustment.page_size() >= adjustment.upper() - 40.0);
            if adjustment.value() <= SCROLLBACK_THRESHOLD {
                view.load_older();
            }
        }
    ));
    // Grown (or the window resized) while at the bottom: stay there.
    scroller.vadjustment().connect_changed(glib::clone!(
        #[strong]
        view,
        move |adjustment| {
            // Not from inside the signal: it comes mid-allocation, and the
            // viewport puts its own value back.
            if view.pinned.get() {
                let adjustment = adjustment.clone();
                glib::idle_add_local_once(move || {
                    adjustment.set_value(adjustment.upper() - adjustment.page_size());
                });
            }
        }
    ));
    page
}

/// A link's host, for a preview with no title of its own to show.
fn host_of(href: &str) -> String {
    href.split_once("://")
        .map_or(href, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string()
}

/// Fetch a picture from the open web onto `picture`, and show it once it's
/// there; a picture that doesn't come stays hidden or blank.
fn fetch_picture(picture: &gtk::Picture, url: String) {
    rt::run(
        {
            let url = url.clone();
            async move { embed::fetch(&url).await }
        },
        glib::clone!(
            #[weak]
            picture,
            move |result| match result {
                Ok(bytes) => {
                    if crate::widgets::gifs::show_image(&picture, &bytes).is_some() {
                        picture.set_visible(true);
                    } else {
                        tracing::debug!("the picture at {url} didn't decode");
                    }
                }
                Err(error) => tracing::debug!("could not fetch the picture at {url}: {error}"),
            }
        ),
    );
}

/// Whether `line` continues a run by the same person within minutes.
fn joins(previous: Option<(i64, chrono::DateTime<chrono::Utc>)>, line: &ChatLine) -> bool {
    previous.is_some_and(|(creator, at)| {
        creator == line.recording.creator.id
            && line
                .recording
                .created_at
                .is_some_and(|t| (t - at).num_minutes() < 5)
    })
}
