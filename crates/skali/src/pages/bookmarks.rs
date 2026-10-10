//! My Bookmarks: the recordings you starred, grouped by project. A row
//! opens in Skali or the browser; its star takes the bookmark away. The
//! list loads again whenever it comes back into view, so a star changed on
//! a recording's page shows here.

use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::links::AppLink;
use skali_core::models::Bookmark;
use std::cell::Cell;

pub fn page(window: &SkaliWindow, client: Client) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    stack.add_named(
        &super::empty(
            &gettext("No bookmarks. Star a message, card or document to keep it here."),
            "non-starred-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(&gettext("My Bookmarks"), &stack);
    load(window, &client, &stack, &column);
    let shown_before = Cell::new(false);
    page.connect_showing(glib::clone!(
        #[weak]
        window,
        #[weak]
        stack,
        #[weak]
        column,
        move |_| {
            if shown_before.replace(true) {
                load(&window, &client, &stack, &column);
            }
        }
    ));
    page
}

/// Fetch the bookmarks and lay them out, replacing what's shown.
fn load(window: &SkaliWindow, client: &Client, stack: &gtk::Stack, column: &gtk::Box) {
    let client_for_rows = client.clone();
    let client = client.clone();
    rt::run(
        async move { client.get_all::<Bookmark>("/my/bookmarks.json").await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            column,
            move |result| match result {
                Ok(bookmarks) => {
                    while let Some(child) = column.first_child() {
                        column.remove(&child);
                    }
                    if bookmarks.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    // Projects in the order their first bookmark comes.
                    let mut projects: Vec<(String, Vec<Bookmark>)> = Vec::new();
                    for bookmark in bookmarks {
                        let name = bookmark.recording.bucket.name.clone();
                        match projects.iter_mut().find(|(p, _)| *p == name) {
                            Some((_, items)) => items.push(bookmark),
                            None => projects.push((name, vec![bookmark])),
                        }
                    }
                    for (project, items) in &projects {
                        column.append(&section(&window, &client_for_rows, project, items));
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report("could not load the bookmarks", &error);
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
}

fn section(window: &SkaliWindow, client: &Client, title: &str, bookmarks: &[Bookmark]) -> gtk::Box {
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
    for bookmark in bookmarks {
        list.append(&row(window, client, bookmark));
    }
    column.append(&list);
    column
}

fn row(window: &SkaliWindow, client: &Client, bookmark: &Bookmark) -> adw::ActionRow {
    let recording = &bookmark.recording;
    let kind = super::kind_label(&recording.kind);
    let title = if recording.title.is_empty() {
        kind.clone()
    } else {
        recording.title.clone()
    };
    let mut parts = vec![kind];
    if !recording.creator.name.is_empty() {
        parts.push(recording.creator.name.clone());
    }
    parts.push(time_label(recording.updated_at.or(recording.created_at)));
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&parts.join(" · ")))
        .activatable(!recording.app_url.is_empty())
        .build();
    row.add_prefix(&avatar(&recording.creator, 32));

    let writes_allowed = client.writes_allowed();
    let unstar = gtk::Button::builder()
        .icon_name("starred-symbolic")
        .tooltip_text(if writes_allowed {
            gettext("Remove bookmark")
        } else {
            gettext("Writes to this account are turned off.")
        })
        .sensitive(writes_allowed)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let recording_id = recording.id;
    let client = client.clone();
    unstar.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        row,
        move |unstar| {
            unstar.set_sensitive(false);
            let client = client.clone();
            let path = format!("/recordings/{recording_id}/bookmark.json");
            rt::run(
                async move { client.delete(&path).await },
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    row,
                    #[weak]
                    unstar,
                    move |result| match result {
                        Ok(()) => remove_row(&row),
                        Err(error) => {
                            unstar.set_sensitive(true);
                            window.report(
                                &format!("could not remove the bookmark on {recording_id}"),
                                &error,
                            );
                        }
                    }
                ),
            );
        }
    ));
    row.add_suffix(&unstar);

    if !recording.app_url.is_empty() {
        let in_app = AppLink::parse(&recording.app_url).is_some();
        row.add_suffix(&gtk::Image::from_icon_name(if in_app {
            "go-next-symbolic"
        } else {
            "adw-external-link-symbolic"
        }));
    }
    let url = recording.app_url.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| {
            if !window.open_app_url(&url) {
                window.open_in_browser(&url);
            }
        }
    ));
    row
}

/// Take a row out; its project's section too when it was the last.
fn remove_row(row: &adw::ActionRow) {
    let Some(list) = row.parent().and_downcast::<gtk::ListBox>() else {
        return;
    };
    list.remove(row);
    if list.first_child().is_some() {
        return;
    }
    let Some(section) = list.parent().and_downcast::<gtk::Box>() else {
        return;
    };
    let Some(column) = section.parent().and_downcast::<gtk::Box>() else {
        return;
    };
    column.remove(&section);
    if column.first_child().is_none()
        && let Some(stack) = column
            .ancestor(gtk::Stack::static_type())
            .and_downcast::<gtk::Stack>()
    {
        stack.set_visible_child_name("empty");
    }
}
