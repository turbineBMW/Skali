//! The event feed: a live WebSocket lane (Action Cable, opened with a
//! short-lived ticket) that wakes the app up, and a polling lane that is
//! the durable record. Every event is a thin pointer; the recording is
//! fetched again before anything is shown.
//!
//! The stream runs on the tokio runtime and hands `FeedMessage`s to a
//! channel the GTK side drains. It reconnects on its own with a fresh
//! ticket and a jittered backoff; a rejected subscription stops it.

use crate::api::Client;
use crate::error::{ApiError, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// A wake-up: something happened to a recording in a project.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Event {
    pub id: i64,
    /// `message.created`, `chat.line.created`, `todo.completed`...
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub bucket_id: Option<i64>,
    #[serde(default)]
    pub creator_id: Option<i64>,
    #[serde(default)]
    pub performed_by_id: Option<i64>,
    #[serde(default)]
    pub recording_id: Option<i64>,
    /// Per-type extras (a moved card's columns, a boost's id).
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

/// What the stream reports to the app.
#[derive(Debug)]
pub enum FeedMessage {
    Connected,
    Event(Event),
    /// Lost the socket; reconnecting after `retry_in`.
    Disconnected {
        retry_in: Duration,
    },
    /// The server refused the subscription: no point retrying.
    Rejected(String),
}

/// A page of the polling lane.
#[derive(Debug, Deserialize)]
pub struct Poll {
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub position: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Deserialize)]
struct Ticket {
    url: String,
}

/// The event types the app follows live. Chat is only streamed when asked
/// for by name.
pub const LIVE_TYPES: &str = "chat.line.created,comment.created,message.created,todo.created,todo.completed,todo.assignment_changed,card.created,card.completed,card.moved,boost.created";

/// Mint a ticket and get the exact URL to connect to.
pub async fn stream_url(client: &Client) -> Result<String> {
    let ticket: Ticket = client
        .post("/events/stream_ticket.json", &serde_json::json!({}))
        .await?;
    Ok(ticket.url)
}

/// Poll from a position (`since=now` to start at the present, or a token).
pub async fn poll(client: &Client, position: Option<&str>) -> Result<Poll> {
    let path = match position {
        Some(position) => format!(
            "/events.json?position={}",
            percent_encoding::utf8_percent_encode(position, percent_encoding::NON_ALPHANUMERIC)
        ),
        None => "/events.json?since=now".to_string(),
    };
    client.get(&path).await
}

