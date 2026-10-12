//! AI summaries of a project (`skali_core::summary`): the project's page --
//! where it stands, what needs you, its topics, its people, and the pull
//! requests and links they posted -- and one person's page. Both show the
//! newest summary from the database at once, then count what's been said
//! since, and offer to summarize again. A run goes on when its page is
//! closed; every open summary page of the project redraws when it ends.

use super::{avatar, shell_with_header, time_label};
use crate::i18n::{format, gettext, plural};
use crate::rt;
use crate::settings as keys;
use crate::window::SkaliWindow;
use adw::prelude::*;
use chrono::{DateTime, Local, Utc};
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{Person, Project};
use skali_core::summary::{self, Backend, Gathered, Link, RunError, Summary, Topic};
use std::cell::RefCell;
use std::rc::Rc;

/// What a page is about: the whole project, or one person in it.
enum Focus {
    Project,
    Person(Person),
}

/// How much was said since the summary shown.
#[derive(Clone, Copy)]
enum Fresh {
    Checking,
    /// The count, and whether the check stopped at its limit.
    Counted(usize, bool),
    Unknown,
}

pub struct SummaryView {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    project: Project,
    focus: Focus,
    content: gtk::Box,
    button: gtk::Button,
    fresh: RefCell<Fresh>,
}

/// The agent the settings name, if summaries are on.
pub fn backend(window: &SkaliWindow) -> Option<Backend> {
    Backend::from_setting(&window.settings().string(keys::SUMMARIZER))
}

fn latest(window: &SkaliWindow, account_id: i64, project_id: i64) -> Option<Summary> {
    match window.db().borrow().latest_summary(account_id, project_id) {
        Ok(summary) => summary,
        Err(error) => {
            tracing::warn!("could not read the summary of project {project_id}: {error}");
            None
        }
    }
}

/// The project's summary page.
pub fn page(window: &SkaliWindow, client: Client, project: Project) -> adw::NavigationPage {
    let title = format(
        &gettext("{project} · Summary"),
        &[("project", &project.name)],
    );
    build(window, client, project, Focus::Project, &title)
}

/// What one person in the project is up to, from its summary.
pub fn person_page(
    window: &SkaliWindow,
    client: Client,
    project: Project,
    person: Person,
) -> adw::NavigationPage {
    let title = person.name.clone();
    build(window, client, project, Focus::Person(person), &title)
}

fn build(
    window: &SkaliWindow,
    client: Client,
    project: Project,
    focus: Focus,
    title: &str,
) -> adw::NavigationPage {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    let (page, header) = shell_with_header(title, &super::scrolled_clamp(&content, 760));
    let button = gtk::Button::builder()
        .css_classes(["suggested-action"])
        .tooltip_text(gettext("Read what's new and update the summary"))
        .child(
            &adw::ButtonContent::builder()
                .icon_name("skali-summary-symbolic")
                .label(gettext("Summarize"))
                .build(),
        )
        .build();
    header.pack_end(&button);
    let view = Rc::new(SummaryView {
        window: window.downgrade(),
        client,
        project,
        focus,
        content,
        button,
        fresh: RefCell::new(Fresh::Unknown),
    });
    view.button.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        view,
        move |_| summarize(&window, view.client.clone(), view.project.clone())
    ));
    window
        .state_mut()
        .summary_views
        .entry(view.project.id)
        .or_default()
        .push(Rc::downgrade(&view));
    // The page holds the view; the registry only a weak reference.
    let held = RefCell::new(Some(view.clone()));
    page.connect_destroy(move |_| {
        held.borrow_mut().take();
    });
    view.reload();
    page
}

impl SummaryView {
    /// Draw from the database, then count what's new.
    fn reload(self: &Rc<Self>) {
        self.render();
        self.check_fresh();
    }

