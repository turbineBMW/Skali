//! The pages of the content pane, one module per kind. Each is a function
//! that builds an `Adw.NavigationPage`, starts its fetch through `rt::run`
//! and fills itself in when the answer comes; the window only pushes it.

pub mod assignments;
pub mod bookmarks;
pub mod cards;
pub mod chat;
pub mod docs;
pub mod hey;
pub mod messages;
pub mod notes;
pub mod pings;
pub mod project;
pub mod recording;
pub mod schedule;
pub mod search;
pub mod timeline;
pub mod todos;

use crate::i18n::gettext;
use adw::prelude::*;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use skali_core::models::Person;

/// A header bar, a page title and a content area with the usual chrome.
pub fn shell(title: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    shell_with_header(title, content).0
}

/// `shell`, and its header bar for the page's own buttons.
pub fn shell_with_header(
    title: &str,
    content: &impl IsA<gtk::Widget>,
) -> (adw::NavigationPage, adw::HeaderBar) {
    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(content));
    let page = adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build();
    (page, header)
}

/// A scrolling column of content, centred and clamped to a readable width.
pub fn scrolled_clamp(child: &impl IsA<gtk::Widget>, max_width: i32) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder()
        .maximum_size(max_width)
        .tightening_threshold(max_width - 200)
        // At the top: a list shorter than the page must not stretch to fill it.
        .valign(gtk::Align::Start)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .child(child)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        // Tab goes to what's inside, not to the scroller itself.
        .focusable(false)
        .child(&clamp)
        .build()
}

/// A spinner for a page still waiting on Basecamp.
pub fn loading() -> gtk::Widget {
    adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build()
        .upcast()
}

/// What to show in place of a list that came back empty.
pub fn empty(title: &str, icon: &str) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        .vexpand(true)
        .build()
        .upcast()
}

/// A person's avatar: their initials at once, their picture when loaded.
pub fn avatar(person: &Person, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::builder()
        .size(size)
        .text(&person.name)
        .show_initials(true)
        .build();
    crate::avatars::load_into(&avatar, &person.avatar_url);
    avatar
}

/// A time as a list shows it: the clock for today, the date otherwise.
pub fn time_label(time: Option<DateTime<Utc>>) -> String {
    let Some(time) = time else {
        return String::new();
    };
    let local = time.with_timezone(&Local);
    let today = Local::now().date_naive();
    if local.date_naive() == today {
        local.format("%H:%M").to_string()
    } else if local.date_naive().year_ce() == today.year_ce() {
        local.format("%b %-d").to_string()
    } else {
        local.format("%b %-d, %Y").to_string()
    }
}

/// A day as a heading: Today, Yesterday, Tomorrow, or its date.
pub fn day_label(day: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if day == today {
        gettext("Today")
    } else if day == today - Duration::days(1) {
        gettext("Yesterday")
    } else if day == today + Duration::days(1) {
        gettext("Tomorrow")
    } else if day.year() == today.year() {
        day.format("%A, %B %-d").to_string()
    } else {
        day.format("%A, %B %-d, %Y").to_string()
    }
}

/// `n` of something, for a subtitle.
pub fn count_label(n: i64, singular: &str, plural: &str) -> String {
    let word = if n == 1 { singular } else { plural };
    crate::i18n::format(
        &gettext("{n} {what}"),
        &[("n", &n.to_string()), ("what", word)],
    )
}

/// A kind of recording (its `type`) in a word, for a subtitle.
pub fn kind_label(kind: &str) -> String {
    match kind {
        "Message" => gettext("Message"),
        "Comment" => gettext("Comment"),
        "Todo" => gettext("To-do"),
        "Todolist" => gettext("To-do list"),
        "Kanban::Card" => gettext("Card"),
        "Schedule::Entry" => gettext("Event"),
        "Document" => gettext("Document"),
        "Upload" | "Attachment" => gettext("File"),
        "Vault" => gettext("Folder"),
        "Question" => gettext("Check-in"),
        "Question::Answer" => gettext("Check-in answer"),
        "Chat::Transcript" | "Chat" => gettext("Chat"),
        "Circle" => gettext("Ping"),
        "Mention" => gettext("Mention"),
        "Inbox::Forward" => gettext("Forwarded email"),
        "Client::Correspondence" => gettext("Client email"),
        "Onboarding" => gettext("From Basecamp"),
        other if other.starts_with("Chat::Lines") => gettext("Chat"),
        other => other.to_string(),
    }
}
