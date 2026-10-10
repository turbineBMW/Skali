//! Search: one account's Basecamp, as you type, optionally one kind of
//! thing, best match or newest first. A result opens on Skali's page for
//! it when there is one (a comment scrolled to), in the browser otherwise.

use super::{avatar, kind_label, shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::{Client, Page, search_path};
use skali_core::error::Result;
use skali_core::links::AppLink;
use skali_core::models::{SearchMetadata, SearchResult};
use skali_core::richtext;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Fewer characters than this isn't worth asking about.
const MIN_QUERY: usize = 2;

/// The page's state. Widgets are held weakly: they own the closures that
/// hold this.
struct SearchPage {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    entry: glib::WeakRef<gtk::SearchEntry>,
    kinds: glib::WeakRef<gtk::DropDown>,
    order: glib::WeakRef<gtk::DropDown>,
    stack: glib::WeakRef<gtk::Stack>,
    list: glib::WeakRef<gtk::ListBox>,
    more: glib::WeakRef<gtk::Button>,
    /// The `type_names[]` key of each kind offered, by its position.
    kind_keys: RefCell<Vec<Option<String>>>,
    /// The next page of the current search.
    next: RefCell<Option<String>>,
    /// Bumped by every search, so a late answer to an earlier one is
    /// dropped.
    generation: Cell<u64>,
}

/// The page, searching for `query` at once if one is given.
pub fn page(window: &SkaliWindow, client: Client, query: &str) -> adw::NavigationPage {
    let entry = gtk::SearchEntry::builder()
        .placeholder_text(gettext("Search Basecamp"))
        .search_delay(300)
        .hexpand(true)
        .build();
    let kinds = gtk::DropDown::from_strings(&[&gettext("Everything")]);
    kinds.set_tooltip_text(Some(&gettext("What to search")));
    let order = gtk::DropDown::from_strings(&[&gettext("Best match"), &gettext("Newest first")]);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.append(&entry);
    controls.append(&kinds);
    controls.append(&order);
    let controls = adw::Clamp::builder()
        .maximum_size(800)
        .tightening_threshold(600)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .child(&controls)
        .build();

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let more = gtk::Button::builder()
        .label(gettext("Show more"))
        .halign(gtk::Align::Center)
        .visible(false)
        .css_classes(["pill"])
        .build();
    let results = gtk::Box::new(gtk::Orientation::Vertical, 18);
    results.append(&list);
    results.append(&more);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(
        &adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title(gettext("Search Basecamp"))
            .description(gettext("Messages, to-dos, chats, files and more."))
            .vexpand(true)
            .build(),
        Some("start"),
    );
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&results, 800), Some("list"));
    stack.add_named(
        &super::empty(&gettext("Nothing found."), "system-search-symbolic"),
        Some("empty"),
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&controls);
    content.append(&stack);
    let page = shell(&gettext("Search"), &content);

    let search = Rc::new(SearchPage {
        window: window.downgrade(),
        client,
        entry: entry.downgrade(),
        kinds: kinds.downgrade(),
        order: order.downgrade(),
        stack: stack.downgrade(),
        list: list.downgrade(),
        more: more.downgrade(),
        kind_keys: RefCell::new(vec![None]),
        next: RefCell::new(None),
        generation: Cell::new(0),
    });
    entry.connect_search_changed(glib::clone!(
        #[strong]
        search,
        move |_| search.search()
    ));
    kinds.connect_selected_notify(glib::clone!(
        #[strong]
        search,
        move |_| search.search()
    ));
    order.connect_selected_notify(glib::clone!(
        #[strong]
        search,
        move |_| search.search()
    ));
    more.connect_clicked(glib::clone!(
        #[strong]
        search,
        move |_| search.show_more()
    ));
    entry.connect_map(|entry| {
        entry.grab_focus();
    });
    search.load_kinds();
    entry.set_text(query);
    page
}

