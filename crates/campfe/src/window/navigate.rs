//! Pushing pages onto the content pane: a project, then one of its tools,
//! then an item in it. Every page is built by `crate::pages` from a
//! `Client` and what it shows, and fetches for itself.

use super::CampfeWindow;
use crate::i18n::gettext;
use crate::pages;
use adw::subclass::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{DockTool, Message, Project, Todolist};

impl CampfeWindow {
    pub fn client_for(&self, account_id: i64) -> Option<Client> {
        self.state().clients.get(&account_id).cloned()
    }

    /// Replace whatever is open with this project's page.
    pub fn open_project(&self, account_id: i64, project: Project) {
        let Some(client) = self.client_for(account_id) else {
            self.toast(&gettext("This account isn't connected yet."));
            return;
        };
        let nav = &self.imp().content_nav;
        nav.pop_to_tag("home");
        nav.push(&pages::project::page(self, client, project));
    }

    pub fn open_tool(&self, client: Client, project: &Project, tool: &DockTool) {
        let page = match tool.name.as_str() {
            "chat" => pages::chat::page(self, client, project, tool),
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
