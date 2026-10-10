//! Preferences: the accounts, with a way out of each, and the few
//! settings there are. The switches are bound to GSettings; the theme
//! and notification code watch those keys.

use crate::i18n::gettext;
use crate::settings as keys;
use crate::window::SkaliWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gio;
use gtk::glib;
use skali_core::models::Account;

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
