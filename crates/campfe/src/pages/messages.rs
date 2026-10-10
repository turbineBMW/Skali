//! A message board: its messages, newest first, each opening to the
//! message and its comments.

use super::recording::RecordingDoc;
use super::{avatar, count_label, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{DockTool, Message, Project};
use campfe_core::richtext;
use gtk::glib;

pub fn board_page(
    window: &CampfeWindow,
    client: Client,
    project: &Project,
    tool: &DockTool,
) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&list, 800), Some("list"));
    stack.add_named(
        &super::empty(&gettext("No messages yet."), "mail-unread-symbolic"),
        Some("empty"),
    );
    let page = shell(&tool.title, &stack);
    page.set_tag(Some(&format!("tool-{}", tool.id)));

    let path = format!("/message_boards/{}/messages.json", tool.id);
    let project_name = project.name.clone();
    let client_for_rows = client.clone();
    rt::run(
        async move { client.get_page::<Message>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            list,
            move |result| match result {
                Ok(page) => {
                    if page.items.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    for message in page.items {
                        let row = adw::ActionRow::builder()
                            .title(glib::markup_escape_text(&message.subject))
                            .subtitle(glib::markup_escape_text(&format!(
                                "{} · {} · {}",
                                message.recording.creator.name,
                                time_label(message.recording.created_at),
                                richtext::excerpt(&message.content, 90)
                            )))
                            .activatable(true)
                            .build();
                        row.add_prefix(&avatar(&message.recording.creator, 32));
                        if message.recording.comments_count > 0 {
                            let comments = gtk::Label::builder()
                                .label(count_label(
                                    message.recording.comments_count,
                                    &gettext("comment"),
                                    &gettext("comments"),
                                ))
                                .css_classes(["dim-label", "caption"])
                                .build();
                            row.add_suffix(&comments);
                        }
                        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                        let client = client_for_rows.clone();
                        row.connect_activated(glib::clone!(
                            #[weak]
                            window,
                            move |_| window.open_message(client.clone(), message.clone())
                        ));
                        list.append(&row);
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report(
                        &format!("could not load the message board of {project_name}"),
                        &error,
                    );
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}

/// A message and its comments.
pub fn message_page(
    window: &CampfeWindow,
    client: Client,
    message: Message,
) -> adw::NavigationPage {
    super::recording::recording_page(
        window,
        client,
        RecordingDoc {
            id: message.recording.id,
            title: message.subject.clone(),
            creator: message.recording.creator.clone(),
            created_at: message.recording.created_at,
            content: message.content.clone(),
            attachments: message.content_attachments.clone(),
            comments_count: message.recording.comments_count,
            extra_html: String::new(),
        },
    )
}
