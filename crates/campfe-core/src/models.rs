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
    pub subtasks_count: i64,
    #[serde(default)]
    pub subtasks_completed_count: i64,
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
