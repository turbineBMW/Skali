//! Hey!: what's new for the signed-in person, unread first, then what's
//! been read. Opening an unread item marks it read; the header has a button
//! for marking all of them. A row opens its item on Campfe's own page when
//! there is one, in Basecamp's web app otherwise.

use super::{avatar, shell_with_header, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::links::AppLink;
use campfe_core::models::{Reading, Readings};
use campfe_core::richtext;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// The page's state. Widgets are held weakly: they own the closures that
/// hold this, and a strong ref back would keep the page alive forever.
struct HeyPage {
    window: glib::WeakRef<CampfeWindow>,
    account_id: i64,
    client: Client,
    stack: glib::WeakRef<gtk::Stack>,
    column: glib::WeakRef<gtk::Box>,
    mark_all: glib::WeakRef<gtk::Button>,
    /// `readable_sgid`s of what's still unread.
    unread: RefCell<Vec<String>>,
}

pub fn page(window: &CampfeWindow, account_id: i64, client: Client) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    stack.add_named(
        &super::empty(
            &gettext("Nothing new. Enjoy the quiet."),
            "mail-unread-symbolic",
        ),
        Some("empty"),
    );
    let (page, header) = shell_with_header(&gettext("Hey!"), &stack);
    let writes_allowed = client.writes_allowed();
    let mark_all = gtk::Button::builder()
        .icon_name("checkbox-checked-symbolic")
        .tooltip_text(if writes_allowed {
            gettext("Mark all as read")
        } else {
            gettext("Writes to this account are turned off.")
        })
        .sensitive(writes_allowed)
        .visible(false)
        .build();
    header.pack_end(&mark_all);

    let hey = Rc::new(HeyPage {
        window: window.downgrade(),
        account_id,
        client,
        stack: stack.downgrade(),
        column: column.downgrade(),
        mark_all: mark_all.downgrade(),
        unread: RefCell::new(Vec::new()),
    });
    mark_all.connect_clicked(glib::clone!(
        #[strong]
        hey,
        move |_| {
            let sgids = hey.unread.borrow().clone();
            // Read now: the list shows that once it's fetched again.
            hey.mark_read(sgids, |hey| hey.load());
        }
    ));
    hey.load();
    page
}

impl HeyPage {
    fn load(self: &Rc<Self>) {
        if let Some(stack) = self.stack.upgrade() {
            stack.set_visible_child_name("loading");
        }
        let client = self.client.clone();
        rt::run(
            async move { client.get::<Readings>("/my/readings.json").await },
            glib::clone!(
                #[strong(rename_to = hey)]
                self,
                move |result| hey.loaded(result)
            ),
        );
    }

    fn loaded(self: &Rc<Self>, result: campfe_core::error::Result<Readings>) {
        let (Some(window), Some(stack), Some(column), Some(mark_all)) = (
            self.window.upgrade(),
            self.stack.upgrade(),
            self.column.upgrade(),
            self.mark_all.upgrade(),
        ) else {
            return;
        };
        while let Some(child) = column.first_child() {
            column.remove(&child);
        }
        let readings = match result {
            Ok(readings) => readings,
            Err(error) => {
                window.report("could not load Hey!", &error);
                stack.set_visible_child_name("empty");
                return;
            }
        };
        *self.unread.borrow_mut() = readings
            .unreads
            .iter()
            .map(|r| r.readable_sgid.clone())
            .filter(|sgid| !sgid.is_empty())
            .collect();
        mark_all.set_visible(!self.unread.borrow().is_empty());
        if readings.unreads.is_empty() && readings.reads.is_empty() {
            stack.set_visible_child_name("empty");
            return;
        }
        if !readings.unreads.is_empty() {
            column.append(&self.section(&gettext("New for you"), &readings.unreads));
        }
        if !readings.reads.is_empty() {
            column.append(&self.section(&gettext("Previously"), &readings.reads));
        }
        stack.set_visible_child_name("list");
    }

