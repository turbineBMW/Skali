//! Keeping up with Basecamp while the window is open: one live stream per
//! account wakes the pages that are showing what changed, and a poll
//! every minute from the stored position repairs anything the stream
//! missed. Events are pointers; the recording is fetched before it's shown.

use super::CampfeWindow;
use crate::pages;
use crate::pages::chat::ChatView;
use crate::rt;
use adw::subclass::prelude::*;
use campfe_core::api::Client;
use campfe_core::feed::{self, Event, FeedMessage};
use campfe_core::models::{ChatLine, Comment, Recording};
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

/// How often the polling lane runs.
const POLL_EVERY: Duration = Duration::from_secs(60);
/// Hey! is re-counted at most this often, however busy the feed.
const UNREAD_RECOUNT_EVERY: Duration = Duration::from_secs(20);

/// A recording page open now, for live comments.
pub type LiveMessage = std::rc::Weak<pages::recording::RecordingPage>;

impl CampfeWindow {
    /// A chat page is open: lines for it go straight in.
    pub fn watch_chat(&self, view: &Rc<ChatView>) {
        self.state_mut()
            .live_chats
            .insert(view.chat_id(), Rc::downgrade(view));
    }

    pub fn watch_recording(&self, page: &Rc<pages::recording::RecordingPage>) {
        self.state_mut()
            .live_messages
            .insert(page.recording_id(), Rc::downgrade(page));
    }

    /// Start the live stream and the poll timer for an account, once.
    pub fn start_feed(&self, account_id: i64) {
        if !self.state_mut().feeds.insert(account_id) {
            return;
        }
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        let (tx, rx) = async_channel::bounded::<FeedMessage>(64);
        rt::spawn({
            let client = client.clone();
            async move { feed::run_stream(client, feed::LIVE_TYPES, tx).await }
        });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                while let Ok(message) = rx.recv().await {
                    match message {
                        FeedMessage::Connected => tracing::info!("live feed up for {account_id}"),
                        FeedMessage::Event(event) => window.handle_event(account_id, &event),
                        FeedMessage::Disconnected { retry_in } => {
                            tracing::info!(
                                "live feed down for {account_id}, retrying in {retry_in:?}"
                            );
                        }
                        FeedMessage::Rejected(reason) => {
                            tracing::warn!("live feed refused for {account_id}: {reason}");
                            break;
                        }
                    }
                }
            }
        ));
        self.poll_feed(account_id);
        glib::timeout_add_local(
            POLL_EVERY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    window.poll_feed(account_id);
                    glib::ControlFlow::Continue
                }
            ),
        );
    }

    /// The durable lane: from the stored position, or the present.
    fn poll_feed(&self, account_id: i64) {
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        let key = format!("feed-position-{account_id}");
        let position = self.db().borrow().meta(&key).ok().flatten();
        rt::run(
            async move {
                let mut events = Vec::new();
                let mut page = feed::poll(&client, position.as_deref()).await?;
                let mut last_position = page.position.take();
                while let Some(next) = page.next.take() {
                    events.append(&mut page.events);
                    page = client.get::<feed::Poll>(&next).await?;
                    if page.position.is_some() {
                        last_position = page.position.take();
                    }
                }
                events.extend(page.events);
                Ok::<_, campfe_core::error::ApiError>((events, last_position))
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| match result {
                    Ok((events, position)) => {
                        for event in &events {
                            window.handle_event(account_id, event);
                        }
                        if let Some(position) = position
                            && let Err(error) = window.db().borrow().set_meta(&key, &position)
                        {
                            tracing::warn!("could not store the feed position: {error}");
                        }
                    }
                    Err(error) => tracing::warn!("feed poll for {account_id} failed: {error}"),
                }
            ),
        );
    }

    /// Act on one event: refetch what it points at and update the page
    /// showing it, if any; recount Hey! for anything that may address us.
    fn handle_event(&self, account_id: i64, event: &Event) {
        tracing::debug!(
            "event {} {} on {:?}",
            event.id,
            event.event_type,
            event.recording_id
        );
        let Some(client) = self.client_for(account_id) else {
            return;
        };
        let Some(recording_id) = event.recording_id else {
            return;
        };
        match event.event_type.as_str() {
            "chat.line.created" => self.live_chat_line(client, recording_id),
            "comment.created" => self.live_comment(client, recording_id),
            _ => {}
        }
        self.recount_unreads_soon(account_id);
    }

    /// A line was posted somewhere: if that chat is open, fetch and show it.
    fn live_chat_line(&self, client: Client, line_id: i64) {
        // The pointer doesn't say which chat; the generic recording does.
        rt::run(
            {
                let client = client.clone();
                async move {
                    client
                        .get::<Recording>(&format!("/recordings/{line_id}.json"))
                        .await
                }
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    let Ok(recording) = result else {
                        return;
                    };
                    let Some(chat_id) = recording.parent.as_ref().map(|p| p.id) else {
                        return;
                    };
                    let Some(view) = window
                        .state()
                        .live_chats
                        .get(&chat_id)
                        .and_then(std::rc::Weak::upgrade)
                    else {
                        return;
                    };
                    rt::run(
                        async move {
                            client
                                .get::<ChatLine>(&format!("/chats/{chat_id}/lines/{line_id}.json"))
                                .await
                        },
                        move |result| match result {
                            Ok(line) => view.append_live(&line),
                            Err(error) => {
                                tracing::warn!("could not fetch live line {line_id}: {error}")
                            }
                        },
                    );
                }
            ),
        );
    }

    /// A comment was posted: if its parent is open, fetch and show it.
    fn live_comment(&self, client: Client, comment_id: i64) {
        rt::run(
            async move {
                client
                    .get::<Comment>(&format!("/comments/{comment_id}.json"))
                    .await
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    let Ok(comment) = result else {
                        return;
                    };
                    let Some(parent_id) = comment.recording.parent.as_ref().map(|p| p.id) else {
                        return;
                    };
                    let target = window
                        .state()
                        .live_messages
                        .get(&parent_id)
                        .and_then(std::rc::Weak::upgrade);
                    if let Some(page) = target {
                        page.add_comment(comment);
                    }
                }
            ),
        );
    }

    /// Re-count Hey!'s unreads, coalescing a burst of events into one call.
    fn recount_unreads_soon(&self, account_id: i64) {
        let imp = self.imp();
        if imp.recount_pending.borrow().contains(&account_id) {
            return;
        }
        imp.recount_pending.borrow_mut().insert(account_id);
        glib::timeout_add_local_once(
            UNREAD_RECOUNT_EVERY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window
                        .imp()
                        .recount_pending
                        .borrow_mut()
                        .remove(&account_id);
                    window.fetch_unread_count(account_id);
                }
            ),
        );
    }
}
