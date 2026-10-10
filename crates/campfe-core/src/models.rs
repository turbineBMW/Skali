//! Basecamp's JSON, as far as Campfe reads it. Every struct ignores fields it
//! doesn't name, since the API adds some without a version change.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// `#[serde(default)]` covers a missing field, not a `null` one; Basecamp
/// sends both for the same field at different times.
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// What Launchpad says about a token: who it belongs to and which Basecamp
/// accounts it opens.
#[derive(Clone, Debug, Deserialize)]
pub struct Authorization {
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    pub identity: Identity,
    #[serde(default)]
    pub accounts: Vec<LaunchpadAccount>,
}

/// The signed-in person as Launchpad knows them. Not a Basecamp person:
/// those are per account, under `/people`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Identity {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub first_name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub last_name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub email_address: String,
}

impl Identity {
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
            .trim()
            .to_string()
    }
}

/// One product account the token opens. Only `bc3` (Basecamp) is ours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LaunchpadAccount {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub product: String,
    /// The API base URL for this account: `https://3.basecampapi.com/<id>`.
    #[serde(default, deserialize_with = "null_default")]
    pub href: String,
    /// Where the web app lives: `https://3.basecamp.com/<id>`.
    #[serde(default, deserialize_with = "null_default")]
    pub app_href: String,
}

impl LaunchpadAccount {
    pub fn is_basecamp(&self) -> bool {
        self.product == "bc3"
    }
}

/// A Basecamp account Campfe is signed in to, as the cache keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub href: String,
    pub app_href: String,
    /// The Launchpad identity whose token opens it; the keyring entry's key.
    pub identity_id: i64,
    pub identity_name: String,
    pub identity_email: String,
}

/// An id that is usually a number but may be a word: a notification's
/// creator can be `"onboarding"` (Basecamp itself). Words read as 0.
fn lenient_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Number(i64),
        Text(String),
        Nothing(()),
    }
    Ok(match Id::deserialize(deserializer)? {
        Id::Number(id) => id,
        Id::Text(text) => text.parse().unwrap_or(0),
        Id::Nothing(()) => 0,
    })
}

/// A person in an account.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Person {
    #[serde(default, deserialize_with = "lenient_id")]
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub email_address: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub avatar_url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub admin: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub owner: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub client: bool,
    /// An open set: User, Agent, Integration, Outsider, Client, Tombstone...
    #[serde(default, deserialize_with = "null_default")]
    pub personable_type: String,
}

/// A tool on a project's dock: a message board, a chat, a to-do set...
/// A project can have several of one kind.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DockTool {
    pub id: i64,
    /// The kind: `message_board`, `todoset`, `vault`, `schedule`, `chat`,
    /// `questionnaire`, `inbox`, `kanban_board`.
    pub name: String,
    /// What the project calls it ("Chat", "Alt Chat").
    #[serde(default, deserialize_with = "null_default")]
    pub title: String,
    #[serde(default, deserialize_with = "null_default")]
    pub enabled: bool,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "null_default")]
    pub url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Project {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub purpose: String,
    #[serde(default, deserialize_with = "null_default")]
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, deserialize_with = "null_default")]
    pub bookmarked: bool,
    #[serde(default)]
    pub starred: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
    #[serde(default)]
    pub dock: Vec<DockTool>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            description: None,
            purpose: String::new(),
            status: String::new(),
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
            bookmarked: false,
            starred: false,
            url: String::new(),
            app_url: String::new(),
            dock: Vec::new(),
        }
    }
}