    /// Tell Basecamp these are read, then refresh the sidebar's count and
    /// run `done`. Quietly nothing where writes are off.
    fn mark_read(self: &Rc<Self>, sgids: Vec<String>, done: impl FnOnce(&Rc<Self>) + 'static) {
        if sgids.is_empty() || !self.client.writes_allowed() {
            return;
        }
        let client = self.client.clone();
        let body = serde_json::json!({ "readables": sgids });
        rt::run(
            async move { client.put_no_reply("/my/unreads.json", &body).await },
            glib::clone!(
                #[strong(rename_to = hey)]
                self,
                move |result| {
                    let window = hey.window.upgrade();
                    match result {
                        Ok(()) => {
                            hey.unread.borrow_mut().retain(|s| !sgids.contains(s));
                            if let Some(mark_all) = hey.mark_all.upgrade() {
                                mark_all.set_visible(!hey.unread.borrow().is_empty());
                            }
                            if let Some(window) = &window {
                                window.fetch_unread_count(hey.account_id);
                            }
                            done(&hey);
                        }
                        Err(error) => {
                            if let Some(window) = &window {
                                window.report("could not mark Hey! items read", &error);
                            }
                        }
                    }
                }
            ),
        );
    }

    fn section(self: &Rc<Self>, title: &str, readings: &[Reading]) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let heading = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        column.append(&heading);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        for reading in readings {
            list.append(&self.row(reading));
        }
        column.append(&list);
        column
    }
}

/// What a notification is about, in a word, by its section and type.
fn describe(reading: &Reading) -> String {
    match reading.section.as_str() {
        "pings" => gettext("Ping"),
        "chats" => gettext("Chat"),
        "mentions" => gettext("Mention"),
        "bubbles" => gettext("Bubbled up"),
        _ => super::kind_label(&reading.kind),
    }
}

impl HeyPage {
    fn row(self: &Rc<Self>, reading: &Reading) -> adw::ActionRow {
        let title = if reading.title.is_empty() {
            describe(reading)
        } else {
            reading.title.clone()
        };
        let mut parts = vec![describe(reading)];
        if !reading.bucket_name.is_empty() {
            parts.push(reading.bucket_name.clone());
        }
        if !reading.creator.name.is_empty() {
            parts.push(reading.creator.name.clone());
        }
        parts.push(time_label(reading.updated_at.or(reading.created_at)));
        let excerpt = richtext::excerpt(&reading.content_excerpt, 100);
        let subtitle = if excerpt.is_empty() {
            parts.join(" · ")
        } else {
            format!("{}\n{}", parts.join(" · "), excerpt)
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&title))
            .subtitle(glib::markup_escape_text(&subtitle))
            .activatable(true)
            .build();
        row.add_prefix(&avatar(&reading.creator, 32));
        let badge = (reading.unread_count > 0).then(|| {
            let badge = gtk::Label::builder()
                .label(reading.unread_count.to_string())
                .css_classes(["badge"])
                .valign(gtk::Align::Center)
                .build();
            row.add_suffix(&badge);
            badge
        });
        let in_app = AppLink::parse(&reading.app_url).is_some();
        row.add_suffix(&gtk::Image::from_icon_name(if in_app {
            "go-next-symbolic"
        } else {
            "external-link-symbolic"
        }));
        let url = reading.app_url.clone();
        let sgid = reading.readable_sgid.clone();
        row.connect_activated(glib::clone!(
            #[strong(rename_to = hey)]
            self,
            move |row| {
                if !url.is_empty()
                    && let Some(window) = hey.window.upgrade()
                    && !window.open_app_url(&url)
                {
                    window.open_in_browser(&url);
                }
                if !hey.unread.borrow().contains(&sgid) {
                    return;
                }
                let row = row.downgrade();
                let badge = badge.as_ref().map(|b| b.downgrade());
                hey.mark_read(vec![sgid.clone()], move |_| {
                    if let Some(row) = row.upgrade() {
                        row.add_css_class("dim-label");
                    }
                    if let Some(badge) = badge.and_then(|b| b.upgrade()) {
                        badge.set_visible(false);
                    }
                });
            }
        ));
        row
    }
}
