//! A Campfire: its lines, oldest at the top, newest at the bottom where
//! the eye lands. The API pages newest-first; each page is reversed for
//! display, and scrolling to the top fetches the next (older) page and
//! prepends it, keeping what was on screen where it was.

use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::ChatLine;
use campfe_core::richtext;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How close to the top (in pixels) asks for the older page.
const SCROLLBACK_THRESHOLD: f64 = 120.0;

/// What the page remembers between fetches, shared by the scroll handler
/// and the fetch callbacks.
struct ChatView {
    window: glib::WeakRef<CampfeWindow>,
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
}

impl ChatView {
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
                            *view.next.borrow_mut() = page.next;
                            let first_page = view.shown.get() == 0;
                            if first_page && page.items.is_empty() {
                                view.stack.set_visible_child_name("empty");
                                return;
                            }
                            view.shown.set(view.shown.get() + page.items.len());
                            prepend_lines(&view.lines_box, page.items);
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
}

pub fn page(
    window: &CampfeWindow,
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
    });
    view.load_older();

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

/// Put a newest-first page of lines above what's shown, oldest first. A
/// run by one person that straddles two pages keeps a header on each
/// side; simpler than merging, and rare.
fn prepend_lines(lines_box: &gtk::Box, mut lines: Vec<ChatLine>) {
    lines.reverse();
    let mut rows: Vec<gtk::Widget> = Vec::with_capacity(lines.len());
    let mut previous: Option<(i64, chrono::DateTime<chrono::Utc>)> = None;
    for line in &lines {
        rows.push(line_row(line, joins(previous, line)));
        previous = line
            .recording
            .created_at
            .map(|at| (line.recording.creator.id, at));
    }
    for row in rows.into_iter().rev() {
        lines_box.prepend(&row);
    }
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

/// One line. In a run of lines by the same person within minutes, only the
/// first carries the avatar and name.
fn line_row(line: &ChatLine, same_run: bool) -> gtk::Widget {
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
        .build();
    let markup = if line.is_rich_text() {
        richtext::pango_markup(&line.content)
    } else {
        richtext::escape(&line.content)
    };
    body.set_markup(&markup);
    column.append(&body);
    row.append(&column);
    row.upcast()
}