    fn check_fresh(self: &Rc<Self>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let summary = latest(&window, self.client.account_id(), self.project.id);
        let Some(since) = summary.and_then(|s| s.covered_until) else {
            return;
        };
        self.fresh.replace(Fresh::Checking);
        self.render();
        let client = self.client.clone();
        let project = self.project.clone();
        rt::run(
            async move { summary::gather(&client, &project, Some(since), summary::CHECK_LIMIT).await },
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |result| {
                    let Some(window) = view.window.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(gathered) => {
                            keep(&window, &view.client, &gathered);
                            view.fresh
                                .replace(Fresh::Counted(gathered.items.len(), gathered.truncated));
                        }
                        Err(error) => {
                            tracing::warn!(
                                "could not count what's new in project {}: {error}",
                                view.project.id
                            );
                            view.fresh.replace(Fresh::Unknown);
                        }
                    }
                    view.render();
                }
            ),
        );
    }

    fn render(self: &Rc<Self>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let account_id = self.client.account_id();
        let summary = latest(&window, account_id, self.project.id);
        let running = window.state().summarizing.get(&self.project.id).copied();
        let backend = backend(&window);
        self.button
            .set_visible(backend.is_some_and(|b| b.find().is_some()));
        self.button.set_sensitive(running.is_none());
        if let Focus::Person(person) = &self.focus {
            self.content
                .append(&person_header(person, summary.as_ref()));
        }
        self.content
            .append(&self.status(summary.as_ref(), running, backend));
        let Some(summary) = summary else {
            return;
        };
        match &self.focus {
            Focus::Project => self.project_sections(&window, &summary),
            Focus::Person(person) => self.person_sections(&window, &summary, person),
        }
    }

    /// The row on top: when the summary was made and what's new since, or
    /// that one is being made, or why there's none.
    fn status(
        &self,
        summary: Option<&Summary>,
        running: Option<Option<usize>>,
        backend: Option<Backend>,
    ) -> gtk::Widget {
        let row = adw::ActionRow::new();
        let name = backend.map_or("", |b| b.name());
        if let Some(read) = running {
            let spinner = adw::Spinner::builder()
                .width_request(24)
                .height_request(24)
                .build();
            row.add_prefix(&spinner);
            row.set_title(&glib::markup_escape_text(&format(
                &gettext("Summarizing with {agent}…"),
                &[("agent", name)],
            )));
            row.set_subtitle(&glib::markup_escape_text(&match read {
                None => gettext("Reading the chats and message boards."),
                Some(n) => format(
                    &plural(
                        "Reading {n} item. This can take a few minutes; it goes on if you leave this page.",
                        "Reading {n} items. This can take a few minutes; it goes on if you leave this page.",
                        n,
                    ),
                    &[("n", &n.to_string())],
                ),
            }));
        } else if let Some(summary) = summary {
            row.add_prefix(&gtk::Image::from_icon_name("skali-summary-symbolic"));
            row.set_title(&glib::markup_escape_text(&format(
                &gettext("Summarized {when}"),
                &[("when", &ago_label(summary.created_at))],
            )));
            let fresh = match *self.fresh.borrow() {
                Fresh::Checking => gettext("Checking for new messages…"),
                Fresh::Counted(0, _) => gettext("Nothing new since"),
                Fresh::Counted(n, more) => {
                    let n = if more { format!("{n}+") } else { n.to_string() };
                    format(&gettext("{n} new since"), &[("n", &n)])
                }
                Fresh::Unknown => String::new(),
            };
            if let Fresh::Counted(n, _) = *self.fresh.borrow()
                && n > 0
            {
                let badge = gtk::Label::builder()
                    .label(fresh.clone())
                    .valign(gtk::Align::Center)
                    .css_classes(["summary-state", "fresh"])
                    .build();
                row.add_suffix(&badge);
            }
            let source = Backend::from_setting(&summary.backend).map_or("", |b| b.name());
            let mut subtitle = format(
                &plural(
                    "{agent} read {n} item up to {until}",
                    "{agent} read {n} items up to {until}",
                    summary.items,
                ),
                &[
                    ("agent", source),
                    ("n", &summary.items.to_string()),
                    ("until", &time_label(summary.covered_until)),
                ],
            );
            if !fresh.is_empty() {
                subtitle = format!("{subtitle} · {fresh}");
            }
            row.set_subtitle(&glib::markup_escape_text(&subtitle));
        } else {
            row.add_prefix(&gtk::Image::from_icon_name("skali-summary-symbolic"));
            match backend {
                None => {
                    row.set_title(&gettext("AI summaries are off"));
                    row.set_subtitle(&glib::markup_escape_text(&gettext(
                        "Choose Claude Code or Codex in Preferences to summarize this project.",
                    )));
                }
                Some(backend) if backend.find().is_none() => {
                    row.set_title(&glib::markup_escape_text(&format(
                        &gettext("{agent} isn't installed"),
                        &[("agent", backend.name())],
                    )));
                    row.set_subtitle(&glib::markup_escape_text(&format(
                        &gettext("Skali runs “{program}” to summarize. Install it, or choose another in Preferences."),
                        &[("program", backend.setting())],
                    )));
                }
                Some(backend) => {
                    row.set_title(&gettext("Not summarized yet"));
                    row.set_subtitle(&glib::markup_escape_text(&format(
                        &gettext(
                            "{agent} reads the last {days} days of chats and message boards, signed in as you, and writes who's working on what. Nothing is sent until you press Summarize.",
                        ),
                        &[
                            ("agent", backend.name()),
                            ("days", &summary::FIRST_DAYS.to_string()),
                        ],
                    )));
                }
            }
        }
        boxed(&[row.upcast()])
    }

    fn project_sections(self: &Rc<Self>, window: &SkaliWindow, summary: &Summary) {
        let digest = &summary.digest;
        if !digest.overview.is_empty() {
            self.content.append(&section(
                &gettext("Where it stands"),
                &prose(&digest.overview),
            ));
        }
        if !digest.needs_you.is_empty() {
            let rows: Vec<gtk::Widget> = digest
                .needs_you
                .iter()
                .map(|thing| {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(thing))
                        .title_selectable(true)
                        .build();
                    row.add_prefix(&gtk::Image::from_icon_name("mail-mark-important-symbolic"));
                    row.upcast()
                })
                .collect();
            self.content
                .append(&section(&gettext("Needs you"), &boxed(&rows)));
        }
        if !digest.topics.is_empty() {
            let rows: Vec<gtk::Widget> = digest
                .topics
                .iter()
                .map(|topic| topic_row(topic, summary))
                .collect();
            self.content
                .append(&section(&gettext("Topics"), &boxed(&rows)));
        }
        if !digest.people.is_empty() {
            let rows: Vec<gtk::Widget> = digest
                .people
                .iter()
                .map(|about| {
                    let person = summary.person(about.id).cloned().unwrap_or_else(|| Person {
                        id: about.id,
                        name: about.name.clone(),
                        ..Default::default()
                    });
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&person.name))
                        .subtitle(glib::markup_escape_text(&skali_core::richtext::shorten(
                            &about.working_on,
                            160,
                        )))
                        .activatable(true)
                        .build();
                    row.add_prefix(&avatar(&person, 36));
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    let client = self.client.clone();
                    let project = self.project.clone();
                    row.connect_activated(glib::clone!(
                        #[weak]
                        window,
                        move |_| window.open_person_summary(
                            client.clone(),
                            project.clone(),
                            person.clone()
                        )
                    ));
                    row.upcast()
                })
                .collect();
            self.content
                .append(&section(&gettext("People"), &boxed(&rows)));
        }
        let links = summary.sorted_links();
        if !links.is_empty() {
            let rows: Vec<gtk::Widget> = links
                .into_iter()
                .map(|link| link_row(window, link, summary, true))
                .collect();
            self.content
                .append(&section(&gettext("Pull requests and links"), &boxed(&rows)));
        }
    }

    fn person_sections(self: &Rc<Self>, window: &SkaliWindow, summary: &Summary, person: &Person) {
        match summary.about(person.id) {
            Some(about) => {
                for (title, text) in [
                    (gettext("Working on"), &about.working_on),
                    (gettext("Discussing"), &about.discussing),
                    (gettext("Waiting on"), &about.waiting_on),
                ] {
                    if !text.is_empty() {
                        self.content.append(&section(&title, &prose(text)));
                    }
                }
            }
            None => self.content.append(&prose(&format(
                &gettext("The latest summary doesn't mention {name}."),
                &[("name", &person.name)],
            ))),
        }
        let topics: Vec<gtk::Widget> = summary
            .digest
            .topics
            .iter()
            .filter(|topic| topic.people.contains(&person.id))
            .map(|topic| topic_row(topic, summary))
            .collect();
        if !topics.is_empty() {
            self.content
                .append(&section(&gettext("Topics"), &boxed(&topics)));
        }
        let links: Vec<gtk::Widget> = summary
            .sorted_links()
            .into_iter()
            .filter(|link| link.posted_by == person.id)
            .map(|link| link_row(window, link, summary, false))
            .collect();
        if !links.is_empty() {
            self.content
                .append(&section(&gettext("Links they posted"), &boxed(&links)));
        }
        // Their own words, from what Skali has kept (offline).
        let said = window
            .db()
            .borrow()
            .said_by(self.client.account_id(), self.project.id, person.id, 8)
            .unwrap_or_else(|error| {
                tracing::warn!("could not read what {} said: {error}", person.id);
                Vec::new()
            });
        if !said.is_empty() {
            let rows: Vec<gtk::Widget> = said
                .into_iter()
                .map(|hit| {
                    let recording = hit.recording;
                    let place = recording
                        .parent
                        .as_ref()
                        .map(|parent| parent.title.clone())
                        .unwrap_or_else(|| super::kind_label(&recording.kind));
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&skali_core::richtext::shorten(
                            &hit.plain_text_content,
                            200,
                        )))
                        .subtitle(glib::markup_escape_text(&format!(
                            "{place} · {}",
                            time_label(recording.created_at)
                        )))
                        .activatable(!recording.app_url.is_empty())
                        .build();
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
                    row.upcast()
                })
                .collect();
            self.content
                .append(&section(&gettext("Lately"), &boxed(&rows)));
        }
    }
}

