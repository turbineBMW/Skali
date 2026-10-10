//! The keyboard shortcuts (Ctrl+?), as set in application.rs: change one,
//! change both.

use crate::i18n::gettext;

pub fn dialog() -> adw::ShortcutsDialog {
    let dialog = adw::ShortcutsDialog::new();
    let section = |title: String, items: &[(String, &str)]| {
        let section = adw::ShortcutsSection::new(Some(&title));
        for (label, accel) in items {
            section.add(adw::ShortcutsItem::new(label, accel));
        }
        section
    };
    dialog.add(section(
        gettext("General"),
        &[
            (gettext("Jump to a project or page"), "<control>j"),
            (gettext("Search"), "<control>f"),
            (gettext("Refresh projects"), "F5"),
            (gettext("Go back"), "<alt>Left"),
            (gettext("Sign in to Basecamp"), "<control>l"),
            (gettext("Preferences"), "<control>comma"),
            (gettext("Keyboard shortcuts"), "<control>question"),
            (gettext("Quit"), "<control>q"),
        ],
    ));
    dialog.add(section(
        gettext("Go to"),
        &[
            (gettext("Hey!"), "<alt>1"),
            (gettext("Pings"), "<alt>2"),
            (gettext("My Assignments"), "<alt>3"),
            (gettext("My Bookmarks"), "<alt>4"),
            (gettext("Timeline"), "<alt>5"),
            (gettext("My Notes"), "<alt>6"),
        ],
    ));
    dialog.add(section(
        gettext("Writing"),
        &[
            (gettext("Send a chat line"), "Return"),
            (gettext("Post a comment"), "<control>Return"),
        ],
    ));
    dialog
}
