//! The jump menu (Ctrl+J): every sidebar destination -- the "me" pages and
//! the projects -- filtered as you type. Return opens the selected one;
//! Up and Down move the selection without leaving the search.

use crate::i18n::gettext;
use crate::window::{SidebarEntry, SkaliWindow, entry_label, sidebar_row};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::rc::Rc;

/// Whether every word of `query` is somewhere in `text` (both lowercase).
fn matches(text: &str, query: &str) -> bool {
    query.split_whitespace().all(|word| text.contains(word))
}

pub fn present(window: &SkaliWindow) {
    let entries: Vec<SidebarEntry> = window
        .sidebar_entries()
        .into_iter()
        .filter(|e| entry_label(e).is_some())
        .collect();
    if entries.is_empty() {
        return;
    }
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Browse)
        .css_classes(["navigation-sidebar"])
        .build();
    // What each row is found by: its title and, with several accounts, the
    // account's name (which the row shows too).
    let mut keys = Vec::with_capacity(entries.len());
    for entry in &entries {
        let row = sidebar_row(entry);
        let (title, _, _) = entry_label(entry).unwrap_or_default();
        let account = account_of(entry).and_then(|id| window.account_label(id));
        if let (Some(account), Some(row)) = (&account, row.downcast_ref::<adw::ActionRow>()) {
            row.set_subtitle(&glib::markup_escape_text(account));
        }
        keys.push(format!("{title} {}", account.unwrap_or_default()).to_lowercase());
        list.append(&row);
    }
    let query = Rc::new(RefCell::new(String::new()));
    let keys = Rc::new(keys);
    list.set_filter_func(glib::clone!(
        #[strong]
        query,
        #[strong]
        keys,
        move |row| matches(&keys[row.index() as usize], &query.borrow())
    ));

    let search = gtk::SearchEntry::builder()
        .placeholder_text(gettext("Jump to a project or page"))
        .hexpand(true)
        .build();
    let header = adw::HeaderBar::builder()
        .title_widget(&search)
        .show_end_title_buttons(false)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build(),
    ));
    let dialog = adw::Dialog::builder()
        .title(gettext("Jump to"))
        .content_width(460)
        .content_height(480)
        .child(&toolbar)
        .build();

    // The rows still shown, in order.
    let visible = glib::clone!(
        #[weak]
        list,
        #[strong]
        query,
        #[strong]
        keys,
        #[upgrade_or_default]
        move || -> Vec<gtk::ListBoxRow> {
            let query = query.borrow();
            (0..keys.len() as i32)
                .filter_map(|i| list.row_at_index(i))
                .filter(|row| matches(&keys[row.index() as usize], &query))
                .collect()
        }
    );
    let visible = Rc::new(visible);
    search.connect_search_changed(glib::clone!(
        #[weak]
        list,
        #[strong]
        query,
        #[strong]
        visible,
        move |search| {
            *query.borrow_mut() = search.text().to_lowercase();
            list.invalidate_filter();
            list.select_row(visible().first());
        }
    ));
    // Up and Down move the selection while typing goes on.
    let keys_controller = gtk::EventControllerKey::new();
    keys_controller.connect_key_pressed(glib::clone!(
        #[weak]
        list,
        #[strong]
        visible,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            let step: i32 = match key {
                gdk::Key::Down | gdk::Key::KP_Down => 1,
                gdk::Key::Up | gdk::Key::KP_Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let rows = visible();
            if rows.is_empty() {
                return glib::Propagation::Stop;
            }
            let at = list
                .selected_row()
                .and_then(|selected| rows.iter().position(|r| *r == selected));
            let next = match at {
                Some(at) => (at as i32 + step).clamp(0, rows.len() as i32 - 1) as usize,
                None => 0,
            };
            list.select_row(Some(&rows[next]));
            rows[next].grab_focus();
            glib::Propagation::Stop
        }
    ));
    search.add_controller(keys_controller);
    // Keep typing into the search when focus went down to a row.
    search.set_key_capture_widget(Some(&list));

    let entries = Rc::new(entries);
    list.connect_row_activated(glib::clone!(
        #[weak]
        window,
        #[weak]
        dialog,
        #[strong]
        entries,
        move |_, row| {
            if let Some(entry) = entries.get(row.index() as usize) {
                dialog.close();
                window.open_entry(entry.clone());
            }
        }
    ));
    search.connect_activate(glib::clone!(
        #[weak]
        list,
        #[strong]
        visible,
        move |_| {
            let row = list.selected_row().or_else(|| visible().into_iter().next());
            if let Some(row) = row {
                row.activate();
            }
        }
    ));
    list.select_row(list.row_at_index(0).as_ref());
    dialog.present(Some(window));
    search.grab_focus();
}

fn account_of(entry: &SidebarEntry) -> Option<i64> {
    match entry {
        SidebarEntry::Header(_) => None,
        SidebarEntry::Hey { account_id, .. }
        | SidebarEntry::Pings { account_id }
        | SidebarEntry::Assignments { account_id }
        | SidebarEntry::Bookmarks { account_id }
        | SidebarEntry::Timeline { account_id }
        | SidebarEntry::Notes { account_id }
        | SidebarEntry::Search { account_id }
        | SidebarEntry::Project { account_id, .. } => Some(*account_id),
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn every_word_must_match() {
        assert!(matches("my project omacom", "proj"));
        assert!(matches("my project omacom", "omac my"));
        assert!(!matches("my project omacom", "proj hey"));
        assert!(matches("anything", ""));
    }
}
