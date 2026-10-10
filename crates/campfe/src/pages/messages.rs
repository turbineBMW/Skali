//! A message board: its messages, newest first, each opening to the
//! message and its comments.

use super::{avatar, count_label, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::widgets::richtext_view::RichTextView;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{Attachment, Comment, DockTool, Message, Project};
use campfe_core::richtext;
use gtk::glib;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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

/// A message and its comments, as one document in a rich-text view, with
/// the composer along the bottom. Images are fetched with the sign-in and
/// inlined; the document is rendered again as they arrive, and as comments
/// do.
pub struct MessagePage {
    window: glib::WeakRef<CampfeWindow>,
    client: Client,
    message: Message,
    comments: RefCell<Vec<Comment>>,
    /// `data:` URIs by attachment sgid, once fetched.
    images: RefCell<HashMap<String, String>>,
    fetching: RefCell<HashSet<String>>,
    view: RichTextView,
}

impl MessagePage {
    pub fn message_id(&self) -> i64 {
        self.message.recording.id
    }

    /// A comment that arrived (posted here, or live). Dropped if shown.
    pub fn add_comment(self: &Rc<Self>, comment: Comment) {
        if self
            .comments
            .borrow()
            .iter()
            .any(|c| c.recording.id == comment.recording.id)
        {
            return;
        }
        self.fetch_images(&comment.content_attachments);
        self.comments.borrow_mut().push(comment);
        self.render();
    }

    fn render(&self) {
        let images = self.images.borrow();
        let resolve = |sgid: &str| {
            images.get(sgid).map(|src| richtext::Expanded::Image {
                src: src.clone(),
                alt: String::new(),
            })
        };
        let mut body = format!(
            "<article>{}</article>",
            richtext::expand_attachments(&self.message.content, resolve)
        );
        let comments = self.comments.borrow();
        let count = comments
            .len()
            .max(self.message.recording.comments_count as usize);
        body.push_str(&format!(
            "<section class=\"comments\"><h2>{}</h2>",
            richtext::escape(&count_label(
                count as i64,
                &gettext("comment"),
                &gettext("comments")
            ))
        ));
        for comment in comments.iter() {
            let person = &comment.recording.creator;
            let avatar = match crate::avatars::cached_bytes(&person.avatar_url) {
                Some(bytes) => format!(
                    "<img class=\"avatar\" src=\"{}\" alt=\"\">",
                    crate::widgets::richtext_view::data_uri("image/*", &bytes)
                ),
                None => format!(
                    "<span class=\"initials\">{}</span>",
                    richtext::escape(
                        &person
                            .name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string()
                    )
                ),
            };
            body.push_str(&format!(
                "<div class=\"comment\" id=\"c{}\"><div class=\"byline\">{avatar}<b>{}</b><time>{}</time></div><div class=\"body\">{}</div></div>",
                comment.recording.id,
                richtext::escape(&person.name),
                richtext::escape(&time_label(comment.recording.created_at)),
                richtext::expand_attachments(&comment.content, resolve)
            ));
        }
        body.push_str("</section>");
        self.view.set_document(&body);
    }

    /// Fetch the images among `attachments` with the sign-in, once each,
    /// and render again as each one lands.
    fn fetch_images(self: &Rc<Self>, attachments: &[Attachment]) {
        for attachment in attachments.iter().filter(|a| a.is_image()) {
            let sgid = attachment.sgid.clone();
            if sgid.is_empty()
                || self.images.borrow().contains_key(&sgid)
                || !self.fetching.borrow_mut().insert(sgid.clone())
            {
                continue;
            }
            let client = self.client.clone();
            let url = attachment.download_url.clone();
            let content_type = attachment.content_type.clone();
            rt::run(
                async move { client.get_bytes(&url).await },
                glib::clone!(
                    #[strong(rename_to = page)]
                    self,
                    move |result| {
                        page.fetching.borrow_mut().remove(&sgid);
                        match result {
                            Ok(bytes) => {
                                let uri =
                                    crate::widgets::richtext_view::data_uri(&content_type, &bytes);
                                page.images.borrow_mut().insert(sgid, uri);
                                page.render();
                            }
                            Err(error) => tracing::warn!("could not fetch an image: {error}"),
                        }
                    }
                ),
            );
        }
    }
}

/// A message and its comments.
pub fn message_page(
    window: &CampfeWindow,
    client: Client,
    message: Message,
) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_margin_start(12);
    column.set_margin_end(12);
    column.set_margin_top(12);
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
    let view = RichTextView::new();
    column.append(view.widget());
    let clamp = adw::Clamp::builder()
        .maximum_size(800)
        .tightening_threshold(600)
        .child(&column)
        .build();

    let page = Rc::new(MessagePage {
        window: window.downgrade(),
        client: client.clone(),
        message: message.clone(),
        comments: RefCell::new(Vec::new()),
        images: RefCell::new(HashMap::new()),
        fetching: RefCell::new(HashSet::new()),
        view,
    });
    window.watch_message(&page);
    page.fetch_images(&message.content_attachments);
    page.render();

    if message.recording.comments_count > 0 {
        let path = format!("/recordings/{}/comments.json", message.recording.id);
        rt::run(
            async move { client.get_all::<Comment>(&path).await },
            glib::clone!(
                #[strong]
                page,
                move |result| match result {
                    Ok(comments) => {
                        for comment in &comments {
                            page.fetch_images(&comment.content_attachments);
                        }
                        page.comments.borrow_mut().extend(comments);
                        page.render();
                    }
                    Err(error) => {
                        if let Some(window) = page.window.upgrade() {
                            window.report("could not load the comments", &error);
                        }
                    }
                }
            ),
        );
    }

    let nav_page = shell(&message.subject, &clamp);
    if let Some(toolbar) = nav_page.child().and_downcast::<adw::ToolbarView>() {
        let composer = comment_composer(&page);
        let composer_clamp = adw::Clamp::builder()
            .maximum_size(800)
            .tightening_threshold(600)
            .child(&composer)
            .build();
        toolbar.add_bottom_bar(&composer_clamp);
    }
    nav_page
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

/// A box to type a comment in, and the button that posts it. The comment
/// joins the document once Basecamp has it.
fn comment_composer(page: &Rc<MessagePage>) -> gtk::Box {
    let writes_allowed = page.client.writes_allowed();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
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
        .min_content_height(60)
        .max_content_height(200)
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
        #[strong]
        page,
        #[weak]
        text,
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
            let client = page.client.clone();
            let path = format!("/recordings/{}/comments.json", page.message_id());
            let body = serde_json::json!({ "content": html });
            rt::run(
                async move { client.post::<Comment, _>(&path, &body).await },
                glib::clone!(
                    #[strong]
                    page,
                    #[weak]
                    text,
                    #[weak]
                    button,
                    move |result| {
                        button.set_sensitive(true);
                        text.set_sensitive(true);
                        match result {
                            Ok(comment) => {
                                text.buffer().set_text("");
                                page.add_comment(comment);
                            }
                            Err(error) => {
                                if let Some(window) = page.window.upgrade() {
                                    window.report("could not post the comment", &error);
                                }
                            }
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