/// A big face and the name on top of a person's page.
fn person_header(person: &Person, summary: Option<&Summary>) -> gtk::Widget {
    let person = summary.and_then(|s| s.person(person.id)).unwrap_or(person);
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .build();
    header.append(&avatar(person, 64));
    let name = gtk::Label::builder()
        .label(&person.name)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["title-2"])
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    header.append(&name);
    header.upcast()
}

fn topic_row(topic: &Topic, summary: &Summary) -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&topic.title))
        .subtitle(glib::markup_escape_text(&topic.summary))
        .build();
    let faces = gtk::Box::builder()
        .spacing(2)
        .valign(gtk::Align::Center)
        .build();
    for id in topic.people.iter().take(4) {
        if let Some(person) = summary.person(*id) {
            let face = avatar(person, 22);
            face.set_tooltip_text(Some(&person.name));
            faces.append(&face);
        }
    }
    row.add_suffix(&faces);
    row.add_suffix(&state_pill(&topic.state, &topic_state_label(&topic.state)));
    row.upcast()
}

fn link_row(
    window: &SkaliWindow,
    link: &Link,
    summary: &Summary,
    with_poster: bool,
) -> gtk::Widget {
    let host = url_host(&link.url);
    let title = if link.title.is_empty() {
        link.url.as_str()
    } else {
        link.title.as_str()
    };
    let mut byline = vec![kind_label(&link.kind)];
    if with_poster && let Some(person) = summary.person(link.posted_by) {
        byline.push(person.name.clone());
    }
    if let Some(day) = link.posted_at.get(..10).filter(|d| d.len() == 10) {
        byline.push(day.to_string());
    }
    byline.push(host);
    let mut subtitle = byline.join(" · ");
    if !link.about.is_empty() {
        subtitle = format!("{}\n{subtitle}", link.about);
    }
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .tooltip_text(&link.url)
        .activatable(link.url.starts_with("http"))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(match link.kind.as_str() {
        "pull_request" => "skali-pull-request-symbolic",
        "doc" => "x-office-document-symbolic",
        _ => "insert-link-symbolic",
    }));
    if !link.state.is_empty() && link.state != "unknown" {
        row.add_suffix(&state_pill(&link.state, &link_state_label(&link.state)));
    }
    let url = link.url.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| window.open_in_browser(&url)
    ));
    row.upcast()
}

