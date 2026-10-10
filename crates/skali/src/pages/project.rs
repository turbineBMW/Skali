//! A project: its tools, as the web app's dock shows them, each a row that
//! opens the tool. Several of one kind (three Chats) are simply several rows.

use super::shell;
use crate::i18n::gettext;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{DockTool, Project};

/// The icon and the kind's name for a dock tool.
fn describe(tool: &DockTool) -> (&'static str, String) {
    match tool.name.as_str() {
        "chat" => ("user-available-symbolic", gettext("Chat")),
        "message_board" => ("mail-unread-symbolic", gettext("Message Board")),
        "todoset" => ("object-select-symbolic", gettext("To-dos")),
        "schedule" => ("x-office-calendar-symbolic", gettext("Schedule")),
        "vault" => ("folder-documents-symbolic", gettext("Docs & Files")),
        "kanban_board" => ("view-dual-symbolic", gettext("Card Table")),
        "questionnaire" => ("dialog-question-symbolic", gettext("Automatic Check-ins")),
        "inbox" => ("mail-unread-symbolic", gettext("Email Forwards")),
        _ => ("application-x-addon-symbolic", tool.name.clone()),
    }
}

pub fn page(window: &SkaliWindow, client: Client, project: Project) -> adw::NavigationPage {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    if let Some(description) = project.description.as_deref().filter(|d| !d.is_empty()) {
        let label = gtk::Label::builder()
            .label(description)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .margin_bottom(12)
            .build();
        content.append(&label);
    }
    let tools = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let mut shown = 0;
    for tool in project.enabled_tools() {
        let (icon, kind) = describe(tool);
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&tool.title))
            .activatable(true)
            .build();
        if tool.title != kind {
            row.set_subtitle(&glib::markup_escape_text(&kind));
        }
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let tool = tool.clone();
        let project_for_row = project.clone();
        let client = client.clone();
        row.connect_activated(glib::clone!(
            #[weak]
            window,
            move |_| window.open_tool(client.clone(), &project_for_row, &tool)
        ));
        tools.append(&row);
        shown += 1;
    }
    if shown == 0 {
        content.append(&super::empty(
            &gettext("This project has no tools turned on."),
            "folder-symbolic",
        ));
    } else {
        content.append(&tools);
    }
    shell(&project.name, &super::scrolled_clamp(&content, 720))
}
