//! gettext setup and the sentences the core's errors turn into.

use crate::config::GETTEXT_DOMAIN;
use campfe_core::error::ApiError;
pub use gettextrs::gettext;
use gettextrs::{LocaleCategory, bindtextdomain, setlocale, textdomain};

pub fn init() {
    // SAFETY: called once at startup, before any other thread exists.
    unsafe {
        setlocale(LocaleCategory::LcAll, "");
    }
    let _ = bindtextdomain(GETTEXT_DOMAIN, locale_dir());
    let _ = textdomain(GETTEXT_DOMAIN);
}

/// Where install.sh put the translations: `share/locale` beside the `bin`
/// the program runs from (~/.local by default, /usr system-wide). A
/// checkout run finds none there and shows the source strings.
fn locale_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share/locale")))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| "/usr/share/locale".into())
}

/// `gettext` with `{name}` placeholders substituted, in a form that keeps the
/// placeholders visible to translators.
pub fn format(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut text = template.to_string();
    for (key, value) in pairs {
        text = text.replace(&format!("{{{key}}}"), value);
    }
    text
}

/// What to tell the user about an error, in a sentence.
pub fn failure_message(error: &ApiError) -> String {
    match error {
        ApiError::Network(_) => gettext("Could not reach Basecamp. Check your connection."),
        ApiError::RateLimited(_) => {
            gettext("Basecamp asked us to slow down. Try again in a moment.")
        }
        ApiError::Unauthorized => {
            gettext("Basecamp no longer accepts this sign-in. Sign in again.")
        }
        ApiError::Auth(reason) => {
            format(&gettext("Sign-in failed: {reason}"), &[("reason", reason)])
        }
        ApiError::ReadOnly => gettext("Writes to this account are turned off."),
        ApiError::Keyring(reason) => format(
            &gettext("Could not use the keyring: {reason}"),
            &[("reason", reason)],
        ),
        ApiError::Http { status, .. } if *status >= 500 => {
            gettext("Basecamp had a problem. Try again in a moment.")
        }
        ApiError::Http { status, message } => format(
            &gettext("Basecamp answered {status}: {message}"),
            &[("status", &status.to_string()), ("message", message)],
        ),
        ApiError::Decode(_) => gettext("Basecamp sent something Campfe didn't understand."),
        ApiError::Db(error) => format(
            &gettext("Database error: {error}"),
            &[("error", &error.to_string())],
        ),
        ApiError::Io(error) => error.to_string(),
    }
}
