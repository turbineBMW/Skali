//! A project's schedule: what's coming up, day by day, a few weeks at a
//! time, recurring events as their occurrences and (when the schedule says
//! so) the project's to-dos and cards due. An event opens to its details
//! and comments.

use super::recording::RecordingDoc;
use super::{avatar, shell};
use crate::i18n::gettext;
use crate::rt;
use crate::window::CampfeWindow;
use adw::prelude::*;
use campfe_core::api::Client;
use campfe_core::models::{
    DockTool, DueItem, Project, Schedule, ScheduleEntry, UpcomingSchedule, When,
};
use campfe_core::richtext;
use campfe_core::schedule::{AgendaItem, agenda};
use chrono::{Datelike, Duration, Local, NaiveDate};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// How far ahead one fetch looks.
const WEEKS: i64 = 8;

/// The page's state. Widgets are held weakly: they own the closures that
/// hold this.
struct SchedulePage {
    window: glib::WeakRef<CampfeWindow>,
    client: Client,
    schedule_id: i64,
    project_id: i64,
    project_name: String,
    /// Whether to show what's due; known after the first fetch.
    include_due: Cell<Option<bool>>,
    /// The last day fetched so far.
    until: Cell<NaiveDate>,
    /// What's on screen, by URL: an event under way at the edge of two
    /// fetches comes back in both.
    shown: RefCell<HashSet<String>>,
    stack: glib::WeakRef<gtk::Stack>,
    days: glib::WeakRef<gtk::Box>,
    more: glib::WeakRef<gtk::Button>,
}

pub fn page(
    window: &CampfeWindow,
    client: Client,
    project: &Project,
    tool: &DockTool,
) -> adw::NavigationPage {
    let days = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let more = gtk::Button::builder()
        .label(gettext("Show later"))
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    column.append(&days);
    column.append(&more);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&column, 800), Some("list"));
    let page = shell(&tool.title, &stack);
    page.set_tag(Some(&format!("tool-{}", tool.id)));

    let today = Local::now().date_naive();
    let schedule = Rc::new(SchedulePage {
        window: window.downgrade(),
        client,
        schedule_id: tool.id,
        project_id: project.id,
        project_name: project.name.clone(),
        include_due: Cell::new(None),
        until: Cell::new(today - Duration::days(1)),
        shown: RefCell::new(HashSet::new()),
        stack: stack.downgrade(),
        days: days.downgrade(),
        more: more.downgrade(),
    });
    more.connect_clicked(glib::clone!(
        #[strong]
        schedule,
        move |more| {
            more.set_sensitive(false);
            schedule.load_more();
        }
    ));
    schedule.load_more();
    page
}

impl SchedulePage {
    /// The next few weeks after what's shown.
    fn load_more(self: &Rc<Self>) {
        let from = self.until.get() + Duration::days(1);
        let to = from + Duration::weeks(WEEKS) - Duration::days(1);
        let client = self.client.clone();
        let schedule_path = (self.include_due.get().is_none())
            .then(|| format!("/schedules/{}.json", self.schedule_id));
        let path = format!(
            "/reports/schedules/upcoming.json?window_starts_on={}&window_ends_on={}",
            from.format("%Y-%m-%d"),
            to.format("%Y-%m-%d")
        );
        rt::run(
            async move {
                let schedule = match schedule_path {
                    Some(path) => Some(client.get::<Schedule>(&path).await?),
                    None => None,
                };
                let upcoming = client.get::<UpcomingSchedule>(&path).await?;
                Ok::<_, campfe_core::error::ApiError>((schedule, upcoming))
            },
            glib::clone!(
                #[strong(rename_to = page)]
                self,
                move |result| page.loaded(from, to, result)
            ),
        );
    }

    fn loaded(
        self: &Rc<Self>,
        from: NaiveDate,
        to: NaiveDate,
        result: campfe_core::error::Result<(Option<Schedule>, UpcomingSchedule)>,
    ) {
        let (Some(window), Some(stack), Some(days), Some(more)) = (
            self.window.upgrade(),
            self.stack.upgrade(),
            self.days.upgrade(),
            self.more.upgrade(),
        ) else {
            return;
        };
        more.set_sensitive(true);
        stack.set_visible_child_name("list");
        let (schedule, upcoming) = match result {
            Ok(answer) => answer,
            Err(error) => {
                window.report(
                    &format!("could not load the schedule of {}", self.project_name),
                    &error,
                );
                return;
            }
        };
        if let Some(schedule) = schedule {
            self.include_due.set(Some(schedule.include_due_assignments));
        }
        self.until.set(to);
        let due_in = self
            .include_due
            .get()
            .unwrap_or(false)
            .then_some(self.project_id);
        let mut added = 0;
        for (day, items) in agenda(&upcoming, self.schedule_id, due_in, from, &Local) {
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();
            for item in items {
                let key = match item {
                    AgendaItem::Entry(entry) => entry.recording.url.clone(),
                    AgendaItem::Due(due) => due.app_url.clone(),
                };
                if !self.shown.borrow_mut().insert(key) {
                    continue;
                }
                list.append(&match item {
                    AgendaItem::Entry(entry) => entry_row(&window, entry),
                    AgendaItem::Due(due) => due_row(&window, due),
                });
            }
            if list.first_child().is_none() {
                continue;
            }
            added += 1;
            let section = gtk::Box::new(gtk::Orientation::Vertical, 6);
            section.append(
                &gtk::Label::builder()
                    .label(day_label(day))
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build(),
            );
            section.append(&list);
            days.append(&section);
        }
        if added == 0 {
            days.append(
                &gtk::Label::builder()
                    .label(crate::i18n::format(
                        &gettext("Nothing scheduled through {date}."),
                        &[("date", &date_label(to))],
                    ))
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );
        }
    }
}