/// How long to wait for the server to confirm the subscription.
const CONFIRM_DEADLINE: Duration = Duration::from_secs(10);
/// Pings come every 3 s; this long without one and the socket is stale.
const STALE_AFTER: Duration = Duration::from_secs(9);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Keep a live stream open until `tx` has no receiver, reporting through
/// it. Returns only then, or on a rejected subscription.
pub async fn run_stream(client: Client, types: &str, tx: async_channel::Sender<FeedMessage>) {
    let mut backoff = BACKOFF_MIN;
    loop {
        let outcome = connect_once(&client, types, &tx).await;
        if tx.is_closed() {
            return;
        }
        match outcome {
            Ok(()) => backoff = BACKOFF_MIN,
            Err(StreamEnd::Rejected(reason)) => {
                let _ = tx.send(FeedMessage::Rejected(reason)).await;
                return;
            }
            Err(StreamEnd::Lost(reason)) => {
                tracing::info!("event stream lost: {reason}");
            }
        }
        let jitter = Duration::from_millis(rand::random_range(0..1000));
        let retry_in = backoff + jitter;
        let _ = tx.send(FeedMessage::Disconnected { retry_in }).await;
        tokio::time::sleep(retry_in).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

enum StreamEnd {
    /// The socket went away, or never confirmed; try again.
    Lost(String),
    Rejected(String),
}

/// One connection: mint, connect, subscribe, then forward until it drops.
/// `Ok` means it ran and then ended cleanly (a server-asked reconnect).
async fn connect_once(
    client: &Client,
    types: &str,
    tx: &async_channel::Sender<FeedMessage>,
) -> std::result::Result<(), StreamEnd> {
    let url = stream_url(client)
        .await
        .map_err(|error| StreamEnd::Lost(format!("could not mint a ticket: {error}")))?;
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|error| StreamEnd::Lost(format!("could not connect: {error}")))?;
    let (mut sink, mut stream) = socket.split();
    let identifier = serde_json::json!({ "channel": "EventsChannel", "types": types }).to_string();
    let subscribe =
        serde_json::json!({ "command": "subscribe", "identifier": identifier }).to_string();
    sink.send(Message::Text(subscribe.into()))
        .await
        .map_err(|error| StreamEnd::Lost(format!("could not subscribe: {error}")))?;

    let mut confirmed = false;
    let confirm_by = tokio::time::Instant::now() + CONFIRM_DEADLINE;
    loop {
        let wait = if confirmed {
            STALE_AFTER
        } else {
            confirm_by.saturating_duration_since(tokio::time::Instant::now())
        };
        let frame = match tokio::time::timeout(wait, stream.next()).await {
            Ok(Some(Ok(frame))) => frame,
            Ok(Some(Err(error))) => return Err(StreamEnd::Lost(error.to_string())),
            Ok(None) => return Err(StreamEnd::Lost("closed".into())),
            Err(_) if confirmed => return Err(StreamEnd::Lost("no ping in time".into())),
            Err(_) => return Err(StreamEnd::Lost("no confirmation in time".into())),
        };
        let text = match frame {
            Message::Text(text) => text,
            Message::Ping(payload) => {
                let _ = sink.send(Message::Pong(payload)).await;
                continue;
            }
            Message::Close(frame) => {
                return Err(StreamEnd::Lost(format!("closed by server: {frame:?}")));
            }
            _ => continue,
        };
        match parse_frame(&text) {
            Frame::Welcome | Frame::Ping => {}
            Frame::Confirmed => {
                confirmed = true;
                let _ = tx.send(FeedMessage::Connected).await;
            }
            Frame::Rejected => {
                return Err(StreamEnd::Rejected("the subscription was rejected".into()));
            }
            Frame::Disconnect { reason, reconnect } => {
                if reconnect {
                    return Ok(());
                }
                return Err(StreamEnd::Rejected(reason));
            }
            Frame::Event(event) => {
                if tx.send(FeedMessage::Event(event)).await.is_err() {
                    return Ok(());
                }
            }
            Frame::Other => {}
        }
    }
}

#[derive(Debug, PartialEq)]
enum Frame {
    Welcome,
    Ping,
    Confirmed,
    Rejected,
    Disconnect { reason: String, reconnect: bool },
    Event(Event),
    Other,
}

/// Read one Action Cable frame.
fn parse_frame(text: &str) -> Frame {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Frame::Other;
    };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("welcome") => return Frame::Welcome,
        Some("ping") => return Frame::Ping,
        Some("confirm_subscription") => return Frame::Confirmed,
        Some("reject_subscription") => return Frame::Rejected,
        Some("disconnect") => {
            return Frame::Disconnect {
                reason: value
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("disconnected")
                    .to_string(),
                reconnect: value
                    .get("reconnect")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(true),
            };
        }
        _ => {}
    }
    match value.get("message") {
        Some(message) => match serde_json::from_value::<Event>(message.clone()) {
            Ok(event) => Frame::Event(event),
            Err(error) => {
                tracing::debug!("an event frame did not parse ({error}): {message}");
                Frame::Other
            }
        },
        None => Frame::Other,
    }
}

impl From<ApiError> for StreamEnd {
    fn from(error: ApiError) -> Self {
        StreamEnd::Lost(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_told_apart() {
        assert_eq!(parse_frame(r#"{"type":"welcome"}"#), Frame::Welcome);
        assert_eq!(
            parse_frame(r#"{"type":"ping","message":1700000000}"#),
            Frame::Ping
        );
        assert_eq!(
            parse_frame(r#"{"identifier":"{}","type":"confirm_subscription"}"#),
            Frame::Confirmed
        );
        assert_eq!(
            parse_frame(r#"{"type":"disconnect","reason":"unauthorized","reconnect":false}"#),
            Frame::Disconnect {
                reason: "unauthorized".into(),
                reconnect: false
            }
        );
        let event = r#"{"identifier":"{}","message":{"id":1071915468,"kind":"chat_line_created","event_type":"chat.line.created","action":"created","created_at":"2026-07-14T06:10:00.159Z","bucket_id":2085958499,"creator_id":1049715945,"performed_by_id":null,"actor_type":"person","recording_id":1069479766,"visible_to_clients":false}}"#;
        match parse_frame(event) {
            Frame::Event(event) => {
                assert_eq!(event.event_type, "chat.line.created");
                assert_eq!(event.recording_id, Some(1069479766));
                assert_eq!(event.performed_by_id, None);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(parse_frame("not json"), Frame::Other);
    }

    #[test]
    fn a_poll_page_parses() {
        let json = r#"{"events":[{"id":1,"kind":"message_created","action":"created","created_at":"2026-07-14T06:10:00.159Z","event_type":"message.created","bucket_id":2,"creator_id":3,"performed_by_id":null,"recording_id":4}],"position":"aBcD"}"#;
        let poll: Poll = serde_json::from_str(json).unwrap();
        assert_eq!(poll.events.len(), 1);
        assert_eq!(poll.position.as_deref(), Some("aBcD"));
        assert!(poll.next.is_none());
    }
}