impl SearchPage {
    /// The kinds Basecamp can narrow a search to, into the drop-down.
    fn load_kinds(self: &Rc<Self>) {
        let client = self.client.clone();
        rt::run(
            async move {
                client
                    .get::<SearchMetadata>("/searches/metadata.json")
                    .await
            },
            glib::clone!(
                #[strong(rename_to = page)]
                self,
                move |result| {
                    let metadata = match result {
                        Ok(metadata) if !metadata.recording_search_types.is_empty() => metadata,
                        Ok(_) => return,
                        Err(error) => {
                            // Searching everything still works.
                            tracing::warn!("could not load the search kinds: {error}");
                            return;
                        }
                    };
                    let Some(kinds) = page.kinds.upgrade() else {
                        return;
                    };
                    *page.kind_keys.borrow_mut() = metadata
                        .recording_search_types
                        .iter()
                        .map(|kind| kind.key.clone())
                        .collect();
                    let names: Vec<&str> = metadata
                        .recording_search_types
                        .iter()
                        .map(|kind| kind.value.as_str())
                        .collect();
                    kinds.set_model(Some(&gtk::StringList::new(&names)));
                }
            ),
        );
    }

    /// Search again for what's typed, with the kind and order chosen.
    fn search(self: &Rc<Self>) {
        let (Some(entry), Some(kinds), Some(order), Some(stack), Some(list), Some(more)) = (
            self.entry.upgrade(),
            self.kinds.upgrade(),
            self.order.upgrade(),
            self.stack.upgrade(),
            self.list.upgrade(),
            self.more.upgrade(),
        ) else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        list.remove_all();
        more.set_visible(false);
        self.next.replace(None);
        let query = entry.text().trim().to_string();
        if query.chars().count() < MIN_QUERY {
            stack.set_visible_child_name("start");
            return;
        }
        let kind = self
            .kind_keys
            .borrow()
            .get(kinds.selected() as usize)
            .cloned()
            .flatten();
        let path = search_path(&query, kind.as_deref(), order.selected() == 1);
        stack.set_visible_child_name("loading");
        self.fetch(path, generation);
    }

    fn show_more(self: &Rc<Self>) {
        let Some(next) = self.next.take() else {
            return;
        };
        if let Some(more) = self.more.upgrade() {
            more.set_sensitive(false);
        }
        self.fetch(next, self.generation.get());
    }

    fn fetch(self: &Rc<Self>, path: String, generation: u64) {
        let client = self.client.clone();
        rt::run(
            async move { client.get_page::<SearchResult>(&path).await },
            glib::clone!(
                #[strong(rename_to = page)]
                self,
                move |result| page.fetched(generation, result)
            ),
        );
    }

    fn fetched(&self, generation: u64, result: Result<Page<SearchResult>>) {
        if generation != self.generation.get() {
            return;
        }
        let (Some(window), Some(stack), Some(list), Some(more)) = (
            self.window.upgrade(),
            self.stack.upgrade(),
            self.list.upgrade(),
            self.more.upgrade(),
        ) else {
            return;
        };
        more.set_sensitive(true);
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                window.report("could not search", &error);
                if list.first_child().is_none() {
                    stack.set_visible_child_name("empty");
                }
                return;
            }
        };
        for result in &page.items {
            list.append(&result_row(&window, result));
        }
        more.set_visible(page.next.is_some());
        self.next.replace(page.next);
        stack.set_visible_child_name(if list.first_child().is_some() {
            "list"
        } else {
            "empty"
        });
    }
}

fn result_row(window: &SkaliWindow, result: &SearchResult) -> adw::ActionRow {
    let recording = &result.recording;
    let kind = kind_label(&recording.kind);
    let title = if recording.title.is_empty() {
        kind.clone()
    } else {
        recording.title.clone()
    };
    let mut meta = vec![kind];
    if !recording.bucket.name.is_empty() {
        meta.push(recording.bucket.name.clone());
    }
    if !recording.creator.name.is_empty() {
        meta.push(recording.creator.name.clone());
    }
    meta.push(time_label(recording.created_at));
    let text = richtext::shorten(&result.plain_text_content, 140);
    let subtitle = if text.is_empty() || text == title {
        meta.join(" · ")
    } else {
        format!("{}\n{}", meta.join(" · "), text)
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .activatable(!recording.app_url.is_empty())
        .build();
    row.add_prefix(&avatar(&recording.creator, 32));
    let in_app = AppLink::parse(&recording.app_url).is_some();
    row.add_suffix(&gtk::Image::from_icon_name(if in_app {
        "go-next-symbolic"
    } else {
        "external-link-symbolic"
    }));
    let url = recording.app_url.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| {
            if !window.open_app_url(&url) {
                window.open_in_browser(&url);
            }
        }
    ));
    row
}
