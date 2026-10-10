//! The main window: accounts and projects in the sidebar, the project's
//! tools and their content on the right. One class, one `impl` block per
//! concern across the files of this module.

mod sign_in;

use crate::application::CampfeApplication;
use crate::i18n::{failure_message, gettext};
use crate::rt;
use crate::settings as keys;
use adw::prelude::*;
use adw::subclass::prelude::*;
use campfe_core::api::{Client, SharedToken};
use campfe_core::db::Database;
use campfe_core::error::ApiError;
use campfe_core::models::{Account, Project};
use gtk::gio;
use gtk::glib;
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Gtk.Stack child names, matching the ids in main-window.blp.
const PAGE_NO_ACCOUNT: &str = "no-account";
const PAGE_SIGNING_IN: &str = "signing-in";
const PAGE_MAIN: &str = "main";

/// Everything the window mutates. Kept in one `RefCell` behind short
/// borrows: read what you need, drop the borrow, then act.
#[derive(Default)]
pub struct State {
    /// Every signed-in account, by id.
    pub accounts: HashMap<i64, Account>,
    /// One API client per account, sharing the identity's token.
    pub clients: HashMap<i64, Client>,
    /// The access token per Launchpad identity, refreshed in place.
    pub tokens: HashMap<i64, SharedToken>,
    /// The projects last listed, per account.
    pub projects: HashMap<i64, Vec<Project>>,
    pub is_signing_in: bool,
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/turbinebmw/Campfe/ui/main-window.ui")]
    pub struct CampfeWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub sign_in_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub split: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub sidebar_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub content_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub content_placeholder: TemplateChild<adw::StatusPage>,

        pub db: OnceCell<Rc<RefCell<Database>>>,
        pub settings: OnceCell<gio::Settings>,
        pub state: RefCell<State>,
        pub project_store: OnceCell<gtk::StringList>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CampfeWindow {
        const NAME: &'static str = "CampfeWindow";
        type Type = super::CampfeWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CampfeWindow {}
    impl WidgetImpl for CampfeWindow {}
    impl WindowImpl for CampfeWindow {
        fn close_request(&self) -> glib::Propagation {
            self.obj().save_geometry();
            self.parent_close_request()
        }
    }
    impl ApplicationWindowImpl for CampfeWindow {}
    impl AdwApplicationWindowImpl for CampfeWindow {}
}

glib::wrapper! {
    pub struct CampfeWindow(ObjectSubclass<imp::CampfeWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl CampfeWindow {
    pub fn new(
        app: &CampfeApplication,
        db: Rc<RefCell<Database>>,
        settings: &gio::Settings,
    ) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        let imp = window.imp();
        let _ = imp.db.set(db);
        let _ = imp.settings.set(settings.clone());

        window.set_default_size(
            settings.int(keys::WINDOW_WIDTH),
            settings.int(keys::WINDOW_HEIGHT),
        );
        if settings.boolean(keys::WINDOW_MAXIMIZED) {
            window.maximize();
        }
        imp.split.set_sidebar_width_fraction(0.22);

        window.setup_actions();
        window.setup_sidebar();
        window.load_accounts();
        window
    }

    fn db(&self) -> Rc<RefCell<Database>> {
        self.imp().db.get().expect("set in new").clone()
    }

    fn settings(&self) -> &gio::Settings {
        self.imp().settings.get().expect("set in new")
    }

    pub fn state(&self) -> std::cell::Ref<'_, State> {
        self.imp().state.borrow()
    }

    pub fn state_mut(&self) -> std::cell::RefMut<'_, State> {
        self.imp().state.borrow_mut()
    }

    fn save_geometry(&self) {
        let settings = self.settings();
        let _ = settings.set_boolean(keys::WINDOW_MAXIMIZED, self.is_maximized());
        if !self.is_maximized() {
            let (width, height) = self.default_size();
            let _ = settings.set_int(keys::WINDOW_WIDTH, width);
            let _ = settings.set_int(keys::WINDOW_HEIGHT, height);
        }
    }

