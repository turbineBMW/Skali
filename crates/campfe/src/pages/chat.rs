//! A Campfire: its lines, oldest at the top, newest at the bottom where
//! the eye lands. The API pages newest-first; the first page is reversed
//! for display. Older pages load when scrolled to the top (later).

use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{ChatLine, DockTool, Project};
use campfe_core::richtext;
use gtk::glib;

pub fn page(
    window: &CampfeWindow,
    client: Client,
    project: &Project,
    tool: &DockTool,
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
    let page = shell(&tool.title, &stack);
    page.set_tag(Some(&format!("tool-{}", tool.id)));

    let path = format!("/chats/{}/lines.json", tool.id);
    let project_name = project.name.clone();
    rt::run(
        async move { client.get_page::<ChatLine>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            lines_box,
            #[weak]
            scroller,
            move |result| match result {
                Ok(page) => {
                    if page.items.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    let mut lines = page.items;
                    lines.reverse();
                    let mut previous: Option<(i64, chrono::DateTime<chrono::Utc>)> = None;
                    for line in &lines {
                        let same_run = previous.is_some_and(|(creator, at)| {
                            creator == line.recording.creator.id
                                && line
                                    .recording
                                    .created_at
                                    .is_some_and(|t| (t - at).num_minutes() < 5)
                        });
                        lines_box.append(&line_row(line, same_run));
                        previous = line
                            .recording
                            .created_at
                            .map(|at| (line.recording.creator.id, at));
                    }
                    stack.set_visible_child_name("lines");
                    // Start at the bottom, where the newest line is.
                    glib::idle_add_local_once(move || {
                        let adjustment = scroller.vadjustment();
                        adjustment.set_value(adjustment.upper() - adjustment.page_size());
                    });
                }
                Err(error) => {
                    window.report(&format!("could not load the chat {project_name}"), &error);
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
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