fn url_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .trim_start_matches("www.")
        .to_string()
}

fn state_pill(state: &str, label: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .css_classes(["summary-state", state])
        .build()
}

fn topic_state_label(state: &str) -> String {
    match state {
        "active" => gettext("Active"),
        "blocked" => gettext("Blocked"),
        "decided" => gettext("Decided"),
        "done" => gettext("Done"),
        other => other.to_string(),
    }
}

fn link_state_label(state: &str) -> String {
    match state {
        "open" => gettext("Open"),
        "draft" => gettext("Draft"),
        "merged" => gettext("Merged"),
        "closed" => gettext("Closed"),
        other => other.to_string(),
    }
}

fn kind_label(kind: &str) -> String {
    match kind {
        "pull_request" => gettext("Pull request"),
        "issue" => gettext("Issue"),
        "commit" => gettext("Commit"),
        "code" => gettext("Code"),
        "doc" => gettext("Document"),
        _ => gettext("Link"),
    }
}

/// A heading and what's under it.
fn section(title: &str, child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title(title).build();
    group.add(child);
    group.upcast()
}

/// Rows in a boxed list.
fn boxed(rows: &[gtk::Widget]) -> gtk::Widget {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for row in rows {
        list.append(row);
    }
    list.upcast()
}

/// A paragraph the agent wrote, on a card.
fn prose(text: &str) -> gtk::Widget {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .focusable(false)
        .css_classes(["summary-prose", "card"])
        .build();
    label.upcast()
}

