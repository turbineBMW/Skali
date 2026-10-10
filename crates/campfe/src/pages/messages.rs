//! A message board: its messages, newest first, each opening to the
//! message and its comments.

use super::recording::RecordingDoc;
use super::{avatar, count_label, scrolled_clamp, shell_with_header, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::widgets::attachments::{self, AttachmentPicker};
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{DockTool, Message, Project};
use campfe_core::richtext;
use gtk::glib;
use std::rc::Rc;

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
    let (page, header) = shell_with_header(&tool.title, &stack);
    page.set_tag(Some(&format!("tool-{}", tool.id)));
    let writes_allowed = client.writes_allowed();
    let new_message = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(if writes_allowed {
            gettext("New message")
        } else {
            gettext("Writes to this account are turned off.")
        })
        .sensitive(writes_allowed)
        .build();
    header.pack_end(&new_message);
    let board_id = tool.id;
    new_message.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        stack,
        #[weak]
        list,
        #[strong]
        client,
        move |_| {
            // A posted message goes to the top of this list.
            let on_posted = glib::clone!(
                #[weak]
                window,
                #[weak]
                stack,
                #[weak]
                list,
                #[strong]
                client,
                move |message: &Message| {
                    list.prepend(&message_row(&window, &client, message.clone()));
                    stack.set_visible_child_name("list");
                }
            );
            window.open_message_composer(client.clone(), board_id, Box::new(on_posted));
        }
    ));

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
                        list.append(&message_row(&window, &client_for_rows, message));
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

fn message_row(window: &CampfeWindow, client: &Client, message: Message) -> adw::ActionRow {
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
    let client = client.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| window.open_message(client.clone(), message.clone())
    ));
    row
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

/// Writing a new message for a board: a title, the body, files. Posting
/// publishes it at once (no draft) and opens it in place of the composer.
pub fn compose_page(
    window: &CampfeWindow,
    client: Client,
    board_id: i64,
    on_posted: Box<dyn Fn(&Message)>,
) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let subject = gtk::Entry::builder()
        .placeholder_text(gettext("Title"))
        .activates_default(false)
        .build();
    let body = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(12)
        .right_margin(12)
        .build();
    let frame = gtk::ScrolledWindow::builder()
        .child(&body)
        .focusable(false)
        .min_content_height(300)
        .propagate_natural_height(true)
        .css_classes(["card"])
        .build();
    let picker = AttachmentPicker::new();
    column.append(&subject);
    column.append(&frame);
    column.append(picker.widget());

    let (page, header) = shell_with_header(&gettext("New message"), &scrolled_clamp(&column, 800));
    let post = gtk::Button::builder()
        .label(gettext("Post"))
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&post);
    subject.connect_changed(glib::clone!(
        #[weak]
        post,
        move |subject| post.set_sensitive(!subject.text().trim().is_empty())
    ));
    subject.connect_map(|subject| {
        subject.grab_focus();
    });

    let on_posted = Rc::new(on_posted);
    post.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        subject,
        #[weak]
        body,
        #[strong]
        picker,
        move |post| {
            let title = subject.text().trim().to_string();
            if title.is_empty() {
                return;
            }
            let buffer = body.buffer();
            let typed = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let files = picker.files();
            let busy = |busy: bool| {
                post.set_sensitive(!busy);
                subject.set_sensitive(!busy);
                body.set_sensitive(!busy);
                picker.set_sensitive(!busy);
            };
            busy(true);
            let job_client = client.clone();
            let path = format!("/message_boards/{board_id}/messages.json");
            rt::run(
                async move {
                    let tags = attachments::upload(job_client.clone(), files).await?;
                    let content = richtext::text_to_html(&typed) + &tags.concat();
                    let body = serde_json::json!({
                        "subject": title,
                        "content": content,
                        "status": "active",
                    });
                    job_client.post::<Message, _>(&path, &body).await
                },
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    post,
                    #[weak]
                    subject,
                    #[weak]
                    body,
                    #[strong]
                    picker,
                    #[strong]
                    client,
                    #[strong]
                    on_posted,
                    move |result| match result {
                        Ok(message) => {
                            on_posted(&message);
                            window.pop_page();
                            window.open_message(client, message);
                        }
                        Err(error) => {
                            window.report("could not post the message", &error);
                            post.set_sensitive(true);
                            subject.set_sensitive(true);
                            body.set_sensitive(true);
                            picker.set_sensitive(true);
                        }
                    }
                ),
            );
        }
    ));
    page
}
