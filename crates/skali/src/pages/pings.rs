//! Pings: the Campfires outside any project, one per conversation.

use super::{shell, time_label};
use crate::i18n::gettext;
use crate::rt;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::glib;
use skali_core::api::Client;
use skali_core::models::Campfire;

pub fn page(window: &SkaliWindow, client: Client) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&super::loading(), Some("loading"));
    stack.add_named(&super::scrolled_clamp(&list, 800), Some("list"));
    stack.add_named(
        &super::empty(
            &gettext("No pings yet. Pings are direct messages, outside any project."),
            "user-available-symbolic",
        ),
        Some("empty"),
    );
    let page = shell(&gettext("Pings"), &stack);

    let client_for_rows = client.clone();
    rt::run(
        async move { client.get_all::<Campfire>("/chats.json").await },
        glib::clone!(
            #[weak]
            window,
            #[weak]
            stack,
            #[weak]
            list,
            move |result| match result {
                Ok(campfires) => {
                    let mut pings: Vec<Campfire> =
                        campfires.into_iter().filter(Campfire::is_ping).collect();
                    if pings.is_empty() {
                        stack.set_visible_child_name("empty");
                        return;
                    }
                    pings.sort_by_key(|p| std::cmp::Reverse(p.recording.updated_at));
                    for ping in pings {
                        let row = adw::ActionRow::builder()
                            .title(glib::markup_escape_text(&ping.recording.title))
                            .subtitle(glib::markup_escape_text(&time_label(
                                ping.recording.updated_at,
                            )))
                            .activatable(true)
                            .build();
                        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                        let client = client_for_rows.clone();
                        row.connect_activated(glib::clone!(
                            #[weak]
                            window,
                            move |_| window.open_campfire(client.clone(), &ping)
                        ));
                        list.append(&row);
                    }
                    stack.set_visible_child_name("list");
                }
                Err(error) => {
                    window.report("could not load the pings", &error);
                    stack.set_visible_child_name("empty");
                }
            }
        ),
    );
    page
}
