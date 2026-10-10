//! A Campfire: its lines, oldest at the top, newest at the bottom where
//! the eye lands. The API pages newest-first; each page is reversed for
//! display, and scrolling to the top fetches the next (older) page and
//! prepends it, keeping what was on screen where it was. Boosts sit under
//! their line; hovering a line shows the button that adds one.

use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{Boost, ChatLine};
use skali_core::richtext;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// How close to the top (in pixels) asks for the older page.
const SCROLLBACK_THRESHOLD: f64 = 120.0;

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

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// A line that just arrived over the feed. Dropped if it's already
    /// shown (we posted it, or the feed repeated itself).
    pub fn append_live(self: &Rc<Self>, line: &ChatLine) {
        if self.line_ids.borrow().contains(&line.recording.id) {
            return;
        }
        let adjustment = self.scroller.vadjustment();
        let at_bottom = adjustment.value() + adjustment.page_size() >= adjustment.upper() - 40.0;
        self.append_line(line);
        if at_bottom {
            glib::idle_add_local_once(move || {
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
            });
        }
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
                                    if adjustment.upper() <= adjustment.page_size() {
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

    /// Your person id in this account, once the window knows it.
    fn me(&self) -> Option<i64> {
        self.window
            .upgrade()?
            .my_person_id(self.client.account_id())
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
        );
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

    /// One line. In a run of lines by the same person within minutes, only the
    /// first carries the avatar and name.
    fn line_row(self: &Rc<Self>, line: &ChatLine, same_run: bool) -> gtk::Widget {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(if same_run { 0 } else { 10 })
            .css_classes(["chat-line"])
            .build();
        if same_run {
            let spacer = gtk::Box::builder().width_request(36).build();
            row.append(&spacer);
        } else {
            let avatar = avatar(&line.recording.creator, 36);
            avatar.set_valign(gtk::Align::Start);
            row.append(&avatar);
        }
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        column.set_hexpand(true);
        if !same_run {
            let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let name = gtk::Label::builder()
                .label(&line.recording.creator.name)
                .xalign(0.0)
                .css_classes(["heading"])
                .build();
            let time = gtk::Label::builder()
                .label(time_label(line.recording.created_at))
                .css_classes(["dim-label", "caption"])
                .valign(gtk::Align::Baseline)
                .build();
            header.append(&name);
            header.append(&time);
            column.append(&header);
        }
        let body = gtk::Label::builder()
            .use_markup(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .selectable(true)
            .focusable(false)
            .build();
        let markup = if line.is_rich_text() {
            richtext::pango_markup(&line.content)
        } else {
            richtext::escape(&line.content)
        };
        body.set_markup(&markup);
        // A link to a Basecamp item opens on Skali's page for it.
        body.connect_activate_link(
            |label, uri| match label.root().and_downcast::<SkaliWindow>() {
                Some(window) if window.open_app_url(uri) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            },
        );
        column.append(&body);
        let line_id = line.recording.id;
        let chips = gtk::Box::builder()
            .spacing(6)
            .margin_top(4)
            .visible(false)
            .build();
        column.append(&chips);
        self.boosts
            .borrow_mut()
            .insert(line_id, (chips, Vec::new()));
        if line.recording.boosts_count > 0 {
            self.fetch_boosts(line_id);
        }
        row.append(&column);
        if self.client.writes_allowed() {
            let add = gtk::Button::builder()
                .icon_name("face-smile-big-symbolic")
                .tooltip_text(gettext("Boost"))
                .valign(gtk::Align::Start)
                .css_classes(["flat", "circular", "boost-add"])
                .build();
            add.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |add| {
                    let weak = Rc::downgrade(&view);
                    crate::widgets::boosts::ask(add, move |content| {
                        if let Some(view) = weak.upgrade() {
                            view.boost(line_id, content);
                        }
                    });
                }
            ));
            row.append(&add);
        }
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
    });
    window.watch_chat(&view);
    view.load_older();

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
    let composer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    composer.append(&entry);
    composer.append(&send);
    let clamp = adw::Clamp::builder()
        .maximum_size(900)
        .tightening_threshold(700)
        .child(&composer)
        .build();
    if let Some(toolbar) = page.child().and_downcast::<adw::ToolbarView>() {
        toolbar.add_bottom_bar(&clamp);
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
            if adjustment.value() <= SCROLLBACK_THRESHOLD {
                view.load_older();
            }
        }
    ));
    page
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
