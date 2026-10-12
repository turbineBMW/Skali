//! Basecamp's web links (a recording's `app_url`, a Hey! entry's, a link
//! in rich text) read as the things Skali can show itself. Anything else
//! stays a web link.

/// A recording Skali has a page for, by its id. Ids are the same in the
/// web app's paths and the API's flat routes (`/messages/<id>.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppLink {
    /// A Campfire or a ping. A link to one line (`chats/1@2`) opens the chat.
    Chat(i64),
    /// A message; a link to a comment on it (`#__recording_<id>`) too.
    Message(i64),
    Card(i64),
    Document(i64),
    Upload(i64),
    Todo(i64),
    Todolist(i64),
    /// An event; for a recurring one, maybe one occurrence of it, by its
    /// date as the link writes it (`20261013`).
    ScheduleEntry {
        id: i64,
        occurrence: Option<u32>,
    },
}

/// What a `skali:` link in a rendered document asks the page to do. The
/// rich-text view has no JavaScript; its controls are these links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAction {
    /// `skali:boost/<recording>`: boost the body or a comment.
    Boost(i64),
    /// `skali:unboost/<recording>/<boost>`: take your boost back.
    Unboost { recording: i64, boost: i64 },
}

impl PageAction {
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix("skali:")?;
        let mut parts = rest.split('/');
        let action = parts.next()?;
        let mut id = || parts.next()?.parse::<i64>().ok();
        let parsed = match action {
            "boost" => Self::Boost(id()?),
            "unboost" => Self::Unboost {
                recording: id()?,
                boost: id()?,
            },
            _ => return None,
        };
        parts.next().is_none().then_some(parsed)
    }

    pub fn uri(&self) -> String {
        match self {
            Self::Boost(recording) => format!("skali:boost/{recording}"),
            Self::Unboost { recording, boost } => format!("skali:unboost/{recording}/{boost}"),
        }
    }
}

impl AppLink {
    /// `https://3.basecamp.com/<account>/buckets/<project>/<kind>/<id>`,
    /// with any `#fragment` or `?query`, or `None` for a link to anything
    /// else (bulletins, onboarding, the schedule, other sites...).
    pub fn parse(url: &str) -> Option<Self> {
        let segments = basecamp_path(url)?;
        let bucket = segments.iter().position(|s| *s == "buckets")?;
        let rest = segments.get(bucket + 2..)?;
        let id = |s: &str| s.split('@').next()?.parse::<i64>().ok();
        match rest {
            ["chats", chat] => id(chat).map(Self::Chat),
            ["messages", message] => id(message).map(Self::Message),
            ["card_tables", "cards", card] => id(card).map(Self::Card),
            ["documents", document] => id(document).map(Self::Document),
            ["uploads", upload] => id(upload).map(Self::Upload),
            ["todos", todo] => id(todo).map(Self::Todo),
            ["todolists", list] => id(list).map(Self::Todolist),
            ["schedule_entries", entry] => id(entry).map(|id| Self::ScheduleEntry {
                id,
                occurrence: None,
            }),
            ["schedule_entries", entry, "occurrences", date] => {
                let occurrence = Some(date.parse::<u32>().ok()?);
                id(entry).map(|id| Self::ScheduleEntry { id, occurrence })
            }
            _ => None,
        }
    }

    /// The recording the link opens on a page of its own, if it does.
    pub fn recording_id(self) -> Option<i64> {
        match self {
            Self::Message(id)
            | Self::Card(id)
            | Self::Document(id)
            | Self::Upload(id)
            | Self::ScheduleEntry { id, .. } => Some(id),
            Self::Chat(_) | Self::Todo(_) | Self::Todolist(_) => None,
        }
    }
}

/// The account a Basecamp web link belongs to: its path's first segment.
pub fn account_of(url: &str) -> Option<i64> {
    basecamp_path(url)?.first()?.parse().ok()
}

/// The comment a link points at: `...#__recording_<id>`.
pub fn comment_of(url: &str) -> Option<i64> {
    url.split_once('#')?
        .1
        .strip_prefix("__recording_")?
        .parse()
        .ok()
}

/// The chat line a link points at: `.../chats/<chat>@<line>`.
pub fn line_of(url: &str) -> Option<i64> {
    let segments = basecamp_path(url)?;
    let chat = segments.windows(2).find(|pair| pair[0] == "chats")?[1];
    chat.split_once('@')?.1.parse().ok()
}

