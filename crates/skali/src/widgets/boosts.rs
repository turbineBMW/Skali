//! Boosts outside the rich-text view: the dialog that asks what to boost
//! with (shared with the document's + Boost), and boosts as native chips
//! for chat lines.

use crate::i18n::gettext;
use adw::prelude::*;
use gtk::glib;
use skali_core::models::Boost;
use std::rc::Rc;

/// The quick choices, before the words.
const QUICK: [&str; 6] = ["👍", "❤️", "🎉", "👏", "😂", "🙌"];

/// Ask what to boost with: one of a few emoji, or up to 16 characters.
/// `on_boost` gets the choice; cancelling calls nothing.
pub fn ask(parent: &impl IsA<gtk::Widget>, on_boost: impl Fn(String) + 'static) {
    let on_boost = Rc::new(on_boost);
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Boost"))
        .close_response("cancel")
        .default_response("boost")
        .build();
    dialog.add_response("cancel", &gettext("Cancel"));
    dialog.add_response("boost", &gettext("Boost"));
    dialog.set_response_appearance("boost", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("boost", false);
    let entry = gtk::Entry::builder()
        .max_length(16)
        .placeholder_text(gettext("Or type a few words"))
        .activates_default(true)
        .build();
    entry.connect_changed(glib::clone!(
        #[weak]
        dialog,
        move |entry| dialog.set_response_enabled("boost", !entry.text().trim().is_empty())
    ));
    let quick = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    for emoji in QUICK {
        let button = gtk::Button::builder()
            .label(emoji)
            .css_classes(["flat", "title-3"])
            .build();
        button.connect_clicked(glib::clone!(
            #[strong]
            on_boost,
            #[weak]
            dialog,
            move |_| {
                on_boost(emoji.to_string());
                dialog.close();
            }
        ));
        quick.append(&button);
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.append(&quick);
    column.append(&entry);
    dialog.set_extra_child(Some(&column));
    dialog.connect_response(
        Some("boost"),
        glib::clone!(
            #[weak]
            entry,
            move |_, _| {
                let text = entry.text().trim().to_string();
                if !text.is_empty() {
                    on_boost(text);
                }
            }
        ),
    );
    dialog.present(Some(parent));
    // Typing goes into the words; the emoji are a click (or Tab) away.
    dialog.set_focus(Some(&entry));
}

/// Fill `row` with one chip per boost, as Basecamp shows them: the
/// booster's face and what they boosted with, their name the tooltip.
/// Yours, when `me` is known and writes are allowed, is a button that
/// takes it back. After them, a round button that adds one (`on_add`).
pub fn fill_chips(
    row: &gtk::Box,
    boosts: &[Boost],
    me: Option<i64>,
    can_write: bool,
    on_unboost: impl Fn(i64) + Clone + 'static,
    on_add: impl Fn(&gtk::Button) + 'static,
) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    for boost in boosts {
        let content = gtk::Box::builder().spacing(4).build();
        let face = adw::Avatar::builder()
            .size(22)
            .text(&boost.booster.name)
            .show_initials(true)
            .build();
        crate::avatars::load_into(&face, &boost.booster.avatar_url);
        content.append(&face);
        content.append(&gtk::Label::new(Some(&boost.content)));
        if can_write && me == Some(boost.booster.id) {
            let chip = gtk::Button::builder()
                .child(&content)
                .tooltip_text(gettext("Take your boost back"))
                .css_classes(["boost-chip", "mine"])
                .build();
            let boost_id = boost.id;
            let on_unboost = on_unboost.clone();
            chip.connect_clicked(move |chip| {
                chip.set_sensitive(false);
                on_unboost(boost_id);
            });
            row.append(&chip);
        } else {
            content.set_tooltip_text(Some(&boost.booster.name));
            content.add_css_class("boost-chip");
            row.append(&content);
        }
    }
    if can_write && !boosts.is_empty() {
        let add = gtk::Button::builder()
            .icon_name("face-smile-big-symbolic")
            .tooltip_text(gettext("Boost"))
            .valign(gtk::Align::Center)
            .css_classes(["circular", "boost-more"])
            .build();
        add.connect_clicked(move |add| on_add(add));
        row.append(&add);
    }
    row.set_visible(!boosts.is_empty());
}
