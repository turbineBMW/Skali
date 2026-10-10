//! The main window: accounts and projects in the sidebar, the project's
//! tools and their content on the right. One class, one `impl` block per
//! concern across the files of this module.

mod live;
mod navigate;
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

/// One sidebar row.
#[derive(Clone, Debug)]
pub enum SidebarEntry {
    /// Hey!: the account's notifications, with its unread count.
    Hey {
        account_id: i64,
        unread: usize,
    },
    Pings {
        account_id: i64,
    },
    Assignments {
        account_id: i64,
    },
    Search {
        account_id: i64,
    },
    /// A section heading, not a row to open.
    Header(String),
    Project {
        account_id: i64,
        project: Project,
    },
}

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
    /// Hey!'s unread count per account, for the sidebar badge.
    pub unread_counts: HashMap<i64, usize>,
    /// The chat pages open now, by chat id, for lines arriving live.
    pub live_chats: HashMap<i64, std::rc::Weak<crate::pages::chat::ChatView>>,
    /// The message pages open now, by message id, for comments arriving live.
    pub live_messages: HashMap<i64, live::LiveMessage>,
    /// The live feed's drain loop per account; aborting it drops the
    /// channel, which ends the stream.
    pub feeds: HashMap<i64, glib::JoinHandle<()>>,
    /// A sign-in waiting for the browser to come back.
    pub sign_in: Option<sign_in::PendingSignIn>,
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
        pub cancel_sign_in_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub split: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub content_nav: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub sidebar_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub content_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub content_placeholder: TemplateChild<adw::StatusPage>,

        pub db: OnceCell<Rc<RefCell<Database>>>,
        pub settings: OnceCell<gio::Settings>,
        pub state: RefCell<State>,
        /// `SidebarEntry`s, in sidebar order.
        pub sidebar_store: OnceCell<gio::ListStore>,
        /// Accounts with a Hey! recount waiting out its delay.
        pub recount_pending: RefCell<std::collections::HashSet<i64>>,
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
        // Search the first account, as the sidebar lists them.
        let search = gio::ActionEntry::builder("search")
            .activate(|window: &Self, _, _| {
                let first = {
                    let state = window.state();
                    state
                        .accounts
                        .values()
                        .min_by(|a, b| a.name.cmp(&b.name))
                        .map(|a| a.id)
                };
                if let Some(account_id) = first {
                    window.open_search(account_id, "");
                }
            })
            .build();
        self.add_action_entries([sign_in, refresh, search]);
        self.imp().sign_in_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.start_sign_in()
        ));
    }

    /// The sidebar: every account's projects, one row each. Rows are built
    /// from `SidebarEntry`s in a ListStore so a refresh splices in place.
    fn setup_sidebar(&self) {
        let imp = self.imp();
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        imp.sidebar_list.bind_model(Some(&store), |object| {
            let entry = object
                .downcast_ref::<glib::BoxedAnyObject>()
                .expect("a boxed entry");
            sidebar_row(&entry.borrow::<SidebarEntry>())
        });
        imp.sidebar_list.connect_row_activated(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, row| {
                let store = window
                    .imp()
                    .sidebar_store
                    .get()
                    .expect("set in setup_sidebar");
                let Some(entry) = store
                    .item(row.index() as u32)
                    .and_downcast::<glib::BoxedAnyObject>()
                else {
                    return;
                };
                let entry = entry.borrow::<SidebarEntry>().clone();
                match entry {
                    SidebarEntry::Hey { account_id, .. } => window.open_hey(account_id),
                    SidebarEntry::Pings { account_id } => window.open_pings(account_id),
                    SidebarEntry::Assignments { account_id } => window.open_assignments(account_id),
                    SidebarEntry::Search { account_id } => window.open_search(account_id, ""),
                    SidebarEntry::Header(_) => {}
                    SidebarEntry::Project {
                        account_id,
                        project,
                    } => window.open_project(account_id, project),
                }
            }
        ));
        let _ = imp.sidebar_store.set(store);
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
            let db = self.db();
            let db = db.borrow();
            let mut state = self.state_mut();
            for account in accounts {
                // What was listed last time, until the network answers.
                match db.cached_projects(account.id) {
                    Ok(projects) if !projects.is_empty() => {
                        state.projects.insert(account.id, projects);
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!("could not read cached projects: {error}"),
                }
                state.accounts.insert(account.id, account);
            }
        }
        self.imp().main_stack.set_visible_child_name(PAGE_MAIN);
        self.rebuild_sidebar();
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

    /// Forget an account: its tokens (when no other account shares the
    /// identity), its cache, its feed, its pages.
    pub fn sign_out(&self, account_id: i64) {
        let (identity_id, shares_identity) = {
            let mut state = self.state_mut();
            let Some(account) = state.accounts.remove(&account_id) else {
                return;
            };
            state.clients.remove(&account_id);
            state.projects.remove(&account_id);
            state.unread_counts.remove(&account_id);
            if let Some(feed) = state.feeds.remove(&account_id) {
                feed.abort();
            }
            let shares = state
                .accounts
                .values()
                .any(|a| a.identity_id == account.identity_id);
            if !shares {
                state.tokens.remove(&account.identity_id);
            }
            (account.identity_id, shares)
        };
        if let Err(error) = self.db().borrow().remove_account(account_id) {
            self.report("could not remove the account", &error);
        }
        if !shares_identity {
            rt::run(
                async move { campfe_core::secrets::clear_tokens(identity_id).await },
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |result| {
                        if let Err(error) = result {
                            window.report("could not clear the keyring", &error);
                        }
                    }
                ),
            );
        }
        self.imp().content_nav.pop_to_tag("home");
        if self.state().accounts.is_empty() {
            self.imp()
                .main_stack
                .set_visible_child_name(PAGE_NO_ACCOUNT);
        }
        self.rebuild_sidebar();
        self.toast(&gettext("Signed out."));
    }

    /// A desktop notification for what's new in Hey!, opening it when clicked.
    fn notify_unreads(&self, account_id: i64, new: &[campfe_core::models::Reading]) {
        let Some(app) = self.application() else {
            return;
        };
        if !self.settings().boolean(keys::NOTIFICATIONS) {
            return;
        }
        let Some(first) = new.first() else {
            return;
        };
        let title = if new.len() == 1 {
            first.title.clone()
        } else {
            crate::i18n::format(
                &gettext("{n} new in Hey!"),
                &[("n", &new.len().to_string())],
            )
        };
        let mut body = Vec::new();
        if !first.creator.name.is_empty() {
            body.push(first.creator.name.clone());
        }
        if !first.bucket_name.is_empty() {
            body.push(first.bucket_name.clone());
        }
        let excerpt = campfe_core::richtext::excerpt(&first.content_excerpt, 120);
        let notification = gio::Notification::new(&title);
        notification.set_body(Some(&if excerpt.is_empty() {
            body.join(" · ")
        } else {
            format!("{}\n{}", body.join(" · "), excerpt)
        }));
        notification
            .set_default_action_and_target_value("app.open-hey", Some(&account_id.to_variant()));
        app.send_notification(Some(&format!("hey-{account_id}")), &notification);
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
            self.fetch_unread_count(id);
            self.start_feed(id);
        }
    }

    /// Hey!'s unread count, for the sidebar badge.
    pub fn fetch_unread_count(&self, account_id: i64) {
        let Some(client) = self.state().clients.get(&account_id).cloned() else {
            return;
        };
        rt::run(
            async move {
                client
                    .get::<campfe_core::models::Readings>("/my/readings.json")
                    .await
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| match result {
                    Ok(readings) => {
                        let before = window
                            .state_mut()
                            .unread_counts
                            .insert(account_id, readings.unreads.len());
                        window.rebuild_sidebar();
                        // More than last time, and the window isn't being
                        // looked at: say so on the desktop.
                        if let Some(before) = before
                            && readings.unreads.len() > before
                            && !window.is_active()
                        {
                            window.notify_unreads(
                                account_id,
                                &readings.unreads[..readings.unreads.len() - before],
                            );
                        }
                    }
                    Err(error) =>
                        tracing::warn!("could not count unreads of {account_id}: {error}"),
                }
            ),
        );
    }

    fn refresh_projects(&self) {
        let ids: Vec<i64> = self.state().clients.keys().copied().collect();
        for id in ids {
            self.fetch_projects(id);
            self.fetch_unread_count(id);
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
        if let Err(error) = self.db().borrow_mut().store_projects(account_id, &projects) {
            tracing::warn!("could not cache the projects of {account_id}: {error}");
        }
        self.state_mut().projects.insert(account_id, projects);
        self.rebuild_sidebar();
        self.debug_open(account_id);
    }

    /// A developer hook: `CAMPFE_DEBUG_OPEN=<project id>[:<tool id>[:first]]`,
    /// or `hey`, `pings`, `assignments` or `search[:<words>]`, or a web link (`https://...`) the
    /// way Hey! opens one, opens that page as soon as the account's projects
    /// are listed, for a look without clicking through; `:first` goes on
    /// into the tool's first item. Acted on once.
    fn debug_open(&self, account_id: i64) {
        let Ok(target) = std::env::var("CAMPFE_DEBUG_OPEN") else {
            return;
        };
        match target.as_str() {
            search if search == "search" || search.starts_with("search:") => {
                // SAFETY: the main thread, with no other thread reading the environment.
                unsafe { std::env::remove_var("CAMPFE_DEBUG_OPEN") };
                let query = search
                    .strip_prefix("search:")
                    .unwrap_or_default()
                    .to_string();
                self.open_search(account_id, &query);
                return;
            }
            url if url.starts_with("https://") => {
                // SAFETY: the main thread, with no other thread reading the environment.
                unsafe { std::env::remove_var("CAMPFE_DEBUG_OPEN") };
                if !self.open_app_url(url) {
                    tracing::warn!("debug open: Campfe has no page for {url}");
                }
                return;
            }
            "hey" | "pings" | "assignments" => {
                // SAFETY: the main thread, with no other thread reading the environment.
                unsafe { std::env::remove_var("CAMPFE_DEBUG_OPEN") };
                match target.as_str() {
                    "hey" => self.open_hey(account_id),
                    "pings" => self.open_pings(account_id),
                    _ => self.open_assignments(account_id),
                }
                return;
            }
            _ => {}
        }
        let mut parts = target.split(':');
        let project_id = parts.next().and_then(|p| p.parse::<i64>().ok());
        let tool_id = parts.next().and_then(|t| t.parse::<i64>().ok());
        let first = parts.next() == Some("first");
        let project = self
            .state()
            .projects
            .get(&account_id)
            .and_then(|projects| projects.iter().find(|p| Some(p.id) == project_id).cloned());
        let Some(project) = project else {
            return;
        };
        // SAFETY: the main thread, with no other thread reading the environment.
        unsafe { std::env::remove_var("CAMPFE_DEBUG_OPEN") };
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        self.open_project(account_id, project.clone());
        if let Some(tool) = project.dock.iter().find(|t| Some(t.id) == tool_id) {
            self.open_tool(client.clone(), &project, tool);
            if first {
                self.debug_open_first(client, tool);
            }
        }
    }

    /// The `:first` of the hook: the tool's first message or to-do list.
    fn debug_open_first(&self, client: Client, tool: &campfe_core::models::DockTool) {
        let kind = tool.name.clone();
        let tool_id = tool.id;
        rt::run(
            {
                let client = client.clone();
                async move {
                    match kind.as_str() {
                        "message_board" => client
                            .get_page::<campfe_core::models::Message>(&format!(
                                "/message_boards/{tool_id}/messages.json"
                            ))
                            .await
                            .map(|page| page.items.into_iter().next().map(DebugFirst::Message)),
                        "todoset" => client
                            .get_page::<campfe_core::models::Todolist>(&format!(
                                "/todosets/{tool_id}/todolists.json"
                            ))
                            .await
                            .map(|page| page.items.into_iter().next().map(DebugFirst::Todolist)),
                        "vault" => client
                            .get_page::<campfe_core::models::Upload>(&format!(
                                "/vaults/{tool_id}/uploads.json"
                            ))
                            .await
                            .map(|page| page.items.into_iter().next().map(DebugFirst::Upload)),
                        _ => Ok(None),
                    }
                }
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| match result {
                    Ok(Some(DebugFirst::Message(message))) => window.open_message(client, message),
                    Ok(Some(DebugFirst::Todolist(list))) => window.open_todolist(client, list),
                    Ok(Some(DebugFirst::Upload(upload))) => window.open_upload(client, upload),
                    Ok(None) => {}
                    Err(error) => tracing::warn!("debug open: {error}"),
                }
            ),
        );
    }

    fn rebuild_sidebar(&self) {
        let imp = self.imp();
        let store = imp.sidebar_store.get().expect("set in setup_sidebar");
        let entries: Vec<SidebarEntry> = {
            let state = self.state();
            let mut accounts: Vec<&Account> = state.accounts.values().collect();
            accounts.sort_by(|a, b| a.name.cmp(&b.name));
            if let Some(first) = accounts.first() {
                imp.sidebar_title.set_title(&first.name);
                imp.sidebar_title.set_subtitle(&first.identity_email);
            }
            let several = accounts.len() > 1;
            let mut entries = Vec::new();
            for account in &accounts {
                let account_id = account.id;
                // With several accounts, each gets its own "me" section.
                if several {
                    entries.push(SidebarEntry::Header(account.name.clone()));
                }
                entries.push(SidebarEntry::Hey {
                    account_id,
                    unread: state.unread_counts.get(&account_id).copied().unwrap_or(0),
                });
                entries.push(SidebarEntry::Pings { account_id });
                entries.push(SidebarEntry::Assignments { account_id });
                entries.push(SidebarEntry::Search { account_id });
                entries.push(SidebarEntry::Header(if several {
                    crate::i18n::format(
                        &gettext("{account} projects"),
                        &[("account", &account.name)],
                    )
                } else {
                    gettext("Projects")
                }));
                entries.extend(state.projects.get(&account_id).into_iter().flatten().map(
                    |project| SidebarEntry::Project {
                        account_id,
                        project: project.clone(),
                    },
                ));
            }
            entries
        };
        let count = entries
            .iter()
            .filter(|e| matches!(e, SidebarEntry::Project { .. }))
            .count();
        let objects: Vec<glib::BoxedAnyObject> =
            entries.into_iter().map(glib::BoxedAnyObject::new).collect();
        store.splice(0, store.n_items(), &objects);
        imp.content_placeholder
            .set_description(Some(&crate::i18n::format(
                &gettext("{count} projects loaded. Pick one to open it."),
                &[("count", &count.to_string())],
            )));
    }
}

