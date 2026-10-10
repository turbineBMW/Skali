//! The application: one database and settings object shared by every
//! window, the app-level actions, and the `--hidden` background start.

use crate::config::{APP_ID, RESOURCE_PATH, VERSION};
use crate::dialogs::preferences::PreferencesDialog;
use crate::i18n::gettext;
use crate::omarchy::OmarchyTheme;
use crate::settings;
use crate::window::SkaliWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gio;
use gtk::glib;
use skali_core::db::Database;
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SkaliApplication {
        pub db: OnceCell<Rc<RefCell<Database>>>,
        pub settings: OnceCell<gio::Settings>,
        pub omarchy_theme: OnceCell<OmarchyTheme>,
        /// Build the window (so syncing runs) but don't present it.
        pub should_start_hidden: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SkaliApplication {
        const NAME: &'static str = "SkaliApplication";
        type Type = super::SkaliApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for SkaliApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let app = self.obj();
            app.add_main_option(
                "hidden",
                glib::Char::from(0u8),
                glib::OptionFlags::NONE,
                glib::OptionArg::None,
                &gettext("Start in the background without showing a window"),
                None,
            );
            app.setup_actions();
        }
    }

    impl ApplicationImpl for SkaliApplication {
        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            app.open_database();
            app.load_css();
        }

        fn handle_local_options(
            &self,
            options: &glib::VariantDict,
        ) -> std::ops::ControlFlow<glib::ExitCode> {
            self.should_start_hidden.set(options.contains("hidden"));
            self.parent_handle_local_options(options)
        }

        fn activate(&self) {
            let app = self.obj();
            let window = app.main_window().unwrap_or_else(|| app.new_window());
            if self.should_start_hidden.replace(false) {
                // Only the launch activation stays hidden; later ones raise it.
                return;
            }
            window.present();
        }
    }
    impl GtkApplicationImpl for SkaliApplication {}
    impl AdwApplicationImpl for SkaliApplication {}
}

glib::wrapper! {
    pub struct SkaliApplication(ObjectSubclass<imp::SkaliApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for SkaliApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl SkaliApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("resource-base-path", RESOURCE_PATH)
            .build()
    }

    pub fn db(&self) -> Rc<RefCell<Database>> {
        self.imp().db.get().expect("opened at startup").clone()
    }

    pub fn settings(&self) -> gio::Settings {
        self.imp()
            .settings
            .get()
            .expect("loaded at startup")
            .clone()
    }

    fn open_database(&self) {
        let data_dir = glib::user_data_dir().join("skali");
        if let Err(error) = std::fs::create_dir_all(&data_dir) {
            tracing::error!("could not create {}: {error}", data_dir.display());
        }
        let path = data_dir.join("skali.db");
        let db = Database::open(&path)
            .unwrap_or_else(|error| panic!("could not open {}: {error}", path.display()));
        let _ = self.imp().db.set(Rc::new(RefCell::new(db)));
        let _ = self.imp().settings.set(settings::load());
    }

    pub(crate) fn new_window(&self) -> SkaliWindow {
        SkaliWindow::new(self, self.db(), &self.settings())
    }

    pub(crate) fn main_window(&self) -> Option<SkaliWindow> {
        self.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SkaliWindow>().ok())
    }

    fn load_css(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let provider = gtk::CssProvider::new();
        provider.load_from_resource(&format!("{RESOURCE_PATH}/style.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        crate::accent::install_fallback(&display);
        let _ = self
            .imp()
            .omarchy_theme
            .set(OmarchyTheme::install(&display, &self.settings()));
    }

    fn setup_actions(&self) {
        let about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| app.show_about())
            .build();
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| app.quit())
            .build();
        let preferences = gio::ActionEntry::builder("preferences")
            .activate(|app: &Self, _, _| {
                app.activate();
                if let Some(window) = app.main_window() {
                    PreferencesDialog::new(&app.settings(), &window).present(Some(&window));
                }
            })
            .build();
        // Reachable from outside (`gapplication action <id> sign-in`), which
        // the window's own action isn't.
        let sign_in = gio::ActionEntry::builder("sign-in")
            .activate(|app: &Self, _, _| {
                app.activate();
                if let Some(window) = app.main_window() {
                    window.start_sign_in();
                }
            })
            .build();
        // A Hey! notification's click: show the window on that account's Hey!.
        let open_hey = gio::ActionEntry::builder("open-hey")
            .parameter_type(Some(glib::VariantTy::INT64))
            .activate(|app: &Self, _, parameter| {
                app.activate();
                if let (Some(window), Some(account_id)) =
                    (app.main_window(), parameter.and_then(|p| p.get::<i64>()))
                {
                    window.open_hey(account_id);
                }
            })
            .build();
        let shortcuts = gio::ActionEntry::builder("shortcuts")
            .activate(|app: &Self, _, _| {
                if let Some(window) = app.main_window() {
                    crate::dialogs::shortcuts::dialog().present(Some(&window));
                }
            })
            .build();
        self.add_action_entries([about, quit, preferences, sign_in, open_hey, shortcuts]);

        // Listed for people in dialogs/shortcuts.rs: keep the two together.
        for (name, accels) in [
            ("app.quit", &["<control>q"][..]),
            ("app.preferences", &["<control>comma"]),
            ("app.shortcuts", &["<control>question"]),
            ("win.sign-in", &["<control>l"]),
            ("win.refresh", &["F5"]),
            ("win.search", &["<control>f"]),
            ("win.jump", &["<control>j", "<control>k"]),
            ("win.go::hey", &["<alt>1"]),
            ("win.go::pings", &["<alt>2"]),
            ("win.go::assignments", &["<alt>3"]),
            ("win.go::bookmarks", &["<alt>4"]),
            ("win.go::timeline", &["<alt>5"]),
            ("win.go::notes", &["<alt>6"]),
        ] {
            self.set_accels_for_action(name, accels);
        }
    }

    fn show_about(&self) {
        let about =
            adw::AboutDialog::from_appdata(&format!("{RESOURCE_PATH}/metainfo.xml"), Some(VERSION));
        about.set_translator_credits(&gettext("translator-credits"));
        about.set_developers(&["Brandon Williams"]);
        about.set_copyright("© 2026 Brandon Williams");
        about.present(self.active_window().as_ref());
    }
}
