//! Pushing pages onto the content pane: a project, then one of its tools,
//! then an item in it. Every page is built by `crate::pages` from a
//! `Client` and what it shows, and fetches for itself.

use super::CampfeWindow;
use crate::i18n::gettext;
use crate::pages;
use adw::prelude::*;
use adw::subclass::prelude::*;
use campfe_core::api::Client;
use campfe_core::error::{ApiError, Result};
use campfe_core::links::{self, AppLink};
use campfe_core::models::{
    Campfire, Card, DockTool, Document, Message, Project, Todo, Todolist, Upload,
};
use gtk::glib;
use std::future::Future;

impl CampfeWindow {
    pub fn client_for(&self, account_id: i64) -> Option<Client> {
        self.state().clients.get(&account_id).cloned()
    }

    /// Show a top-level page (a project, Hey!...) in place of whatever is
    /// open. One already there, even mid-transition, is reused rather
    /// than pushed twice: a navigation view refuses a duplicate tag.
    fn show_root_page(&self, tag: &str, build: impl FnOnce() -> adw::NavigationPage) {
        let nav = &self.imp().content_nav;
        if let Some(existing) = nav.find_page(tag) {
            nav.pop_to_page(&existing);
            return;
        }
        nav.pop_to_tag("home");
        let page = build();
        page.set_tag(Some(tag));
        nav.push(&page);
    }

    /// Replace whatever is open with this project's page.
    pub fn open_project(&self, account_id: i64, project: Project) {
        let Some(client) = self.client_for(account_id) else {
            self.toast(&gettext("This account isn't connected yet."));
            return;
        };
        let tag = format!("project-{}", project.id);
        self.show_root_page(&tag, || pages::project::page(self, client, project));
    }

    /// The pages of the signed-in person, replacing whatever is open.
    pub fn open_hey(&self, account_id: i64) {
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        self.show_root_page(&format!("hey-{account_id}"), || {
            pages::hey::page(self, account_id, client)
        });
    }

    pub fn open_pings(&self, account_id: i64) {
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        self.show_root_page(&format!("pings-{account_id}"), || {
            pages::pings::page(self, client)
        });
    }

    pub fn open_assignments(&self, account_id: i64) {
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        self.show_root_page(&format!("assignments-{account_id}"), || {
            pages::assignments::page(self, client)
        });
    }

    /// A ping, or any Campfire reached outside its project page.
    pub fn open_campfire(&self, client: Client, campfire: &Campfire) {
        let title = if campfire.is_ping() {
            campfire.recording.title.clone()
        } else {
            format!(
                "{} · {}",
                campfire.recording.bucket.name, campfire.recording.title
            )
        };
        self.imp().content_nav.push(&pages::chat::page(
            self,
            client,
            &title,
            campfire.recording.id,
        ));
    }

    pub fn open_tool(&self, client: Client, project: &Project, tool: &DockTool) {
        let page = match tool.name.as_str() {
            "chat" => pages::chat::page(self, client, &tool.title, tool.id),
            "message_board" => pages::messages::board_page(self, client, project, tool),
            "todoset" => pages::todos::set_page(self, client, project, tool),
            "kanban_board" => pages::cards::board_page(self, client, project, tool),
            "vault" => pages::docs::vault_page(self, client, &tool.title, tool.id),
            _ => {
                self.toast(&crate::i18n::format(
                    &gettext("{tool} isn't supported yet."),
                    &[("tool", &tool.title)],
                ));
                return;
            }
        };
        self.imp().content_nav.push(&page);
    }

    pub fn open_message(&self, client: Client, message: Message) {
        self.imp()
            .content_nav
            .push(&pages::messages::message_page(self, client, message));
    }

    pub fn open_message_composer(
        &self,
        client: Client,
        board_id: i64,
        on_posted: Box<dyn Fn(&Message)>,
    ) {
        self.imp().content_nav.push(&pages::messages::compose_page(
            self, client, board_id, on_posted,
        ));
    }

    /// Close the page on top, back to the one under it.
    pub fn pop_page(&self) {
        self.imp().content_nav.pop();
    }

    pub fn open_card(&self, client: Client, card: Card) {
        self.imp()
            .content_nav
            .push(&pages::cards::card_page(self, client, card));
    }