    fn setup_actions(&self) {
        let sign_in = gio::ActionEntry::builder("sign-in")
            .activate(|window: &Self, _, _| window.start_sign_in())
            .build();
        let refresh = gio::ActionEntry::builder("refresh")
            .activate(|window: &Self, _, _| window.refresh_projects())
            .build();
        self.add_action_entries([sign_in, refresh]);
        self.imp().sign_in_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.start_sign_in()
        ));
    }

    /// For now the sidebar is the flat list of projects across accounts.
    fn setup_sidebar(&self) {
        let imp = self.imp();
        let store = gtk::StringList::new(&[]);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let label = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(6)
                .margin_end(6)
                .build();
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let Some(text) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            if let Some(label) = item.child().and_downcast::<gtk::Label>() {
                label.set_text(&text.string());
            }
        });
        imp.sidebar_list.set_factory(Some(&factory));
        imp.sidebar_list
            .set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        let _ = imp.project_store.set(store);
    }

    pub fn toast(&self, text: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(text));
    }

    pub fn report(&self, what: &str, error: &ApiError) {
        tracing::error!("{what}: {error}");
        self.toast(&failure_message(error));
    }

    /// Accounts from the cache, then their tokens from the keyring; with
    /// none, the welcome page.
    fn load_accounts(&self) {
        let accounts = match self.db().borrow().accounts() {
            Ok(accounts) => accounts,
            Err(error) => {
                self.report("could not read accounts", &error);
                Vec::new()
            }
        };
        if accounts.is_empty() {
            self.imp()
                .main_stack
                .set_visible_child_name(PAGE_NO_ACCOUNT);
            return;
        }
        let identity_ids: Vec<i64> = {
            let mut ids: Vec<i64> = accounts.iter().map(|a| a.identity_id).collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        {
            let mut state = self.state_mut();
            for account in accounts {
                state.accounts.insert(account.id, account);
            }
        }
        self.imp().main_stack.set_visible_child_name(PAGE_MAIN);
        for identity_id in identity_ids {
            rt::run(
                async move { campfe_core::secrets::load_tokens(identity_id).await },
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |result| match result {
                        Ok(Some(tokens)) => window.tokens_loaded(identity_id, tokens),
                        Ok(None) => {
                            tracing::warn!("no tokens in the keyring for identity {identity_id}");
                            window.toast(&gettext("Sign in again to reconnect this account."));
                        }
                        Err(error) => window.report("could not read the keyring", &error),
                    }
                ),
            );
        }
    }

    /// An identity's token is in hand: build the clients of its accounts
    /// and fetch their projects.
    fn tokens_loaded(&self, identity_id: i64, tokens: campfe_core::auth::TokenSet) {
        let token: SharedToken = Arc::new(tokio::sync::RwLock::new(tokens.access_token.clone()));
        let account_ids: Vec<i64> = {
            let mut state = self.state_mut();
            state.tokens.insert(identity_id, token.clone());
            let ids: Vec<i64> = state
                .accounts
                .values()
                .filter(|a| a.identity_id == identity_id)
                .map(|a| a.id)
                .collect();
            for id in &ids {
                let href = state.accounts[id].href.clone();
                match Client::new(href, token.clone()) {
                    Ok(client) => {
                        state.clients.insert(*id, client);
                    }
                    Err(error) => {
                        tracing::error!("could not build a client for account {id}: {error}")
                    }
                }
            }
            ids
        };
        if tokens.needs_refresh(chrono::Utc::now()) {
            self.refresh_tokens(identity_id, tokens);
        }
        for id in account_ids {
            self.fetch_projects(id);
        }
    }

    fn refresh_projects(&self) {
        let ids: Vec<i64> = self.state().clients.keys().copied().collect();
        for id in ids {
            self.fetch_projects(id);
        }
    }

    fn fetch_projects(&self, account_id: i64) {
        let Some(client) = self.state().clients.get(&account_id).cloned() else {
            return;
        };
        rt::run(
            async move { client.get_all::<Project>("/projects.json").await },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| match result {
                    Ok(projects) => window.projects_loaded(account_id, projects),
                    Err(error) => window.report(
                        &format!("could not list projects of account {account_id}"),
                        &error
                    ),
                }
            ),
        );
    }

    fn projects_loaded(&self, account_id: i64, projects: Vec<Project>) {
        tracing::info!("account {account_id}: {} projects", projects.len());
        self.state_mut().projects.insert(account_id, projects);
        self.rebuild_sidebar();
    }

    fn rebuild_sidebar(&self) {
        let imp = self.imp();
        let store = imp.project_store.get().expect("set in setup_sidebar");
        let names: Vec<String> = {
            let state = self.state();
            let mut accounts: Vec<&Account> = state.accounts.values().collect();
            accounts.sort_by(|a, b| a.name.cmp(&b.name));
            if let Some(first) = accounts.first() {
                imp.sidebar_title.set_title(&first.name);
                imp.sidebar_title.set_subtitle(&first.identity_email);
            }
            accounts
                .iter()
                .flat_map(|account| {
                    state
                        .projects
                        .get(&account.id)
                        .into_iter()
                        .flatten()
                        .map(|p| p.name.clone())
                })
                .collect()
        };
        let items: Vec<&str> = names.iter().map(String::as_str).collect();
        store.splice(0, store.n_items(), &items);
        let count = names.len();
        imp.content_placeholder
            .set_description(Some(&crate::i18n::format(
                &gettext("{count} projects loaded. Pick one to open it."),
                &[("count", &count.to_string())],
            )));
    }
}
