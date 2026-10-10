//! Signing in with a personal access token, for when the browser sign-in
//! can't be used: Basecamp makes the token, the person pastes it here, and
//! Basecamp says whose it is before anything is kept.

use crate::i18n::gettext;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;

/// Where Basecamp makes personal access tokens.
const TOKENS_PAGE: &str = "https://app.basecamp.com/my/access_tokens";

pub fn present(window: &SkaliWindow) {
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Sign in with a token"))
        .body_use_markup(true)
        .body(crate::i18n::format(
            &gettext(
                "Make a personal access token in Basecamp at <a href=\"{url}\">app.basecamp.com/my/access_tokens</a>, then paste it here. It's kept in your keyring.",
            ),
            &[("url", TOKENS_PAGE)],
        ))
        .close_response("cancel")
        .default_response("sign-in")
        .build();
    dialog.add_response("cancel", &gettext("Cancel"));
    dialog.add_response("sign-in", &gettext("Sign In"));
    dialog.set_response_appearance("sign-in", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("sign-in", false);
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text(gettext("Personal access token"))
        .activates_default(true)
        .build();
    entry.connect_changed(glib::clone!(
        #[weak]
        dialog,
        move |entry| dialog.set_response_enabled("sign-in", !entry.text().trim().is_empty())
    ));
    dialog.set_extra_child(Some(&entry));
    dialog.connect_response(
        Some("sign-in"),
        glib::clone!(
            #[weak]
            window,
            #[weak]
            entry,
            move |_, _| {
                let token = entry.text().trim().to_string();
                entry.set_text("");
                if !token.is_empty() {
                    window.sign_in_with_token(token);
                }
            }
        ),
    );
    dialog.present(Some(window));
    dialog.set_focus(Some(&entry));
}