/// How long ago, for "Summarized …": minutes, hours, yesterday, a date.
fn ago_label(time: DateTime<Utc>) -> String {
    let minutes = (Utc::now() - time).num_minutes().max(0);
    let local = time.with_timezone(&Local);
    if minutes < 1 {
        gettext("just now")
    } else if minutes < 60 {
        let n = minutes as usize;
        format(
            &plural("{n} minute ago", "{n} minutes ago", n),
            &[("n", &n.to_string())],
        )
    } else if minutes < 12 * 60 {
        let n = (minutes / 60) as usize;
        format(
            &plural("{n} hour ago", "{n} hours ago", n),
            &[("n", &n.to_string())],
        )
    } else {
        let clock = local.format("%H:%M").to_string();
        let day = if local.date_naive() == Local::now().date_naive() {
            gettext("today")
        } else if local.date_naive() == Local::now().date_naive() - chrono::Duration::days(1) {
            gettext("yesterday")
        } else {
            time_label(Some(time))
        };
        format(
            &gettext("{day} at {time}"),
            &[("day", &day), ("time", &clock)],
        )
    }
}

/// The line a project's page shows for its summary: when it was made and,
/// once counted, what's new since. None with summaries off.
pub fn project_row(
    window: &SkaliWindow,
    client: &Client,
    project: &Project,
) -> Option<gtk::Widget> {
    backend(window)?;
    let summary = latest(window, client.account_id(), project.id);
    let row = adw::ActionRow::builder()
        .title(gettext("Summary"))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("skali-summary-symbolic"));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let when = summary.as_ref().map(|s| {
        format(
            &gettext("Summarized {when}"),
            &[("when", &ago_label(s.created_at))],
        )
    });
    row.set_subtitle(&glib::markup_escape_text(&when.clone().unwrap_or_else(
        || gettext("Who's working on what, from the chats and message boards"),
    )));
    let client_for_row = client.clone();
    let project_for_row = project.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| window.open_summary(client_for_row.clone(), project_for_row.clone())
    ));
    if let (Some(since), Some(when)) = (summary.and_then(|s| s.covered_until), when) {
        let client = client.clone();
        let project = project.clone();
        rt::run(
            {
                let client = client.clone();
                async move {
                    summary::gather(&client, &project, Some(since), summary::CHECK_LIMIT).await
                }
            },
            glib::clone!(
                #[weak]
                window,
                #[weak]
                row,
                move |result| match result {
                    Ok(gathered) => {
                        keep(&window, &client, &gathered);
                        let n = gathered.items.len();
                        if n > 0 {
                            let n = if gathered.truncated {
                                format!("{n}+")
                            } else {
                                n.to_string()
                            };
                            row.set_subtitle(&glib::markup_escape_text(&format!(
                                "{when} · {}",
                                format(&gettext("{n} new since"), &[("n", &n)])
                            )));
                        }
                    }
                    Err(error) => tracing::warn!("could not count what's new: {error}"),
                }
            ),
        );
    }
    Some(boxed(&[row.upcast()]))
}

/// Keep what a check or a run read for searching offline.
fn keep(window: &SkaliWindow, client: &Client, gathered: &Gathered) {
    window.keep_for_search(client, &gathered.lines);
    window.keep_for_search(client, &gathered.messages);
    window.keep_for_search(client, &gathered.comments);
}

/// Redraw the project's open summary pages.
fn refresh(window: &SkaliWindow, project_id: i64) {
    let views: Vec<Rc<SummaryView>> = {
        let mut state = window.state_mut();
        let Some(views) = state.summary_views.get_mut(&project_id) else {
            return;
        };
        views.retain(|view| view.strong_count() > 0);
        views.iter().filter_map(|view| view.upgrade()).collect()
    };
    for view in views {
        view.reload();
    }
}