impl Project {
    /// The dock as shown: enabled tools in their order.
    pub fn enabled_tools(&self) -> impl Iterator<Item = &DockTool> {
        self.dock.iter().filter(|tool| tool.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_parses_with_unknown_fields_and_dock() {
        let json = r#"{
            "id": 1, "name": "Example Project", "status": "active", "purpose": "topic",
            "created_at": "2026-09-01T10:00:00.000Z", "updated_at": "2026-10-09T10:00:00.000Z",
            "something_new": 42,
            "dock": [
              {"id": 10, "name": "chat", "title": "Chat", "enabled": true, "position": 1, "url": "u", "app_url": "a"},
              {"id": 11, "name": "chat", "title": "Alt Chat", "enabled": true},
              {"id": 12, "name": "schedule", "title": "Schedule", "enabled": false}
            ]
        }"#;
        let project: Project = serde_json::from_str(json).unwrap();
        let titles: Vec<_> = project.enabled_tools().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Chat", "Alt Chat"]);
    }

    #[test]
    fn todo_update_keeps_what_the_editor_leaves_alone() {
        let json = r#"{"id": 9, "content": "Old", "description": "<div>notes</div>",
            "starts_on": "2026-10-01", "due_on": "2026-10-05",
            "assignees": [{"id": 1, "name": "A"}],
            "completion_subscribers": [{"id": 2, "name": "B"}, {"id": 3, "name": "C"}]}"#;
        let todo: Todo = serde_json::from_str(json).unwrap();
        let date = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();

        let body = todo.update_body("New", &[1], Some(date("2026-10-08")));
        assert_eq!(body["content"], "New");
        assert_eq!(body["description"], "<div>notes</div>");
        assert_eq!(body["completion_subscriber_ids"], serde_json::json!([2, 3]));
        assert_eq!(body["starts_on"], "2026-10-01");
        assert_eq!(body["due_on"], "2026-10-08");
        assert_eq!(body["notify"], false);

        // Due before the start, or no due date: the start goes.
        let body = todo.update_body("New", &[1, 4], Some(date("2026-09-30")));
        assert!(body["starts_on"].is_null());
        assert_eq!(body["notify"], true);
        let body = todo.update_body("New", &[], None);
        assert!(body["starts_on"].is_null());
        assert!(body["due_on"].is_null());
        assert_eq!(body["assignee_ids"], serde_json::json!([]));
    }

    #[test]
    fn nulls_read_as_empty() {
        let json = r#"{"identity": {"id": 1, "first_name": null, "last_name": "W", "email_address": null},
            "accounts": [{"id": 2, "name": null, "product": "bc3", "href": "h", "app_href": null}]}"#;
        let auth: Authorization = serde_json::from_str(json).unwrap();
        assert_eq!(auth.identity.full_name(), "W");
        assert_eq!(auth.accounts[0].app_href, "");
    }

    #[test]
    fn authorization_lists_basecamp_accounts() {
        let json = r#"{
            "expires_at": "2026-10-24T00:00:00.000Z",
            "identity": {"id": 7, "first_name": "Brandon", "last_name": "Williams", "email_address": "b@example.com"},
            "accounts": [
              {"id": 1000001, "name": "Example Co", "product": "bc3", "href": "https://3.basecampapi.com/1000001", "app_href": "https://3.basecamp.com/1000001"},
              {"id": 1, "name": "Old", "product": "bcx", "href": "https://basecamp.com/1/api/v1"}
            ]
        }"#;
        let auth: Authorization = serde_json::from_str(json).unwrap();
        assert_eq!(auth.identity.full_name(), "Brandon Williams");
        let basecamp: Vec<_> = auth.accounts.iter().filter(|a| a.is_basecamp()).collect();
        assert_eq!(basecamp.len(), 1);
        assert_eq!(basecamp[0].id, 1000001);
    }
}

// ---------------------------------------------------------------------------
// Recordings: everything inside a project shares these fields.

/// The project (or, for a ping, the circle) a recording lives in.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Bucket {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    /// `Project`, or something else for a ping's circle.
    #[serde(default, deserialize_with = "null_default", rename = "type")]
    pub kind: String,
}

/// What a recording sits in: a message's board, a to-do's list.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Parent {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub title: String,
    #[serde(default, deserialize_with = "null_default", rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "null_default")]
    pub url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
}