    pub fn open_vault(&self, client: Client, title: &str, vault_id: i64) {
        self.imp()
            .content_nav
            .push(&pages::docs::vault_page(self, client, title, vault_id));
    }

    pub fn open_document(&self, client: Client, document: Document) {
        self.imp()
            .content_nav
            .push(&pages::docs::document_page(self, client, document));
    }

    pub fn open_upload(&self, client: Client, upload: Upload) {
        self.imp()
            .content_nav
            .push(&pages::docs::upload_page(self, client, upload));
    }

    pub fn open_todolist(&self, client: Client, list: Todolist) {
        self.imp()
            .content_nav
            .push(&pages::todos::list_page(self, client, list));
    }

    /// Open a Basecamp web link (an `app_url`, a link in rich text) on a
    /// page of Campfe's own when there is one: the recording is fetched
    /// with the client of the link's account, then shown, scrolled to the
    /// comment the link names. `false` for a link Campfe has no page for,
    /// or of an account it isn't signed in to, which the caller opens in
    /// the browser instead.
    pub fn open_app_url(&self, url: &str) -> bool {
        let Some(link) = AppLink::parse(url) else {
            return false;
        };
        let Some(client) = links::account_of(url).and_then(|id| self.client_for(id)) else {
            return false;
        };
        let focus = link.recording_id().zip(links::comment_of(url));
        let get = client.clone();
        let url = url.to_string();
        match link {
            AppLink::Chat(id) => self.open_fetched(
                client,
                url,
                focus,
                async move { get.get::<Campfire>(&format!("/chats/{id}.json")).await },
                |window, client, chat| window.open_campfire(client, &chat),
            ),
            AppLink::Message(id) => self.open_fetched(
                client,
                url,
                focus,
                async move { get.get::<Message>(&format!("/messages/{id}.json")).await },
                Self::open_message,
            ),
            AppLink::Card(id) => self.open_fetched(
                client,
                url,
                focus,
                async move {
                    get.get::<Card>(&format!("/card_tables/cards/{id}.json"))
                        .await
                },
                Self::open_card,
            ),
            AppLink::Document(id) => self.open_fetched(
                client,
                url,
                focus,
                async move { get.get::<Document>(&format!("/documents/{id}.json")).await },
                Self::open_document,
            ),
            AppLink::Upload(id) => self.open_fetched(
                client,
                url,
                focus,
                async move { get.get::<Upload>(&format!("/uploads/{id}.json")).await },
                Self::open_upload,
            ),
            // A to-do has no page of its own: open the list it's on.
            AppLink::Todo(id) => self.open_fetched(
                client,
                url,
                focus,
                async move {
                    let todo = get.get::<Todo>(&format!("/todos/{id}.json")).await?;
                    let list = todo.recording.parent.ok_or_else(|| {
                        ApiError::Io(std::io::Error::other("the to-do is on no list"))
                    })?;
                    get.get::<Todolist>(&format!("/todolists/{}.json", list.id))
                        .await
                },
                Self::open_todolist,
            ),
            AppLink::Todolist(id) => self.open_fetched(
                client,
                url,
                focus,
                async move { get.get::<Todolist>(&format!("/todolists/{id}.json")).await },
                Self::open_todolist,
            ),
        }
        true
    }

    fn open_fetched<T: Send + 'static>(
        &self,
        client: Client,
        url: String,
        focus: Option<(i64, i64)>,
        fetch: impl Future<Output = Result<T>> + Send + 'static,
        open: impl FnOnce(&Self, Client, T) + 'static,
    ) {
        crate::rt::run(
            fetch,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| match result {
                    Ok(item) => {
                        open(&window, client, item);
                        if let Some((recording_id, comment_id)) = focus {
                            window.focus_comment(recording_id, comment_id);
                        }
                    }
                    Err(error) => window.report(&format!("could not open {url}"), &error),
                }
            ),
        );
    }

    /// A link in the browser, for what Campfe doesn't show itself.
    pub fn open_in_browser(&self, url: &str) {
        gtk::UriLauncher::new(url).launch(Some(self), gtk::gio::Cancellable::NONE, |result| {
            if let Err(error) = result {
                tracing::warn!("could not open a link: {error}");
            }
        });
    }
}
