//! Files picked to go with something being written: the button that picks
//! them, a chip for each, and the upload that turns them into
//! `<bc-attachment>`s for the rich text.

use crate::i18n::gettext;
use adw::prelude::*;
use gtk::{gio, glib};
use skali_core::api::Client;
use skali_core::error::Result;
use skali_core::models::UploadedAttachment;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

/// A file waiting to be uploaded. Everything the upload needs, owned, so
/// it can go to the runtime.
#[derive(Clone, Debug)]
pub struct PendingFile {
    pub path: PathBuf,
    pub name: String,
    pub content_type: String,
}

impl PendingFile {
    fn from_file(file: &gio::File) -> Option<Self> {
        let path = file.path()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let (guess, _) = gio::content_type_guess(Some(&path), None::<&[u8]>);
        let content_type = gio::content_type_get_mime_type(&guess)
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".into());
        Some(Self {
            path,
            name,
            content_type,
        })
    }
}

/// Upload `files` in order and return their `<bc-attachment>` tags.
pub async fn upload(client: Client, files: Vec<PendingFile>) -> Result<Vec<String>> {
    let mut tags = Vec::with_capacity(files.len());
    for file in files {
        let bytes = tokio::fs::read(&file.path).await?;
        let reply = client
            .upload_attachment(&file.name, &file.content_type, bytes)
            .await?;
        let uploaded: UploadedAttachment = serde_json::from_value(reply)?;
        tags.push(skali_core::richtext::attachment_tag(
            &uploaded.attachable_sgid,
        ));
    }
    Ok(tags)
}

pub struct AttachmentPicker {
    widget: gtk::Box,
    button: gtk::Button,
    chips: gtk::FlowBox,
    /// The files with the id of their chip, in the order picked.
    files: RefCell<Vec<(u32, PendingFile)>>,
    next_id: Cell<u32>,
}

impl AttachmentPicker {
    pub fn new() -> Rc<Self> {
        let button = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("mail-attachment-symbolic")
                    .label(gettext("Attach files…"))
                    .build(),
            )
            .halign(gtk::Align::Start)
            .css_classes(["flat"])
            .build();
        let chips = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(6)
            .row_spacing(6)
            .max_children_per_line(4)
            .visible(false)
            .build();
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        widget.append(&chips);
        widget.append(&button);
        let picker = Rc::new(Self {
            widget,
            button: button.clone(),
            chips,
            files: RefCell::new(Vec::new()),
            next_id: Cell::new(0),
        });
        // Weak: the button belongs to the picker's widget.
        let weak = Rc::downgrade(&picker);
        button.connect_clicked(move |button| {
            if let Some(picker) = weak.upgrade() {
                picker.choose(button);
            }
        });
        // A developer hook: start with this file picked, for testing the
        // upload without driving the file chooser.
        if let Some(path) = std::env::var_os("SKALI_DEBUG_ATTACH")
            && let Some(file) = PendingFile::from_file(&gio::File::for_path(path))
        {
            picker.add(file);
        }
        picker
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    pub fn files(&self) -> Vec<PendingFile> {
        self.files.borrow().iter().map(|(_, f)| f.clone()).collect()
    }

    pub fn set_sensitive(&self, sensitive: bool) {
        self.widget.set_sensitive(sensitive);
    }

    pub fn clear(&self) {
        self.files.borrow_mut().clear();
        self.chips.remove_all();
        self.chips.set_visible(false);
    }

    fn choose(self: &Rc<Self>, button: &gtk::Button) {
        let window = button.root().and_downcast::<gtk::Window>();
        let weak = Rc::downgrade(self);
        gtk::FileDialog::builder()
            .title(gettext("Attach files"))
            .modal(true)
            .build()
            .open_multiple(window.as_ref(), gio::Cancellable::NONE, move |result| {
                let Some(picker) = weak.upgrade() else {
                    return;
                };
                let Ok(files) = result else {
                    return; // Cancelled.
                };
                for file in files.iter::<gio::File>().flatten() {
                    match PendingFile::from_file(&file) {
                        Some(pending) => picker.add(pending),
                        None => tracing::warn!("not a local file: {}", file.uri()),
                    }
                }
            });
    }

    fn add(self: &Rc<Self>, file: PendingFile) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let chip = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::Start)
            .css_classes(["card"])
            .build();
        chip.append(
            &gtk::Image::builder()
                .gicon(&gio::content_type_get_icon(&file.content_type))
                .margin_start(8)
                .build(),
        );
        chip.append(
            &gtk::Label::builder()
                .label(&file.name)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .max_width_chars(24)
                .build(),
        );
        let remove = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(gettext("Remove"))
            .css_classes(["flat", "circular"])
            .build();
        chip.append(&remove);
        let weak = Rc::downgrade(self);
        remove.connect_clicked(glib::clone!(
            #[weak]
            chip,
            move |_| {
                let Some(picker) = weak.upgrade() else {
                    return;
                };
                picker.files.borrow_mut().retain(|(i, _)| *i != id);
                if let Some(child) = chip.parent() {
                    picker.chips.remove(&child);
                }
                picker.chips.set_visible(!picker.files.borrow().is_empty());
                picker.button.grab_focus();
            }
        ));
        self.chips.append(&chip);
        self.chips.set_visible(true);
        self.files.borrow_mut().push((id, file));
    }
}
