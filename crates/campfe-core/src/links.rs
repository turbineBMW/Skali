//! Basecamp's web links (a recording's `app_url`, a Hey! entry's) read as
//! the things Campfe can show itself. Anything else stays a web link.

/// A recording Campfe has a page for, by its id. Ids are the same in the
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
}

impl AppLink {
    /// `https://3.basecamp.com/<account>/buckets/<project>/<kind>/<id>`,
    /// with any `#fragment` or `?query`, or `None` for a link to anything
    /// else (bulletins, onboarding, the schedule...).
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.split(['#', '?']).next().unwrap_or(url);
        let path = url
            .split_once("://")
            .map_or(url, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
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
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://3.basecamp.com/1000001/buckets/2000002";

    #[test]
    fn links_to_what_campfe_shows() {
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
    }

    #[test]
    fn other_links_stay_web_links() {
        for url in [
            "https://3.basecamp.com/1000001/bulletins/10387727160",
            "https://3.basecamp.com/1000002/onboarding/messages/9923661869",
            "https://3.basecampapi.com/1000001/my/boosts",
            &format!("{BASE}/card_tables/10252036781"),
            &format!("{BASE}/schedules/1"),
            &format!("{BASE}/messages/not-a-number"),
            "",
        ] {
            assert_eq!(AppLink::parse(url), None, "{url}");
        }
    }
}