fn entry_row(window: &CampfeWindow, entry: &ScheduleEntry) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&entry.summary))
        .subtitle(glib::markup_escape_text(&when_label(entry)))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("x-office-calendar-symbolic"));
    for person in entry.participants.iter().take(3) {
        let avatar = avatar(person, 24);
        avatar.set_tooltip_text(Some(&person.name));
        row.add_suffix(&avatar);
    }
    if entry.is_recurring() {
        let repeats = gtk::Image::from_icon_name("media-playlist-repeat-symbolic");
        repeats.set_tooltip_text(Some(&gettext("Repeats")));
        row.add_suffix(&repeats);
    }
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let url = entry.recording.app_url.clone();
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

fn due_row(window: &CampfeWindow, due: &DueItem) -> adw::ActionRow {
    let what = match due.kind.as_str() {
        "card" => gettext("Card due"),
        _ => gettext("To-do due"),
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&due.content))
        .subtitle(glib::markup_escape_text(&what))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
    for person in due.assignees.iter().take(3) {
        let avatar = avatar(person, 24);
        avatar.set_tooltip_text(Some(&person.name));
        row.add_suffix(&avatar);
    }
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let url = due.app_url.clone();
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

/// An event's details above its description, and its comments.
pub fn entry_page(
    window: &CampfeWindow,
    client: Client,
    entry: ScheduleEntry,
) -> adw::NavigationPage {
    let mut details = String::new();
    let mut when = String::new();
    if let Some(start) = entry.starts_at {
        when.push_str(&day_label(start.date_in(&Local)));
        when.push_str(" · ");
    }
    when.push_str(&when_label(&entry));
    details.push_str(&format!("<p><strong>{}</strong>", richtext::escape(&when)));
    if entry.is_recurring() {
        let frequency = entry
            .recurrence_schedule
            .as_ref()
            .map(|r| r.frequency.as_str())
            .unwrap_or_default();
        details.push_str(&format!(
            " · {}",
            richtext::escape(&repeats_label(frequency))
        ));
    }
    details.push_str("</p>");
    if !entry.participants.is_empty() {
        let names: Vec<&str> = entry.participants.iter().map(|p| p.name.as_str()).collect();
        details.push_str(&format!(
            "<p>{}</p>",
            richtext::escape(&crate::i18n::format(
                &gettext("With {names}"),
                &[("names", &names.join(", "))],
            ))
        ));
    }
    if !entry.join_url.is_empty() {
        details.push_str(&format!(
            "<p><a href=\"{0}\">{0}</a></p>",
            richtext::escape(&entry.join_url)
        ));
    }
    super::recording::recording_page(
        window,
        client,
        RecordingDoc {
            id: entry.recording.id,
            title: entry.summary.clone(),
            creator: entry.recording.creator.clone(),
            created_at: entry.recording.created_at,
            content: details + &entry.description,
            attachments: entry.description_attachments.clone(),
            comments_count: entry.recording.comments_count,
            extra_html: String::new(),
        },
    )
}

/// When in the day: the times, "All day", or the days of a longer event.
fn when_label(entry: &ScheduleEntry) -> String {
    let Some(start) = entry.starts_at else {
        return String::new();
    };
    let end = entry.ends_at.unwrap_or(start);
    let (start_day, end_day) = (start.date_in(&Local), end.date_in(&Local));
    if entry.all_day {
        return if start_day == end_day {
            gettext("All day")
        } else {
            format!("{} – {}", date_label(start_day), date_label(end_day))
        };
    }
    let time = |when: When| match when {
        When::At(at) => at.with_timezone(&Local).format("%H:%M").to_string(),
        When::Day(_) => String::new(),
    };
    if start == end {
        time(start)
    } else if start_day == end_day {
        format!("{} – {}", time(start), time(end))
    } else {
        format!(
            "{} {} – {} {}",
            date_label(start_day),
            time(start),
            date_label(end_day),
            time(end)
        )
    }
}

/// A day as a heading: Today, Tomorrow, or its date.
fn day_label(day: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if day == today {
        gettext("Today")
    } else if day == today + Duration::days(1) {
        gettext("Tomorrow")
    } else if day.year() == today.year() {
        day.format("%A, %B %-d").to_string()
    } else {
        day.format("%A, %B %-d, %Y").to_string()
    }
}

fn date_label(day: NaiveDate) -> String {
    if day.year() == Local::now().year() {
        day.format("%b %-d").to_string()
    } else {
        day.format("%b %-d, %Y").to_string()
    }
}

fn repeats_label(frequency: &str) -> String {
    match frequency {
        "every_day" => gettext("Repeats every day"),
        "every_weekday" => gettext("Repeats every weekday"),
        "every_week" => gettext("Repeats every week"),
        "every_other_week" => gettext("Repeats every other week"),
        "every_month" | "every_day_of_month" => gettext("Repeats every month"),
        "every_year" => gettext("Repeats every year"),
        _ => gettext("Repeats"),
    }
}
