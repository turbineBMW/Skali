//! Timeline: what's been happening across the account's projects, newest
//! first and by day, from `/reports/progress.json`. A row opens the thing
//! it's about (a comment's row scrolls to the comment); Show more pages
//! back, and the header's refresh starts again from now.

use super::{avatar, day_label, shell_with_header};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use chrono::{Local, NaiveDate};
use gtk::glib;
use skali_core::api::{Client, Page};
use skali_core::error::Result;
use skali_core::links::AppLink;
use skali_core::models::ProgressEvent;
use skali_core::richtext;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const FIRST_PAGE: &str = "/reports/progress.json";

struct TimelinePage {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    stack: gtk::Stack,
    /// One heading and list per day, in order.
    days: gtk::Box,
    more: gtk::Button,
    refresh: gtk::Button,
    next: RefCell<Option<String>>,
    /// The day the last list is for, so a page that goes on with the same
    /// day adds to it.
    last_day: Cell<Option<NaiveDate>>,
    last_list: RefCell<Option<gtk::ListBox>>,
}

pub fn page(window: &SkaliWindow, client: Client) -> adw::NavigationPage {
    let days = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let more = gtk::Button::builder()
        .label(gettext("Show more"))
        .halign(gtk::Align::Center)
        .visible(false)
        .css_classes(["pill"])
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    column.append(&days);
    column.append(&more);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    stack.add_named(
        &super::empty(
            &gettext("Nothing has happened yet."),
            "document-open-recent-symbolic",
        ),
        Some("empty"),
    );
    let (page, header) = shell_with_header(&gettext("Timeline"), &stack);
    let refresh = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(gettext("Refresh"))
        .build();
    header.pack_end(&refresh);
    let timeline = Rc::new(TimelinePage {
        window: window.downgrade(),
        client,
        stack,
        days,
        more: more.clone(),
        refresh: refresh.clone(),
        next: RefCell::new(None),
        last_day: Cell::new(None),
        last_list: RefCell::new(None),
    });
    more.connect_clicked(glib::clone!(
        #[strong]
        timeline,
        move |_| timeline.show_more()
    ));
    refresh.connect_clicked(glib::clone!(
        #[strong]
        timeline,
        move |_| timeline.reload()
    ));
    timeline.reload();
    page
}

impl TimelinePage {
    /// From the top: the newest page, replacing what's shown.
    fn reload(self: &Rc<Self>) {
        self.refresh.set_sensitive(false);
        self.next.replace(None);
        self.fetch(FIRST_PAGE.to_string(), true);
    }

    fn show_more(self: &Rc<Self>) {
        let Some(next) = self.next.take() else {
            return;
        };
        self.more.set_sensitive(false);
        self.fetch(next, false);
    }

    fn fetch(self: &Rc<Self>, path: String, fresh: bool) {
        let client = self.client.clone();
        rt::run(
            async move { client.get_page::<ProgressEvent>(&path).await },
            glib::clone!(
                #[strong(rename_to = timeline)]
                self,
                move |result| timeline.fetched(result, fresh)
            ),
        );
    }

    fn fetched(&self, result: Result<Page<ProgressEvent>>, fresh: bool) {
        self.refresh.set_sensitive(true);
        self.more.set_sensitive(true);
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                if let Some(window) = self.window.upgrade() {
                    window.report("could not load the timeline", &error);
                }
                if fresh {
                    self.stack.set_visible_child_name("empty");
                }
                return;
            }
        };
        if fresh {
            while let Some(child) = self.days.first_child() {
                self.days.remove(&child);
            }
            self.last_day.set(None);
            self.last_list.replace(None);
        }
        for event in &page.items {
            let day = event
                .created_at
                .map(|at| at.with_timezone(&Local).date_naive());
            if self.last_list.borrow().is_none() || day != self.last_day.get() {
                self.start_day(day);
            }
            if let Some(list) = &*self.last_list.borrow() {
                list.append(&self.row(event));
            }
        }
        self.more.set_visible(page.next.is_some());
        self.next.replace(page.next);
        self.stack
            .set_visible_child_name(if self.days.first_child().is_some() {
                "list"
            } else {
                "empty"
            });
    }

    /// A heading for the day and the list its events go in.
    fn start_day(&self, day: Option<NaiveDate>) {
        let heading = gtk::Label::builder()
            .label(day.map(day_label).unwrap_or_default())
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let section = gtk::Box::new(gtk::Orientation::Vertical, 6);
        section.append(&heading);
        section.append(&list);
        self.days.append(&section);
        self.last_day.set(day);
        self.last_list.replace(Some(list));
    }

    /// The thing as the title; who did what, where and when, and the first
    /// words of it, below.
    fn row(&self, event: &ProgressEvent) -> adw::ActionRow {
        let title = if event.target.is_empty() {
            event.action.clone()
        } else {
            event.target.clone()
        };
        let mut meta = Vec::new();
        if !event.target.is_empty() && !event.action.is_empty() {
            meta.push(event.action.clone());
        }
        if !event.bucket.name.is_empty() {
            meta.push(event.bucket.name.clone());
        }
        if let Some(at) = event.created_at {
            meta.push(at.with_timezone(&Local).format("%H:%M").to_string());
        }
        let excerpt = richtext::shorten(&event.summary_excerpt, 140);
        let subtitle = if excerpt.is_empty() || excerpt == title {
            meta.join(" · ")
        } else {
            format!("{}\n{excerpt}", meta.join(" · "))
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&title))
            .subtitle(glib::markup_escape_text(&subtitle))
            .activatable(!event.app_url.is_empty())
            .build();
        row.add_prefix(&avatar(&event.creator, 32));
        if !event.app_url.is_empty() {
            let in_app = AppLink::parse(&event.app_url).is_some();
            row.add_suffix(&gtk::Image::from_icon_name(if in_app {
                "go-next-symbolic"
            } else {
                "adw-external-link-symbolic"
            }));
        }
        let url = event.app_url.clone();
        let window = self.window.clone();
        row.connect_activated(move |_| {
            if let Some(window) = window.upgrade()
                && !window.open_app_url(&url)
            {
                window.open_in_browser(&url);
            }
        });
        row
    }
}