/// The fields every recording carries. Flattened into each kind.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Recording {
    pub id: i64,
    /// `active`, `archived` or `trashed`.
    #[serde(default, deserialize_with = "null_default")]
    pub status: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "null_default")]
    pub title: String,
    /// The wire type: `Message`, `Todo`, `Chat::Lines::RichText`...
    #[serde(default, deserialize_with = "null_default", rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "null_default")]
    pub url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
    #[serde(default)]
    pub bucket: Bucket,
    #[serde(default)]
    pub creator: Person,
    #[serde(default)]
    pub parent: Option<Parent>,
    #[serde(default)]
    pub comments_count: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub comments_url: String,
    #[serde(default)]
    pub boosts_count: i64,
}

/// The reply to an upload to `/attachments.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct UploadedAttachment {
    pub attachable_sgid: String,
}

/// A file inside rich text, listed beside it (`content_attachments`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Attachment {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub sgid: String,
    #[serde(default, deserialize_with = "null_default")]
    pub filename: String,
    #[serde(default, deserialize_with = "null_default")]
    pub content_type: String,
    #[serde(default)]
    pub byte_size: i64,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
    #[serde(default, deserialize_with = "null_default")]
    pub download_url: String,
    #[serde(default)]
    pub previewable: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub preview_url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub thumbnail_url: String,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/")
    }
}

/// A Campfire: a project's chat, or a ping (whose bucket isn't a Project).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Campfire {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub topic: String,
    #[serde(default, deserialize_with = "null_default")]
    pub lines_url: String,
}

impl Campfire {
    /// A ping is a Campfire outside any project.
    pub fn is_ping(&self) -> bool {
        self.recording.bucket.kind != "Project"
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct ChatLine {
    #[serde(flatten)]
    pub recording: Recording,
    /// Plain text, or HTML for `Chat::Lines::RichText`.
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
}

impl ChatLine {
    pub fn is_rich_text(&self) -> bool {
        self.recording.kind.ends_with("RichText")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct MessageBoard {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default)]
    pub messages_count: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub messages_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Message {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub subject: String,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    #[serde(default)]
    pub content_attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Comment {
    #[serde(flatten)]
    pub recording: Recording,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    #[serde(default)]
    pub content_attachments: Vec<Attachment>,
}

/// The to-do set: a project's one container of to-do lists.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Todoset {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    #[serde(default)]
    pub todos_count: i64,
    #[serde(default)]
    pub todolists_count: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub todolists_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Todolist {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default)]
    pub completed: bool,
    /// "3/7"
    #[serde(default, deserialize_with = "null_default")]
    pub completed_ratio: String,
    #[serde(default, deserialize_with = "null_default")]
    pub todos_url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub groups_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Todo {
    #[serde(flatten)]
    pub recording: Recording,
    /// The to-do's text, plain.
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub starts_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub due_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub assignees: Vec<Person>,
    #[serde(default)]
    pub completion_subscribers: Vec<Person>,
    #[serde(default)]
    pub subtasks_count: i64,
    #[serde(default)]
    pub subtasks_completed_count: i64,
}

impl Todo {
    /// The body of `PUT /todos/<id>.json` changing what the editor edits.
    /// Basecamp clears whatever an update leaves out, so everything else
    /// is sent as it is; a start date the new due date would leave after
    /// it (or without one) goes. Assignees who weren't before are notified.
    pub fn update_body(
        &self,
        content: &str,
        assignee_ids: &[i64],
        due_on: Option<chrono::NaiveDate>,
    ) -> serde_json::Value {
        let starts_on = match (self.starts_on, due_on) {
            (Some(start), Some(due)) if start <= due => Some(start),
            _ => None,
        };
        let notify = assignee_ids
            .iter()
            .any(|id| !self.assignees.iter().any(|p| p.id == *id));
        serde_json::json!({
            "content": content,
            "description": self.description,
            "assignee_ids": assignee_ids,
            "completion_subscriber_ids": self
                .completion_subscribers
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            "due_on": due_on,
            "starts_on": starts_on,
            "notify": notify,
        })
    }
}

/// One entry of Hey!: something unread, or read, for the signed-in person.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Reading {
    pub id: i64,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    /// `inbox`, `chats`, `pings`, `bubbles`, `mentions`.
    #[serde(default, deserialize_with = "null_default")]
    pub section: String,
    #[serde(default)]
    pub unread_count: i64,
    #[serde(default)]
    pub unread_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub read_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "null_default")]
    pub readable_sgid: String,
    #[serde(default, deserialize_with = "null_default")]
    pub readable_identifier: String,
    #[serde(default, deserialize_with = "null_default")]
    pub title: String,
    /// `Message`, `Todo`, `Chat::Transcript`, `Onboarding`...
    #[serde(default, deserialize_with = "null_default", rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "null_default")]
    pub icon_type: String,
    #[serde(default, deserialize_with = "null_default")]
    pub bucket_name: String,
    #[serde(default)]
    pub creator: Person,
    #[serde(default, deserialize_with = "null_default")]
    pub content_excerpt: String,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub unread_url: String,
    #[serde(default)]
    pub subscribed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Readings {
    #[serde(default)]
    pub unreads: Vec<Reading>,
    #[serde(default)]
    pub reads: Vec<Reading>,
    #[serde(default)]
    pub bubble_ups: Vec<Reading>,
}

/// A to-do or card assigned to the signed-in person, as My Assignments
/// lists them: a projection, not the full recording.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Assignment {
    pub id: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    #[serde(default)]
    pub starts_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub due_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub bucket: Bucket,
    #[serde(default)]
    pub completed: bool,
    /// `Todo` or `Kanban::Card`.
    #[serde(default, deserialize_with = "null_default", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub assignees: Vec<Person>,
    #[serde(default)]
    pub comments_count: i64,
    #[serde(default)]
    pub has_description: bool,
    #[serde(default)]
    pub parent: Option<Parent>,
    /// A card's steps.
    #[serde(default)]
    pub children: Vec<Assignment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Assignments {
    /// "Up Next", in the person's order.
    #[serde(default)]
    pub priorities: Vec<Assignment>,
    #[serde(default)]
    pub non_priorities: Vec<Assignment>,
}

#[cfg(test)]
mod recording_tests {
    use super::*;

