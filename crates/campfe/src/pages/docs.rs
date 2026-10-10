//! Docs & Files: a vault's folders, documents and files; a document opens
//! to its text, a file to a preview or a download.

use super::recording::RecordingDoc;
use super::{avatar, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{Attachment, Document, Upload, Vault};
use campfe_core::richtext;
use gtk::glib;

/// Everything in one vault, fetched together.
struct VaultContents {
    folders: Vec<Vault>,
    documents: Vec<Document>,
    uploads: Vec<Upload>,
}

pub fn vault_page(
    window: &CampfeWindow,
    client: Client,
    title: &str,
    vault_id: i64,
) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    stack.add_named(
        &super::empty(&gettext("Nothing here yet."), "folder-documents-symbolic"),
        Some("empty"),
    );
    let page = shell(title, &stack);

    let client_for_rows = client.clone();
    rt::run(
        async move {
            let folders = client
                .get_all::<Vault>(&format!("/vaults/{vault_id}/vaults.json"))
                .await?;
            let documents = client
                .get_all::<Document>(&format!("/vaults/{vault_id}/documents.json"))
                .await?;
            let uploads = client
                .get_all::<Upload>(&format!("/vaults/{vault_id}/uploads.json"))
                .await?;
            Ok::<_, campfe_core::error::ApiError>(VaultContents {
                folders,
                documents,
                uploads,
            })
        },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            column,
            move |result| match result {
                Ok(contents) => {
                    if contents.folders.is_empty()
                        && contents.documents.is_empty()
                        && contents.uploads.is_empty()
                    {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    if !contents.folders.is_empty() {
                        let list = section(&column, &gettext("Folders"));
                        for folder in contents.folders {
                            let row = adw::ActionRow::builder()
                                .title(glib::markup_escape_text(&folder.recording.title))
                                .subtitle(glib::markup_escape_text(&folder_counts(&folder)))
                                .activatable(true)
                                .build();
                            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
                            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                            let client = client_for_rows.clone();
                            row.connect_activated(glib::clone!(
                                #[weak]
                                window,
                                move |_| window.open_vault(
                                    client.clone(),
                                    &folder.recording.title,
                                    folder.recording.id
                                )
                            ));
                            list.append(&row);
                        }
                    }
                    if !contents.documents.is_empty() {
                        let list = section(&column, &gettext("Documents"));
                        for document in contents.documents {
                            let row = adw::ActionRow::builder()
                                .title(glib::markup_escape_text(&document.recording.title))
                                .subtitle(glib::markup_escape_text(&format!(
                                    "{} · {} · {}",
                                    document.recording.creator.name,
                                    time_label(document.recording.updated_at),
                                    richtext::excerpt(&document.content, 80)
                                )))
                                .activatable(true)
                                .build();
                            row.add_prefix(&avatar(&document.recording.creator, 32));
                            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                            let client = client_for_rows.clone();
                            row.connect_activated(glib::clone!(
                                #[weak]
                                window,
                                move |_| window.open_document(client.clone(), document.clone())
                            ));
                            list.append(&row);
                        }
                    }
                    if !contents.uploads.is_empty() {
                        let list = section(&column, &gettext("Files"));
                        for upload in contents.uploads {
                            let row = adw::ActionRow::builder()
                                .title(glib::markup_escape_text(&upload.recording.title))
                                .subtitle(glib::markup_escape_text(&format!(
                                    "{} · {} · {}",
                                    upload.size_label(),
                                    upload.recording.creator.name,
                                    time_label(upload.recording.created_at)
                                )))
                                .activatable(true)
                                .build();
                            row.add_prefix(&gtk::Image::from_icon_name(if upload.is_image() {
                                "image-x-generic-symbolic"
                            } else {
                                "text-x-generic-symbolic"
                            }));
                            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                            let client = client_for_rows.clone();
                            row.connect_activated(glib::clone!(
                                #[weak]
                                window,
                                move |_| window.open_upload(client.clone(), upload.clone())
                            ));
                            list.append(&row);
                        }
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report("could not load Docs & Files", &error);
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}

/// A heading and a boxed list under it, appended to the column.
fn section(column: &gtk::Box, title: &str) -> gtk::ListBox {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let heading = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    group.append(&heading);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    group.append(&list);
    column.append(&group);
    list
}

fn folder_counts(folder: &Vault) -> String {
    let mut parts = Vec::new();
    if folder.vaults_count > 0 {
        parts.push(super::count_label(
            folder.vaults_count,
            &gettext("folder"),
            &gettext("folders"),
        ));
    }
    if folder.documents_count > 0 {
        parts.push(super::count_label(
            folder.documents_count,
            &gettext("document"),
            &gettext("documents"),
        ));
    }
    if folder.uploads_count > 0 {
        parts.push(super::count_label(
            folder.uploads_count,
            &gettext("file"),
            &gettext("files"),
        ));
    }
    if parts.is_empty() {
        gettext("Empty")
    } else {
        parts.join(" · ")
    }
}

pub fn document_page(
    window: &CampfeWindow,
    client: Client,
    document: Document,
) -> adw::NavigationPage {
    super::recording::recording_page(
        window,
        client,
        RecordingDoc {
            id: document.recording.id,
            title: document.recording.title.clone(),
            creator: document.recording.creator.clone(),
            created_at: document.recording.created_at,
            content: document.content.clone(),
            attachments: document.content_attachments.clone(),
            comments_count: document.recording.comments_count,
            extra_html: String::new(),
        },
    )
}

/// A file: its description, a preview for an image, the download for
/// anything else, then comments. The file is passed to the document as a
/// made-up attachment so the page fetches and inlines it like any other.
pub fn upload_page(window: &CampfeWindow, client: Client, upload: Upload) -> adw::NavigationPage {
    let sgid = format!("upload-{}", upload.recording.id);
    let attachment = Attachment {
        id: upload.recording.id,
        sgid: sgid.clone(),
        filename: upload.filename.clone(),
        content_type: upload.content_type.clone(),
        byte_size: upload.byte_size,
        width: upload.width,
        height: upload.height,
        download_url: upload.download_url.clone(),
        ..Attachment::default()
    };
    let content = format!(
        "{}<bc-attachment sgid=\"{}\" content-type=\"{}\" filename=\"{}\" href=\"{}\"></bc-attachment><p class=\"dim\">{} · {}</p>",
        upload.description,
        richtext::escape(&sgid),
        richtext::escape(&upload.content_type),
        richtext::escape(&upload.filename),
        richtext::escape(&upload.download_url),
        richtext::escape(&upload.filename),
        richtext::escape(&upload.size_label())
    );
    super::recording::recording_page(
        window,
        client,
        RecordingDoc {
            id: upload.recording.id,
            title: upload.recording.title.clone(),
            creator: upload.recording.creator.clone(),
            created_at: upload.recording.created_at,
            content,
            attachments: vec![attachment],
            comments_count: upload.recording.comments_count,
            extra_html: String::new(),
        },
    )
}
