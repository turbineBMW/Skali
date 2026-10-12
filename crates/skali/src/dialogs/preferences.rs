//! Preferences: the accounts, with a way out of each, and the few
//! settings there are. The switches and the KLIPY key are bound to
//! GSettings; the theme and notification code watch those keys, and the
//! GIF picker reads the key at each search.

use crate::i18n::gettext;
use crate::settings as keys;
use crate::window::SkaliWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gio;
use gtk::glib;
use skali_core::models::Account;
use skali_core::summary::Backend;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/turbinebmw/Skali/ui/preferences-dialog.ui")]
    pub struct PreferencesDialog {
        #[template_child]
        pub accounts_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub add_account_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub follow_omarchy_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub notifications_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub klipy_key_row: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub summarizer_row: TemplateChild<adw::ComboRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PreferencesDialog {
        const NAME: &'static str = "SkaliPreferencesDialog";
        type Type = super::PreferencesDialog;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PreferencesDialog {}
    impl WidgetImpl for PreferencesDialog {}
    impl AdwDialogImpl for PreferencesDialog {}
    impl PreferencesDialogImpl for PreferencesDialog {}
}

glib::wrapper! {
    pub struct PreferencesDialog(ObjectSubclass<imp::PreferencesDialog>)
        @extends adw::PreferencesDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PreferencesDialog {
    pub fn new(settings: &gio::Settings, window: &SkaliWindow) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        settings
            .bind(
                keys::FOLLOW_OMARCHY_THEME,
                &*imp.follow_omarchy_row,
                "active",
            )
            .build();
        settings
            .bind(keys::NOTIFICATIONS, &*imp.notifications_row, "active")
            .build();
        settings
            .bind(keys::KLIPY_API_KEY, &*imp.klipy_key_row, "text")
            .build();
        bind_summarizer(settings, &imp.summarizer_row);
        imp.add_account_button.connect_clicked(glib::clone!(
            #[weak]
            window,
            #[weak]
            dialog,
            move |_| {
                dialog.close();
                window.start_sign_in();
            }
        ));
        let mut accounts: Vec<Account> = window.state().accounts.values().cloned().collect();
        accounts.sort_by(|a, b| a.name.cmp(&b.name));
        if accounts.is_empty() {
            imp.accounts_group.add(
                &adw::ActionRow::builder()
                    .title(gettext("Not signed in"))
                    .build(),
            );
        }
        for account in accounts {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&account.name))
                .subtitle(glib::markup_escape_text(&format!(
                    "{} <{}>",
                    account.identity_name, account.identity_email
                )))
                .build();
            let sign_out = gtk::Button::builder()
                .label(gettext("Sign out"))
                .valign(gtk::Align::Center)
                .css_classes(["destructive-action", "flat"])
                .build();
            let account_id = account.id;
            sign_out.connect_clicked(glib::clone!(
                #[weak]
                window,
                #[weak]
                dialog,
                move |_| {
                    dialog.close();
                    window.sign_out(account_id);
                }
            ));
            row.add_suffix(&sign_out);
            imp.accounts_group.add(&row);
        }
        dialog
    }
}

/// The summarizer setting's values, in the combo row's order.
const SUMMARIZERS: [&str; 3] = ["off", "claude", "codex"];

/// The combo row picks the `summarizer` setting, and says when the program
/// it names isn't to be found.
fn bind_summarizer(settings: &gio::Settings, row: &adw::ComboRow) {
    let explain = |row: &adw::ComboRow, value: &str| {
        let missing = Backend::from_setting(value).filter(|backend| backend.find().is_none());
        row.set_subtitle(&match missing {
            Some(backend) => glib::markup_escape_text(&crate::i18n::format(
                &gettext("“{program}” isn't installed, or Skali can't find it"),
                &[("program", backend.setting())],
            ))
            .to_string(),
            None => String::new(),
        });
    };
    let value = settings.string(keys::SUMMARIZER);
    let index = SUMMARIZERS.iter().position(|v| *v == value).unwrap_or(0);
    row.set_selected(index as u32);
    explain(row, &value);
    row.connect_selected_notify(glib::clone!(
        #[strong]
        settings,
        move |row| {
            let value = SUMMARIZERS
                .get(row.selected() as usize)
                .copied()
                .unwrap_or("off");
            if let Err(error) = settings.set_string(keys::SUMMARIZER, value) {
                tracing::warn!("could not save the summarizer: {error}");
            }
            explain(row, value);
        }
    ));
}
