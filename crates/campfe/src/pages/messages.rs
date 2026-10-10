//! A message board: its messages, newest first, each opening to the
//! message and its comments.

use super::{avatar, count_label, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{Comment, DockTool, Message, Project};
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
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let title = gtk::Label::builder()
        .label(&message.subject)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["title-1"])
        .build();
    column.append(&title);
    column.append(&byline(
        &message.recording.creator,
        message.recording.created_at,
    ));
    column.append(&rich_body(&message.content));
    for attachment in &message.content_attachments {
        column.append(&attachment_row(
            &attachment.filename,
            &attachment.download_url,
        ));
    }

    let comments_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(24)
        .build();
    let comments_heading = gtk::Label::builder()
        .label(count_label(
            message.recording.comments_count,
            &gettext("comment"),
            &gettext("comments"),
        ))
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    comments_box.append(&comments_heading);
    column.append(&comments_box);
    window.watch_message(message.recording.id, &comments_box, &comments_heading);
    column.append(&comment_composer(
        window,
        client.clone(),
        message.recording.id,
        &comments_box,
        &comments_heading,
    ));
    if message.recording.comments_count > 0 {
        comments_box.append(&super::loading());
        let path = format!("/recordings/{}/comments.json", message.recording.id);
        rt::run(
            async move { client.get_all::<Comment>(&path).await },
            glib::clone!(
                #[weak]
                window,
                #[weak]
                comments_box,
                move |result| {
                    if let Some(spinner) = comments_box.last_child() {
                        comments_box.remove(&spinner);
                    }
                    match result {
                        Ok(comments) => {
                            for comment in comments {
                                comments_box.append(&comment_row(&comment));
                            }
                        }
                        Err(error) => window.report("could not load the comments", &error),
                    }
                }
            ),
        );
    }

    let page = shell(&message.subject, &super::scrolled_clamp(&column, 760));
    page.set_tag(Some(&format!("message-{}", message.recording.id)));
    page
}

fn byline(
    person: &campfe_core::models::Person,
    at: Option<chrono::DateTime<chrono::Utc>>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&avatar(person, 32));
    let name = gtk::Label::builder()
        .label(&person.name)
        .css_classes(["heading"])
        .build();
    let time = gtk::Label::builder()
        .label(time_label(at))
        .css_classes(["dim-label"])
        .build();
    row.append(&name);
    row.append(&time);
    row
}

/// Rich text as a label, for now; a WebKit view comes later for images.
fn rich_body(html: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .use_markup(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .focusable(false)
        .css_classes(["body"])
        .build();
    label.set_markup(&richtext::pango_markup(html));
    label
}

fn attachment_row(filename: &str, url: &str) -> gtk::LinkButton {
    gtk::LinkButton::builder()
        .label(filename)
        .uri(url)
        .halign(gtk::Align::Start)
        .build()
}

/// A comment that arrived live, under the others, unless it's there already.
pub fn append_live_comment(comments_box: &gtk::Box, heading: &gtk::Label, comment: &Comment) {
    let mut child = comments_box.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == comment.recording.id.to_string() {
            return;
        }
        child = widget.next_sibling();
    }
    comments_box.append(&comment_row(comment));
    let count = comments_box.observe_children().n_items().saturating_sub(1);
    heading.set_label(&count_label(
        i64::from(count),
        &gettext("comment"),
        &gettext("comments"),
    ));
}

fn comment_row(comment: &Comment) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.append(&byline(
        &comment.recording.creator,
        comment.recording.created_at,
    ));
    let body = rich_body(&comment.content);
    body.set_margin_start(42);
    column.append(&body);
    for attachment in &comment.content_attachments {
        let link = attachment_row(&attachment.filename, &attachment.download_url);
        link.set_margin_start(42);
        column.append(&link);
    }
    column
}

/// A box to type a comment in, and the button that posts it. The comment
/// appears under the others once Basecamp has it.
fn comment_composer(
    window: &CampfeWindow,
    client: Client,
    recording_id: i64,
    comments_box: &gtk::Box,
    heading: &gtk::Label,
) -> gtk::Box {
    let heading = heading.clone();
    let writes_allowed = client.writes_allowed();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .build();
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .sensitive(writes_allowed)
        .build();
    let frame = gtk::ScrolledWindow::builder()
        .child(&text)
        .focusable(false)
        .min_content_height(80)
        .max_content_height(240)
        .propagate_natural_height(true)
        .css_classes(["card"])
        .build();
    let post = gtk::Button::builder()
        .label(gettext("Post comment"))
        .halign(gtk::Align::End)
        .sensitive(writes_allowed)
        .css_classes(["suggested-action", "pill"])
        .tooltip_text(if writes_allowed {
            String::new()
        } else {
            gettext("Writes to this account are turned off.")
        })
        .build();
    post.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        text,
        #[weak]
        comments_box,
        move |button| {
            let buffer = text.buffer();
            let typed = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let html = richtext::text_to_html(&typed);
            if html.is_empty() {
                return;
            }
            button.set_sensitive(false);
            text.set_sensitive(false);
            let client = client.clone();
            let path = format!("/recordings/{recording_id}/comments.json");
            let body = serde_json::json!({ "content": html });
            rt::run(
                async move { client.post::<Comment, _>(&path, &body).await },
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    text,
                    #[weak]
                    button,
                    #[weak]
                    comments_box,
                    #[weak]
                    heading,
                    move |result| {
                        button.set_sensitive(true);
                        text.set_sensitive(true);
                        match result {
                            Ok(comment) => {
                                text.buffer().set_text("");
                                comments_box.append(&comment_row(&comment));
                                // The heading, the spinner's slot and the rows: all but two are comments.
                                let count =
                                    comments_box.observe_children().n_items().saturating_sub(1);
                                heading.set_label(&count_label(
                                    i64::from(count),
                                    &gettext("comment"),
                                    &gettext("comments"),
                                ));
                            }
                            Err(error) => window.report("could not post the comment", &error),
                        }
                    }
                ),
            );
        }
    ));
    // Ctrl+Return posts, as in the chat.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        post,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, modifiers| {
            let is_enter = matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            if is_enter && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                post.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    ));
    text.add_controller(keys);
    // A developer hook: focus the box as soon as the page is on screen.
    if std::env::var_os("CAMPFE_DEBUG_FOCUS").is_some() {
        text.connect_map(|text| {
            text.grab_focus();
        });
    }
    column.append(&frame);
    column.append(&post);
    column
}
