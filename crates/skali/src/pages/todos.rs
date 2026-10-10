//! To-dos: the project's lists, each opening to its to-dos.

use super::{avatar, shell, shell_with_header, time_label};
use crate::dialogs::todo_editor::{self, Target};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::{DockTool, Project, Todo, Todolist};
use skali_core::richtext;

pub fn set_page(
    window: &SkaliWindow,
    client: Client,
    project: &Project,
    tool: &DockTool,
) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&list, 800), Some("list"));
    stack.add_named(
        &super::empty(&gettext("No to-do lists yet."), "object-select-symbolic"),
        Some("empty"),
    );
    let page = shell(&tool.title, &stack);
    page.set_tag(Some(&format!("tool-{}", tool.id)));

    let path = format!("/todosets/{}/todolists.json", tool.id);
    let project_name = project.name.clone();
    let client_for_rows = client.clone();
    rt::run(
        async move { client.get_all::<Todolist>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            list,
            move |result| match result {
                Ok(lists) => {
                    window.keep_for_search(&client_for_rows, &lists);
                    if lists.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    for todolist in lists {
                        let row = adw::ActionRow::builder()
                            .title(glib::markup_escape_text(&todolist.name))
                            .activatable(true)
                            .build();
                        let subtitle = if todolist.description.is_empty() {
                            todolist.completed_ratio.clone()
                        } else {
                            format!(
                                "{} · {}",
                                todolist.completed_ratio,
                                richtext::excerpt(&todolist.description, 80)
                            )
                        };
                        row.set_subtitle(&glib::markup_escape_text(&subtitle));
                        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                        let client = client_for_rows.clone();
                        row.connect_activated(glib::clone!(
                            #[weak]
                            window,
                            move |_| window.open_todolist(client.clone(), todolist.clone())
                        ));
                        list.append(&row);
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report(
                        &format!("could not load the to-do lists of {project_name}"),
                        &error,
                    );
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}

/// One list: its open to-dos, then the completed ones.
pub fn list_page(window: &SkaliWindow, client: Client, list: Todolist) -> adw::NavigationPage {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    if !list.description.is_empty() {
        let description = gtk::Label::builder()
            .use_markup(true)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        description.set_markup(&richtext::pango_markup(&list.description));
        column.append(&description);
    }
    let open = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let done_heading = gtk::Label::builder()
        .label(gettext("Completed"))
        .xalign(0.0)
        .css_classes(["heading"])
        .margin_top(12)
        .visible(false)
        .build();
    let done = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .visible(false)
        .build();
    let spinner = super::loading();
    column.append(&spinner);
    column.append(&open);
    column.append(&done_heading);
    column.append(&done);

    let path = format!("/todolists/{}/todos.json", list.recording.id);
    let completed_path = format!("/todolists/{}/todos.json?completed=true", list.recording.id);
    let client_for_rows = client.clone();
    let client_for_add = client.clone();
    rt::run(
        async move {
            let open = client.get_all::<Todo>(&path).await?;
            let completed = client.get_all::<Todo>(&completed_path).await?;
            Ok::<_, skali_core::error::ApiError>((open, completed))
        },
        glib::clone!(
            #[weak]
            window,
            #[strong(rename_to = client)]
            client_for_rows,
            #[weak]
            column,
            #[weak]
            spinner,
            #[weak]
            open,
            #[weak]
            done,
            #[weak]
            done_heading,
            move |result| {
                column.remove(&spinner);
                match result {
                    Ok((open_todos, completed)) => {
                        window.keep_for_search(&client, &open_todos);
                        window.keep_for_search(&client, &completed);
                        if open_todos.is_empty() {
                            open.append(&nothing_to_do());
                        }
                        for todo in &open_todos {
                            open.append(&todo_row(&window, &client, todo));
                        }
                        if !completed.is_empty() {
                            for todo in &completed {
                                done.append(&todo_row(&window, &client, todo));
                            }
                            done_heading.set_visible(true);
                            done.set_visible(true);
                        }
                    }
                    Err(error) => window.report("could not load the to-dos", &error),
                }
            }
        ),
    );

    let (page, header) = shell_with_header(&list.name, &super::scrolled_clamp(&column, 800));
    page.set_tag(Some(&format!("todolist-{}", list.recording.id)));
    let client = client_for_add;
    let writes_allowed = client.writes_allowed();
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(if writes_allowed {
            gettext("New to-do")
        } else {
            gettext("Writes to this account are turned off.")
        })
        .sensitive(writes_allowed)
        .build();
    header.pack_end(&add);
    let list_id = list.recording.id;
    let project_id = list.recording.bucket.id;
    add.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        open,
        #[strong]
        client,
        move |_| {
            todo_editor::present(
                &window,
                client.clone(),
                project_id,
                Target::New { list_id },
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    open,
                    #[strong]
                    client,
                    move |todo| {
                        window.keep_for_search(&client, std::slice::from_ref(&todo));
                        if let Some(placeholder) = open
                            .first_child()
                            .filter(|row| row.widget_name() == "nothing-to-do")
                        {
                            open.remove(&placeholder);
                        }
                        open.append(&todo_row(&window, &client, &todo));
                    }
                ),
            );
        }
    ));
    page
}

fn nothing_to_do() -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(gettext("Nothing to do"))
        .build();
    row.set_widget_name("nothing-to-do");
    row
}

