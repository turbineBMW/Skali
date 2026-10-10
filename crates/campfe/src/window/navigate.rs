//! Pushing pages onto the content pane: a project, then one of its tools,
//! then an item in it. Every page is built by `crate::pages` from a
//! `Client` and what it shows, and fetches for itself.

use super::CampfeWindow;
use crate::i18n::gettext;
use crate::pages;
use adw::prelude::*;
use adw::subclass::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{Campfire, DockTool, Message, Project, Todolist};

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
            pages::hey::page(self, client)
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

    pub fn open_todolist(&self, client: Client, list: Todolist) {
        self.imp()
            .content_nav
            .push(&pages::todos::list_page(self, client, list));
    }
}
