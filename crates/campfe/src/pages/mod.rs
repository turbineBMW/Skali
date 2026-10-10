//! The pages of the content pane, one module per kind. Each is a function
//! that builds an `Adw.NavigationPage`, starts its fetch through `rt::run`
//! and fills itself in when the answer comes; the window only pushes it.

pub mod assignments;
pub mod cards;
pub mod chat;
pub mod docs;
pub mod hey;
pub mod messages;
pub mod pings;
pub mod project;
pub mod recording;
pub mod todos;

use crate::i18n::gettext;
use adw::prelude::*;
use campfe_core::models::Person;
use chrono::{DateTime, Datelike, Local, Utc};

/// A header bar, a page title and a content area with the usual chrome.
pub fn shell(title: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
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

/// `n` of something, for a subtitle.
pub fn count_label(n: i64, singular: &str, plural: &str) -> String {
    let word = if n == 1 { singular } else { plural };
    crate::i18n::format(
        &gettext("{n} {what}"),
        &[("n", &n.to_string()), ("what", word)],
    )
}