    #[test]
    fn chat_line_flattens_the_recording() {
        let json = r#"{"id": 1069479442, "status": "active", "type": "Chat::Lines::RichText",
            "created_at": "2026-03-12T16:43:00.000Z", "parent": {"id": 1, "type": "Chat::Transcript"},
            "bucket": {"id": 2, "name": "Newsroom", "type": "Project"},
            "creator": {"id": 3, "name": "Victor Cooper", "personable_type": "User", "tagline": null},
            "content": "<div>Words</div>"}"#;
        let line: ChatLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.recording.id, 1069479442);
        assert_eq!(line.recording.creator.name, "Victor Cooper");
        assert_eq!(line.recording.bucket.kind, "Project");
        assert!(line.is_rich_text());
        assert_eq!(line.recording.parent.unwrap().kind, "Chat::Transcript");
    }

    #[test]
    fn a_readings_creator_may_be_a_word() {
        let json = r#"{"id": 5, "section": "inbox", "type": "Onboarding",
            "creator": {"id": "onboarding", "name": "Basecamp", "personable_type": "LocalPerson"}}"#;
        let reading: Reading = serde_json::from_str(json).unwrap();
        assert_eq!(reading.creator.id, 0);
        assert_eq!(reading.creator.name, "Basecamp");
        let numeric: Person = serde_json::from_str(r#"{"id": "42", "name": "x"}"#).unwrap();
        assert_eq!(numeric.id, 42);
    }

    #[test]
    fn todo_dates_and_assignees() {
        let json = r#"{"id": 1, "content": "Ship it", "completed": false, "due_on": "2026-10-31",
            "starts_on": null, "assignees": [{"id": 9, "name": "A"}], "bucket": {"id": 2}}"#;
        let todo: Todo = serde_json::from_str(json).unwrap();
        assert_eq!(todo.due_on.unwrap().to_string(), "2026-10-31");
        assert_eq!(todo.assignees[0].name, "A");
        assert_eq!(todo.recording.creator, Person::default());
    }
}

// ---------------------------------------------------------------------------
// Card tables and Docs & Files.

