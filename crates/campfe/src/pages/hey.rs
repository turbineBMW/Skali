//! Hey!: what's new for the signed-in person, unread first, then what's
//! been read. Each row opens the item in Basecamp's web app until Campfe
//! can show every kind itself.

use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{Reading, Readings};
use campfe_core::richtext;
use gtk::glib;

pub fn page(window: &CampfeWindow, client: Client) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    stack.add_named(
        &super::empty(
            &gettext("Nothing new. Enjoy the quiet."),
            "mail-unread-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(&gettext("Hey!"), &stack);
    page.set_tag(Some("hey"));

    rt::run(
        async move { client.get::<Readings>("/my/readings.json").await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            column,
            move |result| match result {
                Ok(readings) => {
                    if readings.unreads.is_empty() && readings.reads.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    if !readings.unreads.is_empty() {
                        column.append(&section(
                            &gettext("New for you"),
                            &readings.unreads,
                            &window,
                        ));
                    }
                    if !readings.reads.is_empty() {
                        column.append(&section(&gettext("Previously"), &readings.reads, &window));
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report("could not load Hey!", &error);
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}

fn section(title: &str, readings: &[Reading], window: &CampfeWindow) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let heading = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    column.append(&heading);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for reading in readings {
        list.append(&row(reading, window));
    }
    column.append(&list);
    column
}

/// What a notification is about, in a word, by its section and type.
fn describe(reading: &Reading) -> String {
    match reading.section.as_str() {
        "pings" => gettext("Ping"),
        "chats" => gettext("Chat"),
        "mentions" => gettext("Mention"),
        "bubbles" => gettext("Bubbled up"),
        _ => match reading.kind.as_str() {
            "Message" => gettext("Message"),
            "Comment" => gettext("Comment"),
            "Todo" => gettext("To-do"),
            "Todolist" => gettext("To-do list"),
            "Kanban::Card" => gettext("Card"),
            "Schedule::Entry" => gettext("Event"),
            "Document" => gettext("Document"),
            "Upload" => gettext("File"),
            "Question::Answer" => gettext("Check-in answer"),
            "Onboarding" => gettext("From Basecamp"),
            other => other.to_string(),
        },
    }
}

fn row(reading: &Reading, window: &CampfeWindow) -> adw::ActionRow {
    let title = if reading.title.is_empty() {
        describe(reading)
    } else {
        reading.title.clone()
    };
    let mut parts = vec![describe(reading)];
    if !reading.bucket_name.is_empty() {
        parts.push(reading.bucket_name.clone());
    }
    if !reading.creator.name.is_empty() {
        parts.push(reading.creator.name.clone());
    }
    parts.push(time_label(reading.updated_at.or(reading.created_at)));
    let excerpt = richtext::excerpt(&reading.content_excerpt, 100);
    let subtitle = if excerpt.is_empty() {
        parts.join(" · ")
    } else {
        format!("{}\n{}", parts.join(" · "), excerpt)
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .activatable(true)
        .build();
    row.add_prefix(&avatar(&reading.creator, 32));
    if reading.unread_count > 0 {
        let badge = gtk::Label::builder()
            .label(reading.unread_count.to_string())
            .css_classes(["badge"])
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&badge);
    }
    row.add_suffix(&gtk::Image::from_icon_name("external-link-symbolic"));
    let url = reading.app_url.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| {
            if url.is_empty() {
                return;
            }
            gtk::UriLauncher::new(&url).launch(
                Some(&window),
                gtk::gio::Cancellable::NONE,
                |result| {
                    if let Err(error) = result {
                        tracing::warn!("could not open {error}");
                    }
                },
            );
        }
    ));
    row
}
