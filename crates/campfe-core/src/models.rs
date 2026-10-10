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

/// A person in an account.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Person {
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
    #[serde(default, deserialize_with = "null_default")]
    pub url: String,
    #[serde(default, deserialize_with = "null_default")]
    pub app_url: String,
    #[serde(default)]
    pub dock: Vec<DockTool>,
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
