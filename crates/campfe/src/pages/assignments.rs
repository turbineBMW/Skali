//! My Assignments: the to-dos and cards assigned to the signed-in person,
//! Up Next first, then the rest grouped by project. A row opens the card,
//! or the to-do's list.

use super::shell;
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{Assignment, Assignments};
use gtk::glib;
use std::collections::BTreeMap;

pub fn page(window: &CampfeWindow, client: Client) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    stack.add_named(
        &super::empty(
            &gettext("Nothing assigned to you."),
            "object-select-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(&gettext("My Assignments"), &stack);

    let client_for_rows = client.clone();
    rt::run(
        async move { client.get::<Assignments>("/my/assignments.json").await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            column,
            #[strong(rename_to = client)]
            client_for_rows,
            move |result| match result {
                Ok(assignments) => {
                    if assignments.priorities.is_empty() && assignments.non_priorities.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    if !assignments.priorities.is_empty() {
                        column.append(&section(
                            &window,
                            &client,
                            &gettext("Up Next"),
                            &assignments.priorities,
                        ));
                    }
                    let mut by_project: BTreeMap<String, Vec<&Assignment>> = BTreeMap::new();
                    for assignment in &assignments.non_priorities {
                        by_project
                            .entry(assignment.bucket.name.clone())
                            .or_default()
                            .push(assignment);
                    }
                    for (project, items) in by_project {
                        let items: Vec<Assignment> = items.into_iter().cloned().collect();
                        column.append(&section(&window, &client, &project, &items));
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report("could not load the assignments", &error);
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}

fn section(
    window: &CampfeWindow,
    client: &Client,
    title: &str,
    assignments: &[Assignment],
) -> gtk::Box {
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
    for assignment in assignments {
        list.append(&row(window, client, assignment));
    }
    column.append(&list);
    column
}

fn row(window: &CampfeWindow, client: &Client, assignment: &Assignment) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&assignment.content))
        .activatable(!assignment.app_url.is_empty())
        .build();
    let url = assignment.app_url.clone();
    let client = client.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| {
            if !window.open_app_url(client.clone(), &url) {
                window.open_in_browser(&url);
            }
        }
    ));
    let check = gtk::CheckButton::builder()
        .active(assignment.completed)
        .sensitive(false)
        .valign(gtk::Align::Center)
        .build();
    row.add_prefix(&check);
    let mut details = Vec::new();
    if let Some(parent) = assignment.parent.as_ref().filter(|p| !p.title.is_empty()) {
        details.push(parent.title.clone());
    }
    if let Some(due) = assignment.due_on {
        details.push(crate::i18n::format(
            &gettext("Due {date}"),
            &[("date", &due.format("%b %-d").to_string())],
        ));
    }
    if assignment.comments_count > 0 {
        details.push(super::count_label(
            assignment.comments_count,
            &gettext("comment"),
            &gettext("comments"),
        ));
    }
    if !assignment.children.is_empty() {
        let done = assignment.children.iter().filter(|c| c.completed).count();
        details.push(crate::i18n::format(
            &gettext("{done}/{total} steps"),
            &[
                ("done", &done.to_string()),
                ("total", &assignment.children.len().to_string()),
            ],
        ));
    }
    if !details.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(&details.join(" · ")));
    }
    if !assignment.app_url.is_empty() {
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    }
    row
}