/// Read what's new in the project since its last summary and have the
/// agent update the summary. One run per project at a time.
pub fn summarize(window: &SkaliWindow, client: Client, project: Project) {
    let Some(backend) = backend(window) else {
        window.toast(&gettext(
            "Choose Claude Code or Codex in Preferences first.",
        ));
        return;
    };
    if window.state().summarizing.contains_key(&project.id) {
        return;
    }
    let account_id = client.account_id();
    let previous = latest(window, account_id, project.id);
    let since = previous.as_ref().and_then(|p| p.covered_until);
    window.state_mut().summarizing.insert(project.id, None);
    refresh(window, project.id);
    rt::run(
        {
            let client = client.clone();
            let project = project.clone();
            async move { summary::gather(&client, &project, since, summary::MAX_ITEMS).await }
        },
        glib::clone!(
            #[weak]
            window,
            move |result| {
                let gathered = match result {
                    Ok(gathered) => gathered,
                    Err(error) => {
                        finish(&window, project.id);
                        window.report(
                            &format!("could not read project {} to summarize it", project.id),
                            &error,
                        );
                        return;
                    }
                };
                keep(&window, &client, &gathered);
                if gathered.items.is_empty() && previous.is_none() {
                    finish(&window, project.id);
                    window.toast(&format(
                        &gettext("Nothing was said in {project} in the last {days} days."),
                        &[
                            ("project", &project.name),
                            ("days", &summary::FIRST_DAYS.to_string()),
                        ],
                    ));
                    return;
                }
                window
                    .state_mut()
                    .summarizing
                    .insert(project.id, Some(gathered.items.len()));
                refresh(&window, project.id);
                let reader = window.my_person(account_id);
                let prompt =
                    summary::prompt(&project, reader.as_ref(), previous.as_ref(), &gathered);
                let domains = summary::link_domains(&gathered, previous.as_ref());
                tracing::info!(
                    "summarizing project {} with {}: {} items, {} characters, fetching from {:?}",
                    project.id,
                    backend.name(),
                    gathered.items.len(),
                    prompt.len(),
                    domains
                );
                rt::run(
                    summary::run(backend, prompt, domains),
                    glib::clone!(
                        #[weak]
                        window,
                        move |result| {
                            match result {
                                Ok(digest) => {
                                    let summary = Summary::new(
                                        project.id,
                                        backend,
                                        &gathered,
                                        previous.as_ref(),
                                        digest,
                                    );
                                    if let Err(error) =
                                        window.db().borrow_mut().store_summary(account_id, &summary)
                                    {
                                        tracing::warn!(
                                            "could not keep the summary of project {}: {error}",
                                            project.id
                                        );
                                    }
                                    tracing::info!(
                                        "summarized project {}: {} topics, {} people, {} links",
                                        project.id,
                                        summary.digest.topics.len(),
                                        summary.digest.people.len(),
                                        summary.digest.links.len()
                                    );
                                    window.toast(&format(
                                        &gettext("The summary of {project} is ready."),
                                        &[("project", &project.name)],
                                    ));
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        "could not summarize project {}: {error}",
                                        project.id
                                    );
                                    window.toast(&run_failure_message(&error));
                                }
                            }
                            finish(&window, project.id);
                        }
                    ),
                );
            }
        ),
    );
}

fn finish(window: &SkaliWindow, project_id: i64) {
    window.state_mut().summarizing.remove(&project_id);
    refresh(window, project_id);
}

fn run_failure_message(error: &RunError) -> String {
    match error {
        RunError::NotFound(agent) => format(
            &gettext("{agent} isn't installed, or Skali can't find it."),
            &[("agent", agent)],
        ),
        RunError::Failed(reason) => format(
            &gettext("The summary failed: {reason}"),
            &[("reason", &skali_core::richtext::shorten(reason, 200))],
        ),
        RunError::TimedOut => gettext("The summary took too long and was stopped."),
        RunError::Output(_) => gettext("The summary came back in a shape Skali didn't understand."),
        RunError::Io(error) => format(
            &gettext("Could not run the summary: {error}"),
            &[("error", &error.to_string())],
        ),
    }
}
