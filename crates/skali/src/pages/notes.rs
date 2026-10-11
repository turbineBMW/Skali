//! My Notes: your one notebook note in the account, shown as rich text.
//! Edit turns it into plain text in a box; Save writes it back as rich text
//! (`PUT /my/notes.json`). A note with formatting says so first, since the
//! plain-text round trip keeps the words and loses the rest.

use super::shell_with_header;
use crate::i18n::gettext;
use crate::rt;
use crate::widgets::richtext_view::RichTextView;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::MyNote;
use skali_core::richtext;
use std::cell::RefCell;
use std::rc::Rc;

const PATH: &str = "/my/notes.json";

struct NotesPage {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    note: RefCell<MyNote>,
    stack: gtk::Stack,
    view: RichTextView,
    text: gtk::TextView,
    banner: adw::Banner,
    edit: gtk::Button,
    save: gtk::Button,
    cancel: gtk::Button,
}

pub fn page(window: &SkaliWindow, client: Client) -> adw::NavigationPage {
    let view = RichTextView::new();
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .vexpand(true)
        .build();
    let banner = adw::Banner::new(&gettext(
        "This note has formatting. Editing it here keeps the words; bold, links, lists and attachments are lost when you save.",
    ));
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 0);
    editor.append(&banner);
    editor.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&text)
            .vexpand(true)
            .build(),
    );
    let reading = adw::Clamp::builder()
        .maximum_size(800)
        .tightening_threshold(600)
        .margin_start(12)
        .margin_end(12)
        .child(view.widget())
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&reading, Some("note"));
    stack.add_named(
        &super::empty(&gettext("No notes yet"), "document-edit-symbolic"),
        Some("empty"),
    );
    stack.add_named(&editor, Some("edit"));
    let (nav_page, header) = shell_with_header(&gettext("My Notes"), &stack);

    let writes_allowed = client.writes_allowed();
    let edit = gtk::Button::builder()
        .label(gettext("Edit"))
        .sensitive(false)
        .tooltip_text(if writes_allowed {
            String::new()
        } else {
            gettext("Writes to this account are turned off.")
        })
        .build();
    let save = gtk::Button::builder()
        .label(gettext("Save"))
        .css_classes(["suggested-action"])
        .visible(false)
        .build();
    let cancel = gtk::Button::builder()
        .label(gettext("Cancel"))
        .visible(false)
        .build();
    header.pack_end(&edit);
    header.pack_end(&save);
    header.pack_start(&cancel);

    let notes = Rc::new(NotesPage {
        window: window.downgrade(),
        client,
        note: RefCell::new(MyNote::default()),
        stack,
        view,
        text,
        banner,
        edit: edit.clone(),
        save: save.clone(),
        cancel: cancel.clone(),
    });
    edit.connect_clicked(glib::clone!(
        #[strong]
        notes,
        move |_| notes.start_editing()
    ));
    cancel.connect_clicked(glib::clone!(
        #[strong]
        notes,
        move |_| notes.show()
    ));
    save.connect_clicked(glib::clone!(
        #[strong]
        notes,
        move |_| notes.save()
    ));
    notes.load(writes_allowed);
    nav_page
}

impl NotesPage {
    fn load(self: &Rc<Self>, writes_allowed: bool) {
        let client = self.client.clone();
        rt::run(
            async move { client.get::<MyNote>(PATH).await },
            glib::clone!(
                #[strong(rename_to = notes)]
                self,
                move |result| match result {
                    Ok(note) => {
                        *notes.note.borrow_mut() = note;
                        notes.edit.set_sensitive(writes_allowed);
                        notes.show();
                    }
                    Err(error) => {
                        if let Some(window) = notes.window.upgrade() {
                            window.report("could not load your notes", &error);
                        }
                        notes.stack.set_visible_child_name("empty");
                    }
                }
            ),
        );
    }

    /// Reading: the note, or the empty page.
    fn show(&self) {
        self.edit.set_visible(true);
        self.save.set_visible(false);
        self.cancel.set_visible(false);
        let note = self.note.borrow();
        if richtext::plain_text(&note.content).trim().is_empty() {
            self.stack.set_visible_child_name("empty");
        } else {
            self.view.set_document(
                &format!(
                    "<article>{}</article>",
                    richtext::expand_attachments(
                        &note.content,
                        |_| None,
                        crate::widgets::richtext_view::face,
                    )
                ),
                None,
            );
            self.stack.set_visible_child_name("note");
        }
    }

    fn start_editing(&self) {
        let note = self.note.borrow();
        self.text
            .buffer()
            .set_text(richtext::plain_text(&note.content).trim());
        self.banner
            .set_revealed(richtext::has_formatting(&note.content));
        self.edit.set_visible(false);
        self.save.set_visible(true);
        self.cancel.set_visible(true);
        self.stack.set_visible_child_name("edit");
        self.text.grab_focus();
    }

    fn save(self: &Rc<Self>) {
        let buffer = self.text.buffer();
        let typed = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let content = richtext::text_to_html(&typed);
        self.save.set_sensitive(false);
        let client = self.client.clone();
        rt::run(
            async move {
                client
                    .put::<MyNote, _>(PATH, &serde_json::json!({ "note": { "content": content } }))
                    .await
            },
            glib::clone!(
                #[strong(rename_to = notes)]
                self,
                move |result| {
                    notes.save.set_sensitive(true);
                    match result {
                        Ok(note) => {
                            *notes.note.borrow_mut() = note;
                            notes.show();
                        }
                        Err(error) => {
                            if let Some(window) = notes.window.upgrade() {
                                window.report("could not save your notes", &error);
                            }
                        }
                    }
                }
            ),
        );
    }
}