fn todo_row(window: &SkaliWindow, client: &Client, todo: &Todo) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&todo.content))
        .activatable(client.writes_allowed())
        .build();
    // Activating it opens the editor; what's saved replaces the row.
    let editing = todo.clone();
    let editor_client = client.clone();
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |row| {
            let row = row.downgrade();
            todo_editor::present(
                &window,
                editor_client.clone(),
                editing.recording.bucket.id,
                Target::Edit(Box::new(editing.clone())),
                glib::clone!(
                    #[weak]
                    window,
                    #[strong]
                    editor_client,
                    move |todo| {
                        window.keep_for_search(&editor_client, std::slice::from_ref(&todo));
                        let Some(row) = row.upgrade() else {
                            return;
                        };
                        let Some(list) = row.parent().and_downcast::<gtk::ListBox>() else {
                            return;
                        };
                        list.insert(&todo_row(&window, &editor_client, &todo), row.index());
                        list.remove(&row);
                    }
                ),
            );
        }
    ));
    let check = gtk::CheckButton::builder()
        .active(todo.completed)
        .sensitive(client.writes_allowed())
        .valign(gtk::Align::Center)
        .build();
    // Ticking completes the to-do; unticking reopens it. The box follows
    // Basecamp's answer, so a failure puts it back.
    let todo_id = todo.recording.id;
    let client = client.clone();
    check.connect_toggled(glib::clone!(
        #[weak]
        window,
        #[weak]
        row,
        move |check| {
            if !check.is_sensitive() {
                return;
            }
            let completing = check.is_active();
            check.set_sensitive(false);
            let client = client.clone();
            let path = format!("/todos/{todo_id}/completion.json");
            rt::run(
                async move {
                    if completing {
                        client.post_empty(&path).await
                    } else {
                        client.delete(&path).await
                    }
                },
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    check,
                    #[weak]
                    row,
                    move |result| {
                        check.set_sensitive(true);
                        match result {
                            Ok(()) => {
                                if completing {
                                    row.add_css_class("dim-label");
                                } else {
                                    row.remove_css_class("dim-label");
                                }
                            }
                            Err(error) => {
                                window.report("could not change the to-do", &error);
                                // Put it back without going round again.
                                check.set_sensitive(false);
                                check.set_active(!completing);
                                check.set_sensitive(true);
                            }
                        }
                    }
                ),
            );
        }
    ));
    row.add_prefix(&check);
    let mut details = Vec::new();
    if let Some(due) = todo.due_on {
        details.push(crate::i18n::format(
            &gettext("Due {date}"),
            &[("date", &due.format("%b %-d").to_string())],
        ));
    }
    if todo.recording.comments_count > 0 {
        details.push(super::count_label(
            todo.recording.comments_count,
            &gettext("comment"),
            &gettext("comments"),
        ));
    }
    if !todo.description.is_empty() {
        details.push(richtext::excerpt(&todo.description, 80));
    }
    if !details.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(&details.join(" · ")));
    }
    if todo.completed {
        row.add_css_class("dim-label");
        let _ = time_label(todo.recording.updated_at);
    }
    for assignee in &todo.assignees {
        let avatar = avatar(assignee, 24);
        avatar.set_tooltip_text(Some(&assignee.name));
        row.add_suffix(&avatar);
    }
    row
}