/// What the `:first` debug hook found to open.
enum DebugFirst {
    Message(campfe_core::models::Message),
    Todolist(campfe_core::models::Todolist),
    Upload(campfe_core::models::Upload),
}

/// The widget for one sidebar entry.
fn sidebar_row(entry: &SidebarEntry) -> gtk::Widget {
    let (title, icon, badge) = match entry {
        SidebarEntry::Header(text) => {
            let label = gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .css_classes(["heading", "dim-label", "caption-heading"])
                .margin_top(14)
                .margin_bottom(4)
                .margin_start(12)
                .build();
            let row = gtk::ListBoxRow::builder()
                .child(&label)
                .activatable(false)
                .selectable(false)
                .focusable(false)
                .build();
            return row.upcast();
        }
        SidebarEntry::Hey { unread, .. } => (gettext("Hey!"), "mail-unread-symbolic", *unread),
        SidebarEntry::Pings { .. } => (gettext("Pings"), "user-available-symbolic", 0),
        SidebarEntry::Assignments { .. } => {
            (gettext("My Assignments"), "object-select-symbolic", 0)
        }
        SidebarEntry::Search { .. } => (gettext("Search"), "system-search-symbolic", 0),
        SidebarEntry::Project { project, .. } => (project.name.clone(), "", 0),
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .activatable(true)
        .build();
    if !icon.is_empty() {
        row.add_prefix(&gtk::Image::from_icon_name(icon));
    }
    if badge > 0 {
        let label = gtk::Label::builder()
            .label(badge.to_string())
            .css_classes(["badge"])
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&label);
    }
    row.upcast()
}