/// A card table: its columns ("lists"), each with its cards.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct CardTable {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default)]
    pub lists: Vec<CardColumn>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct CardColumn {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub cards_count: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub cards_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Card {
    #[serde(flatten)]
    pub recording: Recording,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    #[serde(default)]
    pub content_attachments: Vec<Attachment>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub due_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub assignees: Vec<Person>,
    #[serde(default)]
    pub steps: Vec<CardStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct CardStep {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub due_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub assignees: Vec<Person>,
}

/// A vault: Docs & Files itself, or a folder in it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Vault {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default)]
    pub documents_count: i64,
    #[serde(default)]
    pub uploads_count: i64,
    #[serde(default)]
    pub vaults_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Document {
    #[serde(flatten)]
    pub recording: Recording,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    #[serde(default)]
    pub content_attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Upload {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default, deserialize_with = "null_default")]
    pub content_type: String,
    #[serde(default)]
    pub byte_size: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub filename: String,
    #[serde(default, deserialize_with = "null_default")]
    pub download_url: String,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
}

impl Upload {
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/")
    }

    /// "4.9 kB", "2.3 MB".
    pub fn size_label(&self) -> String {
        let bytes = self.byte_size as f64;
        if bytes < 1000.0 {
            format!("{} B", self.byte_size)
        } else if bytes < 1_000_000.0 {
            format!("{:.1} kB", bytes / 1000.0)
        } else if bytes < 1_000_000_000.0 {
            format!("{:.1} MB", bytes / 1_000_000.0)
        } else {
            format!("{:.1} GB", bytes / 1_000_000_000.0)
        }
    }
}

#[cfg(test)]
mod vault_tests {
    use super::*;

    #[test]
    fn a_card_table_carries_its_columns() {
        let json = r#"{"id": 1, "title": "Card Table", "lists": [
            {"id": 2, "title": "Triage", "color": null, "cards_count": 3, "cards_url": "u"},
            {"id": 3, "title": "Done", "color": "green", "cards_count": 0}]}"#;
        let table: CardTable = serde_json::from_str(json).unwrap();
        assert_eq!(table.lists.len(), 2);
        assert_eq!(table.lists[1].color.as_deref(), Some("green"));
    }

    #[test]
    fn upload_sizes_read_well() {
        let upload = |byte_size| Upload {
            byte_size,
            ..Default::default()
        };
        assert_eq!(upload(512).size_label(), "512 B");
        assert_eq!(upload(4887).size_label(), "4.9 kB");
        assert_eq!(upload(2_300_000).size_label(), "2.3 MB");
    }
}

/// When a schedule entry starts or ends: a bare date for an all-day entry
/// (`2026-10-20`), a moment for the rest (`2026-10-13T10:00:00.000-04:00`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Day(chrono::NaiveDate),
    At(DateTime<Utc>),
}

impl When {
    /// The day it falls on in `tz`.
    pub fn date_in<Tz: chrono::TimeZone>(&self, tz: &Tz) -> chrono::NaiveDate {
        match self {
            When::Day(day) => *day,
            When::At(at) => at.with_timezone(tz).date_naive(),
        }
    }
}

impl<'de> Deserialize<'de> for When {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if let Ok(day) = chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d") {
            return Ok(When::Day(day));
        }
        DateTime::parse_from_rfc3339(&text)
            .map(|at| When::At(at.with_timezone(&Utc)))
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for When {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            When::Day(day) => serializer.serialize_str(&day.format("%Y-%m-%d").to_string()),
            When::At(at) => serializer.serialize_str(&at.to_rfc3339()),
        }
    }
}

/// A project's schedule (the dock's `schedule` tool).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Schedule {
    #[serde(flatten)]
    pub recording: Recording,
    /// Whether to-dos and cards due show on it too.
    #[serde(default)]
    pub include_due_assignments: bool,
}

