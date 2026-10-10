//! A card table: its columns side by side, each with its cards; a card
//! opens to its body, steps and comments, and can be moved to another
//! column (Done among them) and have its steps ticked.

use super::recording::{RecordingDoc, RecordingPage};
use super::{avatar, shell};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::{gio, glib};
use skali_core::api::Client;
use skali_core::models::{Card, CardColumn, CardStep, CardTable, DockTool, Project};
use skali_core::richtext;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Column widths: the same for every column so the board reads as a grid.
const COLUMN_WIDTH: i32 = 280;

pub fn board_page(
    window: &SkaliWindow,
    client: Client,
    project: &Project,
    tool: &DockTool,
) -> adw::NavigationPage {
    let columns_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .valign(gtk::Align::Start)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .focusable(false)
        .vexpand(true)
        .child(&columns_box)
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&scroller, Some("board"));
    stack.add_named(
        &super::empty(
            &gettext("This card table has no columns."),
            "view-dual-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(&tool.title, &stack);

    let path = format!("/card_tables/{}.json", tool.id);
    load_board(window, &client, &path, &project.name, &stack, &columns_box);
    // Back from a card that may have moved: load again. Unchanged columns
    // cost a 304 each.
    let shown_before = Cell::new(false);
    page.connect_showing(glib::clone!(
        #[weak]
        window,
        #[weak]
        stack,
        #[weak]
        columns_box,
        #[strong(rename_to = project_name)]
        project.name,
        move |_| {
            if shown_before.replace(true) {
                load_board(&window, &client, &path, &project_name, &stack, &columns_box);
            }
        }
    ));
    page
}

/// Fetch the table and lay out its columns, replacing any shown.
fn load_board(
    window: &SkaliWindow,
    client: &Client,
    path: &str,
    project_name: &str,
    stack: &gtk::Stack,
    columns_box: &gtk::Box,
) {
    let client_for_columns = client.clone();
    let client = client.clone();
    let path = path.to_string();
    let project_name = project_name.to_string();
    rt::run(
        async move { client.get::<CardTable>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            columns_box,
            move |result| match result {
                Ok(table) => {
                    if table.lists.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    while let Some(child) = columns_box.first_child() {
                        columns_box.remove(&child);
                    }
                    for column in &table.lists {
                        columns_box.append(&column_widget(
                            &window,
                            client_for_columns.clone(),
                            column,
                        ));
                    }
                    stack.set_visible_child_name("board");
                }
                Err(error) => {
                    window.report(
                        &format!("could not load the card table of {project_name}"),
                        &error,
                    );
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
}

/// One column: its title, count and cards, each fetched for itself.
fn column_widget(window: &SkaliWindow, client: Client, column: &CardColumn) -> gtk::Box {
    let widget = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(COLUMN_WIDTH)
        .css_classes(["card-column"])
        .build();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if let Some(color) = column.color.as_deref().filter(|c| !c.is_empty()) {
        let dot = gtk::Box::builder()
            .width_request(10)
            .height_request(10)
            .valign(gtk::Align::Center)
            .css_classes(["column-dot", &format!("column-{color}")])
            .build();
        header.append(&dot);
    }
    let title = gtk::Label::builder()
        .label(&column.recording.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    header.append(&title);
    let count = gtk::Label::builder()
        .label(column.cards_count.to_string())
        .css_classes(["dim-label", "caption"])
        .build();
    header.append(&count);
    widget.append(&header);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    widget.append(&list);
    if column.cards_count == 0 {
        list.append(
            &adw::ActionRow::builder()
                .title(gettext("No cards"))
                .css_classes(["dim-label"])
                .build(),
        );
        return widget;
    }
    let path = format!("/card_tables/lists/{}/cards.json", column.recording.id);
    let client_for_rows = client.clone();
    rt::run(
        async move { client.get_all::<Card>(&path).await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            list,
            move |result| match result {
                Ok(cards) => {
                    for card in cards {
                        list.append(&card_row(&window, client_for_rows.clone(), card));
                    }
                }
                Err(error) => window.report("could not load the cards", &error),
            }
        ),
    );
    widget
}

fn card_row(window: &SkaliWindow, client: Client, card: Card) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&card.recording.title))
        .activatable(true)
        .build();
    let mut details = Vec::new();
    if let Some(due) = card.due_on {
        details.push(crate::i18n::format(
            &gettext("Due {date}"),
            &[("date", &due.format("%b %-d").to_string())],
        ));
    }
    if !card.steps.is_empty() {
        let done = card.steps.iter().filter(|s| s.completed).count();
        details.push(crate::i18n::format(
            &gettext("{done}/{total} steps"),
            &[
                ("done", &done.to_string()),
                ("total", &card.steps.len().to_string()),
            ],
        ));
    }
    if card.recording.comments_count > 0 {
        details.push(super::count_label(
            card.recording.comments_count,
            &gettext("comment"),
            &gettext("comments"),
        ));
    }
    if !details.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(&details.join(" · ")));
    }
    for assignee in &card.assignees {
        let avatar = avatar(assignee, 24);
        avatar.set_tooltip_text(Some(&assignee.name));
        row.add_suffix(&avatar);
    }
    if card.completed {
        row.add_css_class("dim-label");
    }
    row.connect_activated(glib::clone!(
        #[weak]
        window,
        move |_| window.open_card(client.clone(), card.clone())
    ));
    row
}

/// A card: its body, then its steps as a checklist, then comments. The
/// header moves it to another column or straight to Done, and ticks steps.
pub fn card_page(window: &SkaliWindow, client: Client, card: Card) -> adw::NavigationPage {
    let mut content = card.content.clone();
    if let Some(due) = card.due_on {
        content = format!(
            "<p class=\"due\">{}</p>{content}",
            richtext::escape(&crate::i18n::format(
                &gettext("Due {date}"),
                &[("date", &due.format("%B %-d, %Y").to_string())],
            ))
        );
    }
    let (nav_page, header, page) = super::recording::recording_page_parts(
        window,
        client.clone(),
        RecordingDoc {
            id: card.recording.id,
            title: card.recording.title.clone(),
            creator: card.recording.creator.clone(),
            created_at: card.recording.created_at,
            content,
            attachments: card.content_attachments.clone(),
            comments_count: card.recording.comments_count,
            boosts_count: card.recording.boosts_count,
            extra_html: steps_html(&card.steps),
        },
    );
    let controls = CardControls::new(window, client, &card, page);
    nav_page.insert_action_group("card", Some(&controls.actions));
    header.pack_end(&controls.move_button);
    header.pack_end(&controls.done);
    if !card.steps.is_empty() {
        header.pack_end(&controls.steps_button);
    }
    if let Some(column) = &card.recording.parent {
        controls.fetch_columns(column.id);
    }
    nav_page
}

/// The steps as the document shows them: a checklist, read-only.
fn steps_html(steps: &[CardStep]) -> String {
    if steps.is_empty() {
        return String::new();
    }
    let mut html = format!(
        "<section class=\"steps\"><h2>{}</h2><ul class=\"steps\">",
        richtext::escape(&gettext("Steps"))
    );
    for step in steps {
        html.push_str(&format!(
            "<li class=\"{}\">{} {}</li>",
            if step.completed { "done" } else { "open" },
            if step.completed { "☑" } else { "☐" },
            richtext::escape(&step.recording.title)
        ));
    }
    html.push_str("</ul></section>");
    html
}

/// "2/5": done steps of all.
fn steps_label(steps: &[CardStep]) -> String {
    let done = steps.iter().filter(|s| s.completed).count();
    format!("{done}/{}", steps.len())
}

/// A card page's header controls and what they act on.
struct CardControls {
    window: glib::WeakRef<SkaliWindow>,
    client: Client,
    card_id: i64,
    /// `card.move`: its state is the card's column, its parameter the
    /// column to move to. The move menu shows it as radio items.
    actions: gio::SimpleActionGroup,
    move_action: gio::SimpleAction,
    columns: RefCell<Vec<CardColumn>>,
    move_button: gtk::MenuButton,
    done: gtk::Button,
    steps: RefCell<Vec<CardStep>>,
    steps_button: gtk::MenuButton,
    page: Rc<RecordingPage>,
}

impl CardControls {
    fn new(window: &SkaliWindow, client: Client, card: &Card, page: Rc<RecordingPage>) -> Rc<Self> {
        let writes_allowed = client.writes_allowed();
        let current = card.recording.parent.as_ref().map_or(0, |p| p.id);
        let move_action = gio::SimpleAction::new_stateful(
            "move",
            Some(glib::VariantTy::INT64),
            &current.to_variant(),
        );
        move_action.set_enabled(writes_allowed);
        let actions = gio::SimpleActionGroup::new();
        actions.add_action(&move_action);
        let off = gettext("Writes to this account are turned off.");
        // Hidden until the columns are in.
        // Labelled with the card's column, so it says where the card is.
        let move_button = gtk::MenuButton::builder()
            .tooltip_text(gettext("Move to column"))
            .visible(false)
            .build();
        let done = gtk::Button::builder()
            .icon_name("object-select-symbolic")
            .tooltip_text(if writes_allowed {
                gettext("Move to Done")
            } else {
                off.clone()
            })
            .visible(false)
            .build();
        let steps_list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let steps_button = gtk::MenuButton::builder()
            .label(steps_label(&card.steps))
            .tooltip_text(if writes_allowed {
                gettext("Steps")
            } else {
                off
            })
            .popover(&gtk::Popover::builder().child(&steps_list).build())
            .build();
        let controls = Rc::new(Self {
            window: window.downgrade(),
            client,
            card_id: card.recording.id,
            actions,
            move_action,
            columns: RefCell::new(Vec::new()),
            move_button,
            done,
            steps: RefCell::new(card.steps.clone()),
            steps_button,
            page,
        });
        controls.move_action.connect_activate(glib::clone!(
            #[weak]
            controls,
            move |_, target| {
                if let Some(target) = target.and_then(|t| t.get::<i64>()) {
                    controls.move_to(target);
                }
            }
        ));
        for (index, step) in card.steps.iter().enumerate() {
            steps_list.append(&controls.step_check(index, step, writes_allowed));
        }
        controls
    }

    /// The card's column, its table, and so the table's columns.
    fn fetch_columns(self: &Rc<Self>, column_id: i64) {
        let client = self.client.clone();
        rt::run(
            async move {
                let column = client
                    .get::<CardColumn>(&format!("/card_tables/columns/{column_id}.json"))
                    .await?;
                match column.recording.parent {
                    Some(table) => client
                        .get::<CardTable>(&format!("/card_tables/{}.json", table.id))
                        .await
                        .map(Some),
                    None => Ok(None),
                }
            },
            glib::clone!(
                #[strong(rename_to = controls)]
                self,
                move |result| match result {
                    Ok(Some(table)) => controls.show_columns(table),
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(window) = controls.window.upgrade() {
                            window.report("could not load the card's columns", &error);
                        }
                    }
                }
            ),
        );
    }

    fn show_columns(&self, table: CardTable) {
        let menu = gio::Menu::new();
        for column in &table.lists {
            let item = gio::MenuItem::new(Some(&column.recording.title), None);
            item.set_action_and_target_value(
                Some("card.move"),
                Some(&column.recording.id.to_variant()),
            );
            menu.append_item(&item);
        }
        self.move_button.set_menu_model(Some(&menu));
        self.move_button.set_visible(true);
        if let Some(done) = table.done_column() {
            self.done
                .set_action_target_value(Some(&done.recording.id.to_variant()));
            self.done.set_action_name(Some("card.move"));
        }
        *self.columns.borrow_mut() = table.lists;
        self.update_header();
    }

    fn column_title(&self, column_id: i64) -> String {
        self.columns
            .borrow()
            .iter()
            .find(|c| c.recording.id == column_id)
            .map(|c| c.recording.title.clone())
            .unwrap_or_default()
    }

    fn current_column(&self) -> i64 {
        self.move_action
            .state()
            .and_then(|s| s.get::<i64>())
            .unwrap_or(0)
    }

    /// The move button names the card's column; Done shows while there is
    /// a Done column and the card isn't in it.
    fn update_header(&self) {
        self.move_button
            .set_label(&self.column_title(self.current_column()));
        let done_id = self.done.action_target_value().and_then(|v| v.get::<i64>());
        self.done
            .set_visible(done_id.is_some_and(|id| id != self.current_column()));
    }

    fn move_to(self: &Rc<Self>, column_id: i64) {
        if column_id == self.current_column() {
            return;
        }
        let title = self.column_title(column_id);
        tracing::debug!("moving card {} to column {column_id}", self.card_id);
        self.move_action.set_enabled(false);
        let client = self.client.clone();
        let path = format!("/card_tables/cards/{}/moves.json", self.card_id);
        rt::run(
            async move {
                client
                    .post_no_reply(&path, &serde_json::json!({ "column_id": column_id }))
                    .await
            },
            glib::clone!(
                #[strong(rename_to = controls)]
                self,
                move |result| {
                    controls.move_action.set_enabled(true);
                    let Some(window) = controls.window.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(()) => {
                            controls.move_action.set_state(&column_id.to_variant());
                            controls.update_header();
                            window.toast(&crate::i18n::format(
                                &gettext("Moved to {column}"),
                                &[("column", &title)],
                            ));
                        }
                        Err(error) => window.report(
                            &format!("could not move card {} to {title}", controls.card_id),
                            &error,
                        ),
                    }
                }
            ),
        );
    }

    /// One step's checkbox: ticking completes it, unticking reopens it,
    /// and a failure puts the box back.
    fn step_check(
        self: &Rc<Self>,
        index: usize,
        step: &CardStep,
        writes_allowed: bool,
    ) -> gtk::CheckButton {
        let check = gtk::CheckButton::builder()
            .label(&step.recording.title)
            .active(step.completed)
            .sensitive(writes_allowed)
            .build();
        let step_id = step.recording.id;
        check.connect_toggled(glib::clone!(
            #[weak(rename_to = controls)]
            self,
            move |check| {
                if !check.is_sensitive() {
                    return;
                }
                let completing = check.is_active();
                check.set_sensitive(false);
                let client = controls.client.clone();
                let path = format!("/card_tables/steps/{step_id}/completions.json");
                let completion = if completing { "on" } else { "off" };
                rt::run(
                    async move {
                        client
                            .put_no_reply(&path, &serde_json::json!({ "completion": completion }))
                            .await
                    },
                    glib::clone!(
                        #[strong]
                        controls,
                        #[weak]
                        check,
                        move |result| {
                            check.set_sensitive(true);
                            match result {
                                Ok(()) => {
                                    let html = {
                                        let mut steps = controls.steps.borrow_mut();
                                        if let Some(step) = steps.get_mut(index) {
                                            step.completed = completing;
                                        }
                                        controls.steps_button.set_label(&steps_label(&steps));
                                        steps_html(&steps)
                                    };
                                    controls.page.set_extra_html(html);
                                }
                                Err(error) => {
                                    if let Some(window) = controls.window.upgrade() {
                                        window.report(
                                            &format!("could not change card step {step_id}"),
                                            &error,
                                        );
                                    }
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
        check
    }
}
