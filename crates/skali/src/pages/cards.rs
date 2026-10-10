//! A card table: its columns side by side, each with its cards; a card
//! opens to its body, steps and comments.

use super::recording::RecordingDoc;
use super::{avatar, shell};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{Card, CardColumn, CardTable, DockTool, Project};
use skali_core::richtext;

/// Column widths: the same for every column so the board reads as a grid.
const COLUMN_WIDTH: i32 = 280;

pub fn board_page(
    window: &SkaliWindow,
    client: Client,
    project: &Project,
    tool: &DockTool,
) -> adw::NavigationPage {
    let columns_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .valign(gtk::Align::Start)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .focusable(false)
        .vexpand(true)
        .child(&columns_box)
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&scroller, Some("board"));
    stack.add_named(
        &super::empty(
            &gettext("This card table has no columns."),
            "view-columns-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(&tool.title, &stack);

    let path = format!("/card_tables/{}.json", tool.id);
    let project_name = project.name.clone();
    let client_for_columns = client.clone();
    rt::run(
        async move { client.get::<CardTable>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            columns_box,
            move |result| match result {
                Ok(table) => {
                    if table.lists.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    for column in &table.lists {
                        columns_box.append(&column_widget(
                            &window,
                            client_for_columns.clone(),
                            column,
                        ));
                    }
                    stack.set_visible_child_name("board");
                }
                Err(error) => {
                    window.report(
                        &format!("could not load the card table of {project_name}"),
                        &error,
                    );
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}

/// One column: its title, count and cards, each fetched for itself.
fn column_widget(window: &SkaliWindow, client: Client, column: &CardColumn) -> gtk::Box {
    let widget = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(COLUMN_WIDTH)
        .css_classes(["card-column"])
        .build();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if let Some(color) = column.color.as_deref().filter(|c| !c.is_empty()) {
        let dot = gtk::Box::builder()
            .width_request(10)
            .height_request(10)
            .valign(gtk::Align::Center)
            .css_classes(["column-dot", &format!("column-{color}")])
            .build();
        header.append(&dot);
    }
    let title = gtk::Label::builder()
        .label(&column.recording.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    header.append(&title);
    let count = gtk::Label::builder()
        .label(column.cards_count.to_string())
        .css_classes(["dim-label", "caption"])
        .build();
    header.append(&count);
    widget.append(&header);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    widget.append(&list);
    if column.cards_count == 0 {
        list.append(
            &adw::ActionRow::builder()
                .title(gettext("No cards"))
                .css_classes(["dim-label"])
                .build(),
        );
        return widget;
    }
    let path = format!("/card_tables/lists/{}/cards.json", column.recording.id);
    let client_for_rows = client.clone();
    rt::run(
        async move { client.get_all::<Card>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            list,
            move |result| match result {
                Ok(cards) => {
                    for card in cards {
                        list.append(&card_row(&window, client_for_rows.clone(), card));
                    }
                }
                Err(error) => window.report("could not load the cards", &error),
            }
        ),
    );
    widget
}

fn card_row(window: &SkaliWindow, client: Client, card: Card) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&card.recording.title))
        .activatable(true)
        .build();
    let mut details = Vec::new();
    if let Some(due) = card.due_on {
        details.push(crate::i18n::format(
            &gettext("Due {date}"),
            &[("date", &due.format("%b %-d").to_string())],
        ));
    }
    if !card.steps.is_empty() {
        let done = card.steps.iter().filter(|s| s.completed).count();
        details.push(crate::i18n::format(
            &gettext("{done}/{total} steps"),
            &[
                ("done", &done.to_string()),
                ("total", &card.steps.len().to_string()),
            ],
        ));
    }
    if card.recording.comments_count > 0 {
        details.push(super::count_label(
            card.recording.comments_count,
            &gettext("comment"),
            &gettext("comments"),
        ));
    }
    if !details.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(&details.join(" · ")));
    }
    for assignee in &card.assignees {
        let avatar = avatar(assignee, 24);
        avatar.set_tooltip_text(Some(&assignee.name));
        row.add_suffix(&avatar);
    }
    if card.completed {
        row.add_css_class("dim-label");
    }
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| window.open_card(client.clone(), card.clone())
    ));
    row
}

/// A card: its body, then its steps as a checklist, then comments.
pub fn card_page(window: &SkaliWindow, client: Client, card: Card) -> adw::NavigationPage {
    let mut steps = String::new();
    if !card.steps.is_empty() {
        steps.push_str(&format!(
            "<section class=\"steps\"><h2>{}</h2><ul class=\"steps\">",
            richtext::escape(&gettext("Steps"))
        ));
        for step in &card.steps {
            steps.push_str(&format!(
                "<li class=\"{}\">{} {}</li>",
                if step.completed { "done" } else { "open" },
                if step.completed { "☑" } else { "☐" },
                richtext::escape(&step.recording.title)
            ));
        }
        steps.push_str("</ul></section>");
    }
    let mut content = card.content.clone();
    if let Some(due) = card.due_on {
        content = format!(
            "<p class=\"due\">{}</p>{content}",
            richtext::escape(&crate::i18n::format(
                &gettext("Due {date}"),
                &[("date", &due.format("%B %-d, %Y").to_string())],
            ))
        );
    }
    super::recording::recording_page(
        window,
        client,
        RecordingDoc {
            id: card.recording.id,
            title: card.recording.title.clone(),
            creator: card.recording.creator.clone(),
            created_at: card.recording.created_at,
            content,
            attachments: card.content_attachments.clone(),
            comments_count: card.recording.comments_count,
            extra_html: steps,
        },
    )
}