/// The path segments of an `https` link on a Basecamp web host, without
/// any query or fragment.
fn basecamp_path(url: &str) -> Option<Vec<&str>> {
    let rest = url.strip_prefix("https://")?;
    let rest = rest.split(['#', '?']).next().unwrap_or(rest);
    let (host, path) = rest.split_once('/')?;
    if host != "basecamp.com" && !host.ends_with(".basecamp.com") {
        return None;
    }
    Some(path.split('/').filter(|s| !s.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://3.basecamp.com/1000001/buckets/2000002";

    #[test]
    fn a_chat_link_may_name_a_line() {
        assert_eq!(
            line_of(&format!("{BASE}/chats/7@10392386011")),
            Some(10392386011)
        );
        assert_eq!(line_of(&format!("{BASE}/chats/7")), None);
        assert_eq!(line_of(&format!("{BASE}/messages/7@8")), None);
        assert_eq!(line_of("https://example.com/1/buckets/2/chats/7@8"), None);
    }

    #[test]
    fn links_to_what_skali_shows() {
        let parse = |path: &str| AppLink::parse(&format!("{BASE}/{path}"));
        assert_eq!(parse("chats/10252036780"), Some(AppLink::Chat(10252036780)));
        assert_eq!(
            parse("chats/10252036780@10392386011"),
            Some(AppLink::Chat(10252036780))
        );
        assert_eq!(
            parse("messages/10310327010#__recording_10314180681"),
            Some(AppLink::Message(10310327010))
        );
        assert_eq!(
            parse("card_tables/cards/10320547856"),
            Some(AppLink::Card(10320547856))
        );
        assert_eq!(parse("documents/1"), Some(AppLink::Document(1)));
        assert_eq!(parse("uploads/2?x=1"), Some(AppLink::Upload(2)));
        assert_eq!(parse("todos/3"), Some(AppLink::Todo(3)));
        assert_eq!(parse("todolists/4"), Some(AppLink::Todolist(4)));
        assert_eq!(
            parse("schedule_entries/5"),
            Some(AppLink::ScheduleEntry {
                id: 5,
                occurrence: None
            })
        );
        assert_eq!(
            parse("schedule_entries/10392772631/occurrences/20261013"),
            Some(AppLink::ScheduleEntry {
                id: 10392772631,
                occurrence: Some(20261013)
            })
        );
        // The sandbox's links are on another Basecamp host.
        assert_eq!(
            AppLink::parse("https://app.basecamp.com/1000002/buckets/2000001/uploads/5"),
            Some(AppLink::Upload(5))
        );
    }

    #[test]
    fn other_links_stay_web_links() {
        for url in [
            "https://3.basecamp.com/1000001/bulletins/10387727160",
            "https://3.basecamp.com/1000002/onboarding/messages/9923661869",
            "https://3.basecampapi.com/1000001/my/boosts",
            "https://example.com/1/buckets/2/messages/3",
            "https://notbasecamp.com/1/buckets/2/messages/3",
            "http://3.basecamp.com/1/buckets/2/messages/3",
            &format!("{BASE}/card_tables/10252036781"),
            &format!("{BASE}/schedules/1"),
            &format!("{BASE}/schedule_entries/1/occurrences/next-week"),
            &format!("{BASE}/messages/not-a-number"),
            "",
        ] {
            assert_eq!(AppLink::parse(url), None, "{url}");
        }
    }

    #[test]
    fn account_and_comment_come_from_the_link() {
        let url = format!("{BASE}/messages/10310327010#__recording_10314180681");
        assert_eq!(account_of(&url), Some(1000001));
        assert_eq!(comment_of(&url), Some(10314180681));
        assert_eq!(comment_of(&format!("{BASE}/messages/1")), None);
        assert_eq!(comment_of(&format!("{BASE}/messages/1#top")), None);
        assert_eq!(account_of("https://example.com/1000001/x"), None);
    }

    #[test]
    fn page_actions_round_trip() {
        for action in [
            PageAction::Boost(5),
            PageAction::Unboost {
                recording: 5,
                boost: 9,
            },
        ] {
            assert_eq!(PageAction::parse(&action.uri()), Some(action));
        }
        assert_eq!(PageAction::parse("skali:boost/x"), None);
        assert_eq!(PageAction::parse("skali:boost/5/6"), None);
        assert_eq!(PageAction::parse("skali:unboost/5"), None);
        assert_eq!(PageAction::parse("https://x/boost/5"), None);
    }
}