/// An event on a schedule, or one occurrence of a recurring one. The
/// upcoming report sends a slimmer one (no description) than the entry's
/// own endpoint.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct ScheduleEntry {
    #[serde(flatten)]
    pub recording: Recording,
    #[serde(default, deserialize_with = "null_default")]
    pub summary: String,
    /// Rich text (HTML).
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default)]
    pub description_attachments: Vec<Attachment>,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub starts_at: Option<When>,
    #[serde(default)]
    pub ends_at: Option<When>,
    #[serde(default)]
    pub participants: Vec<Person>,
    /// A video-call link, say.
    #[serde(default, deserialize_with = "null_default")]
    pub join_url: String,
    /// Set in the upcoming report.
    #[serde(default)]
    pub recurring: bool,
    /// Set on a recurring entry's own record.
    #[serde(default)]
    pub recurrence_schedule: Option<Recurrence>,
}

impl ScheduleEntry {
    pub fn is_recurring(&self) -> bool {
        self.recurring || self.recurrence_schedule.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Recurrence {
    /// `every_day`, `every_weekday`, `every_week`, `every_other_week`,
    /// `every_month`, `every_day_of_month`, `every_year`, `custom_week`,
    /// `custom_month`.
    #[serde(default, deserialize_with = "null_default")]
    pub frequency: String,
}

/// A to-do or card due, as the upcoming report lists it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct DueItem {
    pub id: i64,
    /// `todo` or `card`.
    #[serde(default, deserialize_with = "null_default", rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    #[serde(default)]
    pub due_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
    #[serde(default)]
    pub bucket: Bucket,
    #[serde(default)]
    pub assignees: Vec<Person>,
}

/// `GET /reports/schedules/upcoming.json`: everything on every schedule
/// in a window of days, recurring entries expanded into occurrences.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct UpcomingSchedule {
    #[serde(default)]
    pub schedule_entries: Vec<ScheduleEntry>,
    #[serde(default)]
    pub recurring_schedule_entry_occurrences: Vec<ScheduleEntry>,
    #[serde(default)]
    pub assignables: Vec<DueItem>,
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn entries_start_on_a_day_or_at_a_moment() {
        let json = r#"{
          "schedule_entries": [
            {"id": 1, "summary": "Release day", "all_day": true, "recurring": false,
             "starts_at": "2026-10-11", "ends_at": "2026-10-11", "parent": {"id": 9, "title": "Calendar"}}
          ],
          "recurring_schedule_entry_occurrences": [
            {"id": 2, "summary": "Weekly sync", "all_day": false, "recurring": true,
             "starts_at": "2026-10-13T10:00:00.000-04:00", "ends_at": "2026-10-13T10:30:00.000-04:00",
             "url": "https://3.basecampapi.com/1/buckets/2/schedule_entries/2/occurrences/20261013.json"}
          ],
          "assignables": [
            {"id": 3, "type": "card", "content": "Design", "due_on": "2026-10-20", "completed": false,
             "bucket": {"id": 2000001, "name": "My Project"}, "assignees": []}
          ]
        }"#;
        let upcoming: UpcomingSchedule = serde_json::from_str(json).unwrap();
        let day = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let release = &upcoming.schedule_entries[0];
        assert_eq!(release.starts_at, Some(When::Day(day("2026-10-11"))));
        let sync = &upcoming.recurring_schedule_entry_occurrences[0];
        assert!(sync.is_recurring());
        assert_eq!(
            sync.starts_at,
            Some(When::At("2026-10-13T14:00:00Z".parse().unwrap()))
        );
        // 10:00 in New York is still the 13th; in Tokyo it's the 13th, 23:00.
        let tokyo = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(sync.starts_at.unwrap().date_in(&tokyo), day("2026-10-13"));
        let samoa = chrono::FixedOffset::west_opt(11 * 3600).unwrap();
        assert_eq!(sync.starts_at.unwrap().date_in(&samoa), day("2026-10-13"));
        assert_eq!(upcoming.assignables[0].due_on, Some(day("2026-10-20")));
        // Round trip.
        let again: UpcomingSchedule =
            serde_json::from_str(&serde_json::to_string(&upcoming).unwrap()).unwrap();
        assert_eq!(again, upcoming);
    }
}
