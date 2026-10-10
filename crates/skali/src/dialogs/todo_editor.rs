//! Adding a to-do, or changing one: what it is, who it's assigned to, when
//! it's due. A to-do's notes are rich text and are kept as they are.

use crate::i18n::gettext;
use crate::pages::avatar;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use chrono::{Datelike, NaiveDate};
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{Person, Todo};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

pub enum Target {
    /// A new to-do at the end of this list.
    New {
        list_id: i64,
    },
    Edit(Box<Todo>),
}

pub fn present(
    window: &SkaliWindow,
    client: Client,
    project_id: i64,
    target: Target,
    on_saved: impl Fn(Todo) + 'static,
) {
    let (title, save_label, todo) = match &target {
        Target::New { .. } => (gettext("New to-do"), gettext("Add"), None),
        Target::Edit(todo) => (gettext("Edit to-do"), gettext("Save"), Some(todo)),
    };
    let assigned: Rc<RefCell<BTreeSet<i64>>> = Rc::new(RefCell::new(
        todo.map(|t| t.assignees.iter().map(|p| p.id).collect())
            .unwrap_or_default(),
    ));
    let due: Rc<Cell<Option<NaiveDate>>> = Rc::new(Cell::new(todo.and_then(|t| t.due_on)));

    let content = adw::EntryRow::builder()
        .title(gettext("To-do"))
        .text(todo.map(|t| t.content.as_str()).unwrap_or_default())
        .build();
    let what = adw::PreferencesGroup::new();
    what.add(&content);

    let people = adw::PreferencesGroup::builder()
        .title(gettext("Assigned to"))
        .build();
    let people_spinner = adw::ActionRow::builder()
        .child(&crate::pages::loading())
        .build();
    people.add(&people_spinner);

    let when = adw::PreferencesGroup::new();
    let due_row = adw::ActionRow::builder().title(gettext("Due date")).build();
    let calendar = gtk::Calendar::new();
    let pick = gtk::MenuButton::builder()
        .icon_name("x-office-calendar-symbolic")
        .tooltip_text(gettext("Pick a date"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .popover(&gtk::Popover::builder().child(&calendar).build())
        .build();
    let clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text(gettext("No due date"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    due_row.add_suffix(&clear);
    due_row.add_suffix(&pick);
    due_row.set_activatable_widget(Some(&pick));
    when.add(&due_row);
    let show_due = glib::clone!(
        #[weak]
        due_row,
        #[weak]
        clear,
        #[strong]
        due,
        move || {
            match due.get() {
                Some(date) => due_row.set_subtitle(&date.format("%A, %B %-d, %Y").to_string()),
                None => due_row.set_subtitle(&gettext("None")),
            }
            clear.set_visible(due.get().is_some());
        }
    );
    show_due();
    if let Some(date) = due.get()
        && let Ok(day) = glib::DateTime::from_local(
            date.year(),
            date.month() as i32,
            date.day() as i32,
            0,
            0,
            0.0,
        )
    {
        calendar.select_day(&day);
    }
    calendar.connect_day_selected(glib::clone!(
        #[strong]
        due,
        #[strong]
        show_due,
        #[weak]
        pick,
        move |calendar| {
            let day = calendar.date();
            due.set(NaiveDate::from_ymd_opt(
                day.year(),
                day.month() as u32,
                day.day_of_month() as u32,
            ));
            show_due();
            pick.popdown();
        }
    ));
    clear.connect_clicked(glib::clone!(
        #[strong]
        due,
        #[strong]
        show_due,
        move |_| {
            due.set(None);
            show_due();
        }
    ));

    let page = adw::PreferencesPage::new();
    page.add(&what);
    page.add(&people);
    page.add(&when);

    let cancel = gtk::Button::with_label(&gettext("Cancel"));
    let save = gtk::Button::builder()
        .label(&save_label)
        .css_classes(["suggested-action"])
        .sensitive(!content.text().trim().is_empty())
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(&title)
        .content_width(480)
        .content_height(560)
        .child(&toolbar)
        .build();

    content.connect_changed(glib::clone!(
        #[weak]
        save,
        move |content| save.set_sensitive(!content.text().trim().is_empty())
    ));
    content.connect_entry_activated(glib::clone!(
        #[weak]
        save,
        move |_| {
            if save.is_sensitive() {
                save.emit_clicked();
            }
        }
    ));
    cancel.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    // Everyone on the project, ticked if assigned.
    let people_client = client.clone();
    rt::run(
        async move {
            people_client
                .get_all::<Person>(&format!("/projects/{project_id}/people.json"))
                .await
        },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            people,
            #[weak]
            people_spinner,
            #[strong]
            assigned,
            move |result| {
                people.remove(&people_spinner);
                match result {
                    Ok(everyone) => {
                        for person in everyone {
                            people.add(&person_row(&person, &assigned));
                        }
                    }
                    Err(error) => {
                        window.report("could not list the people on the project", &error);
                        people.set_description(Some(&gettext(
                            "Couldn't load the people on this project.",
                        )));
                    }
                }
            }
        ),
    );

    let on_saved = Rc::new(on_saved);
    let target = Rc::new(target);
    save.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        dialog,
        #[weak]
        content,
        #[strong]
        assigned,
        #[strong]
        due,
        #[strong]
        on_saved,
        #[strong]
        target,
        move |save| {
            let text = content.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            let assignee_ids: Vec<i64> = assigned.borrow().iter().copied().collect();
            let (path, body, creating) = match target.as_ref() {
                Target::New { list_id } => (
                    format!("/todolists/{list_id}/todos.json"),
                    serde_json::json!({
                        "content": text,
                        "assignee_ids": assignee_ids,
                        "due_on": due.get(),
                        "notify": !assignee_ids.is_empty(),
                    }),
                    true,
                ),
                Target::Edit(todo) => (
                    format!("/todos/{}.json", todo.recording.id),
                    todo.update_body(&text, &assignee_ids, due.get()),
                    false,
                ),
            };
            save.set_sensitive(false);
            dialog.set_can_close(false);
            let client = client.clone();
            rt::run(
                async move {
                    if creating {
                        client.post::<Todo, _>(&path, &body).await
                    } else {
                        client.put::<Todo, _>(&path, &body).await
                    }
                },
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    dialog,
                    #[weak]
                    save,
                    #[strong]
                    on_saved,
                    move |result| {
                        dialog.set_can_close(true);
                        match result {
                            Ok(todo) => {
                                on_saved(todo);
                                dialog.close();
                            }
                            Err(error) => {
                                save.set_sensitive(true);
                                window.report("could not save the to-do", &error);
                            }
                        }
                    }
                ),
            );
        }
    ));

    dialog.present(Some(window));
    content.grab_focus();
}

fn person_row(person: &Person, assigned: &Rc<RefCell<BTreeSet<i64>>>) -> adw::ActionRow {
    let check = gtk::CheckButton::builder()
        .active(assigned.borrow().contains(&person.id))
        .valign(gtk::Align::Center)
        .build();
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&person.name))
        .activatable_widget(&check)
        .build();
    row.add_prefix(&avatar(person, 28));
    row.add_suffix(&check);
    let id = person.id;
    check.connect_toggled(glib::clone!(
        #[strong]
        assigned,
        move |check| {
            if check.is_active() {
                assigned.borrow_mut().insert(id);
            } else {
                assigned.borrow_mut().remove(&id);
            }
        }
    ));
    row
}
