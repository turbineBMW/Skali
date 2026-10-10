//! The SQLite cache: accounts, projects, and the text of every recording
//! Skali has shown, in an FTS5 index for searching offline. One connection,
//! on the main thread. Migrations are append-only, tracked by
//! `PRAGMA user_version`.

use crate::error::Result;
use crate::models::{Account, Project, Recording, SearchResult, Searchable};
use rusqlite::{Connection, params};
use std::path::Path;

const MIGRATIONS: &[&str] = &[
    // 1: accounts and a key-value table for sync state (ETags, positions).
    "CREATE TABLE accounts (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        href TEXT NOT NULL,
        app_href TEXT NOT NULL DEFAULT '',
        identity_id INTEGER NOT NULL,
        identity_name TEXT NOT NULL DEFAULT '',
        identity_email TEXT NOT NULL DEFAULT '',
        position INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    // 2: projects as last listed, whole, so the sidebar fills before the
    // network answers.
    "CREATE TABLE projects (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        position INTEGER NOT NULL DEFAULT 0,
        json TEXT NOT NULL
    );
    CREATE INDEX projects_account ON projects(account_id, position);",
    // 3: the local search index: each recording Skali has shown, its shared
    // fields whole and its words plain, with FTS5 over title and text kept
    // in step by triggers.
    "CREATE TABLE recordings (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        bucket_kind TEXT NOT NULL,
        created_at TEXT,
        title TEXT NOT NULL,
        text TEXT NOT NULL,
        json TEXT NOT NULL
    );
    CREATE INDEX recordings_account ON recordings(account_id, created_at);
    CREATE VIRTUAL TABLE recordings_fts USING fts5(
        title, text, content = 'recordings', content_rowid = 'id',
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE TRIGGER recordings_ai AFTER INSERT ON recordings BEGIN
        INSERT INTO recordings_fts (rowid, title, text) VALUES (new.id, new.title, new.text);
    END;
    CREATE TRIGGER recordings_ad AFTER DELETE ON recordings BEGIN
        INSERT INTO recordings_fts (recordings_fts, rowid, title, text)
        VALUES ('delete', old.id, old.title, old.text);
    END;
    CREATE TRIGGER recordings_au AFTER UPDATE ON recordings BEGIN
        INSERT INTO recordings_fts (recordings_fts, rowid, title, text)
        VALUES ('delete', old.id, old.title, old.text);
        INSERT INTO recordings_fts (rowid, title, text) VALUES (new.id, new.title, new.text);
    END;",
];

/// The most recordings the index keeps per account; the oldest go first.
pub const INDEX_LIMIT: usize = 100_000;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            self.conn.execute_batch(migration)?;
            self.conn
                .pragma_update(None, "user_version", index as i64 + 1)?;
        }
        Ok(())
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, href, app_href, identity_id, identity_name, identity_email
             FROM accounts ORDER BY position, name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Account {
                id: row.get(0)?,
                name: row.get(1)?,
                href: row.get(2)?,
                app_href: row.get(3)?,
                identity_id: row.get(4)?,
                identity_name: row.get(5)?,
                identity_email: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Insert or refresh an account; the id is Basecamp's.
    pub fn upsert_account(&self, account: &Account) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (id, name, href, app_href, identity_id, identity_name, identity_email)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, href = excluded.href,
               app_href = excluded.app_href, identity_id = excluded.identity_id,
               identity_name = excluded.identity_name, identity_email = excluded.identity_email",
            params![
                account.id,
                account.name,
                account.href,
                account.app_href,
                account.identity_id,
                account.identity_name,
                account.identity_email
            ],
        )?;
        Ok(())
    }

    pub fn remove_account(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// The projects of an account as last stored, in order.
    pub fn cached_projects(&self, account_id: i64) -> Result<Vec<Project>> {
        let mut statement = self
            .conn
            .prepare("SELECT json FROM projects WHERE account_id = ?1 ORDER BY position")?;
        let rows = statement.query_map(params![account_id], |row| row.get::<_, String>(0))?;
        let mut projects = Vec::new();
        for json in rows {
            match serde_json::from_str::<Project>(&json?) {
                Ok(project) => projects.push(project),
                // A row from an older model: skipped, overwritten on the next store.
                Err(error) => tracing::warn!("a cached project did not parse: {error}"),
            }
        }
        Ok(projects)
    }

    /// Replace an account's projects with this listing.
    pub fn store_projects(&mut self, account_id: i64, projects: &[Project]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM projects WHERE account_id = ?1",
            params![account_id],
        )?;
        for (position, project) in projects.iter().enumerate() {
            tx.execute(
                "INSERT INTO projects (id, account_id, position, json) VALUES (?1, ?2, ?3, ?4)",
                params![
                    project.id,
                    account_id,
                    position as i64,
                    serde_json::to_string(project)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep these recordings' words for searching offline, replacing what
    /// was kept of them (a partial copy only fills a gap). A trashed one is
    /// dropped instead.
    pub fn index_recordings<T: Searchable>(&mut self, account_id: i64, items: &[T]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare_cached(
                "INSERT INTO recordings (id, account_id, kind, bucket_kind, created_at, title, text, json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET account_id = excluded.account_id,
                   kind = excluded.kind, bucket_kind = excluded.bucket_kind,
                   created_at = excluded.created_at, title = excluded.title,
                   text = excluded.text, json = excluded.json
                 WHERE recordings.title IS NOT excluded.title
                   OR recordings.text IS NOT excluded.text
                   OR recordings.json IS NOT excluded.json",
            )?;
            let mut insert = tx.prepare_cached(
                "INSERT INTO recordings (id, account_id, kind, bucket_kind, created_at, title, text, json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO NOTHING",
            )?;
            let mut delete = tx.prepare_cached("DELETE FROM recordings WHERE id = ?1")?;
            for item in items {
                let recording = item.recording();
                if recording.status == "trashed" {
                    delete.execute(params![recording.id])?;
                    continue;
                }
                let statement = if item.is_whole() {
                    &mut upsert
                } else {
                    &mut insert
                };
                statement.execute(params![
                    recording.id,
                    account_id,
                    recording.kind,
                    recording.bucket.kind,
                    recording.created_at.map(|at| at.to_rfc3339()),
                    recording.title,
                    item.search_text(),
                    serde_json::to_string(recording)?
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Search an account's kept recordings for every word typed (the last
    /// one as a prefix), optionally one kind of thing (a `type_names[]`
    /// key from Basecamp's search), best match or newest first. Each
    /// result's text is the stretch around what matched.
    pub fn search_recordings(
        &self,
        account_id: i64,
        words: &str,
        kind: Option<&str>,
        newest_first: bool,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let Some(query) = fts_query(words) else {
            return Ok(Vec::new());
        };
        let kind_filter = match kind {
            None => "",
            Some("Chat::Transcript") => {
                "AND r.kind LIKE 'Chat::Lines::%' AND r.bucket_kind = 'Project'"
            }
            Some("Circle") => "AND r.kind LIKE 'Chat::Lines::%' AND r.bucket_kind != 'Project'",
            Some("Attachment") => "AND r.kind = 'Upload'",
            Some("Todo") => "AND r.kind IN ('Todo', 'Todolist')",
            Some(_) => "AND r.kind = ?4",
        };
        let order = if newest_first {
            "r.created_at DESC"
        } else {
            // Words in the title count for more.
            "bm25(recordings_fts, 4.0, 1.0)"
        };
        let sql = format!(
            "SELECT r.json, snippet(recordings_fts, 1, '', '', '…', 24)
             FROM recordings_fts JOIN recordings r ON r.id = recordings_fts.rowid
             WHERE recordings_fts MATCH ?1 AND r.account_id = ?2 {kind_filter}
             ORDER BY {order} LIMIT ?3"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let map = |row: &rusqlite::Row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
        let rows = if kind_filter.contains("?4") {
            statement.query_map(params![query, account_id, limit as i64, kind], map)?
        } else {
            statement.query_map(params![query, account_id, limit as i64], map)?
        };
        let mut results = Vec::new();
        for row in rows {
            let (json, snippet) = row?;
            match serde_json::from_str::<Recording>(&json) {
                Ok(recording) => results.push(SearchResult {
                    recording,
                    plain_text_content: snippet,
                }),
                Err(error) => tracing::warn!("a kept recording did not parse: {error}"),
            }
        }
        Ok(results)
    }

    /// Drop each account's oldest kept recordings beyond `keep`.
    pub fn prune_index(&self, keep: usize) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM recordings WHERE id IN (
                SELECT id FROM (
                    SELECT id, row_number() OVER (
                        PARTITION BY account_id ORDER BY created_at DESC
                    ) AS n FROM recordings
                ) WHERE n > ?1
            )",
            params![keep as i64],
        )?)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .ok();
        Ok(value)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// What was typed as an FTS5 query: each word quoted (so nothing in it is
/// read as syntax), all of them required, the last a prefix, since it may
/// be half typed.
fn fts_query(words: &str) -> Option<String> {
    let words: Vec<String> = words
        .split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect();
    let mut query = words.join(" ");
    if query.is_empty() {
        return None;
    }
    query.push('*');
    Some(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: 1000001,
            name: "Example Co".into(),
            href: "https://3.basecampapi.com/1000001".into(),
            app_href: "https://3.basecamp.com/1000001".into(),
            identity_id: 7,
            identity_name: "Brandon Williams".into(),
            identity_email: "b@example.com".into(),
        }
    }

    #[test]
    fn migrates_and_round_trips_accounts() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.accounts().unwrap().is_empty());
        db.upsert_account(&account()).unwrap();
        let mut renamed = account();
        renamed.name = "Example Basecamp".into();
        db.upsert_account(&renamed).unwrap();
        assert_eq!(db.accounts().unwrap(), vec![renamed]);
        db.remove_account(1000001).unwrap();
        assert!(db.accounts().unwrap().is_empty());
    }

    #[test]
    fn projects_are_cached_per_account_in_order() {
        let mut db = Database::open_in_memory().unwrap();
        db.upsert_account(&account()).unwrap();
        let project = |id: i64, name: &str| Project {
            id,
            name: name.into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        };
        db.store_projects(1000001, &[project(2, "B"), project(1, "A")])
            .unwrap();
        let names: Vec<String> = db
            .cached_projects(1000001)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["B", "A"]);
        db.store_projects(1000001, &[project(1, "A")]).unwrap();
        assert_eq!(db.cached_projects(1000001).unwrap().len(), 1);
        assert!(db.cached_projects(1).unwrap().is_empty());
    }

    fn line(id: i64, text: &str, minutes_ago: i64) -> crate::models::ChatLine {
        crate::models::ChatLine {
            recording: Recording {
                id,
                status: "active".into(),
                kind: "Chat::Lines::Text".into(),
                title: text.into(),
                created_at: Some(chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)),
                bucket: crate::models::Bucket {
                    id: 1,
                    name: "Launch".into(),
                    kind: "Project".into(),
                },
                ..Default::default()
            },
            content: text.into(),
        }
    }

    fn found(db: &Database, words: &str, kind: Option<&str>, newest_first: bool) -> Vec<i64> {
        db.search_recordings(1000001, words, kind, newest_first, 50)
            .unwrap()
            .into_iter()
            .map(|result| result.recording.id)
            .collect()
    }

    fn index_entries(db: &Database, words: &str) -> i64 {
        db.conn
            .query_row(
                "SELECT count(*) FROM recordings_fts WHERE recordings_fts MATCH ?1",
                params![fts_query(words).unwrap()],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn kept_recordings_are_searched_by_every_word_and_a_prefix() {
        let mut db = Database::open_in_memory().unwrap();
        db.upsert_account(&account()).unwrap();
        db.index_recordings(
            1000001,
            &[
                line(1, "The café opens at nine", 30),
                line(2, "Cafe meeting moved", 10),
                line(3, "Nothing to see", 5),
            ],
        )
        .unwrap();
        let message = crate::models::Message {
            recording: Recording {
                id: 4,
                kind: "Message".into(),
                title: "Opening plans".into(),
                created_at: Some(chrono::Utc::now()),
                ..Default::default()
            },
            content: "<div>When the <strong>cafe</strong> opens</div>".into(),
            ..Default::default()
        };
        db.index_recordings(1000001, &[message]).unwrap();

        // Accents fold; every word must be there; the last may be half typed.
        let mut cafe = found(&db, "cafe", None, false);
        cafe.sort();
        assert_eq!(cafe, [1, 2, 4]);
        let mut opens = found(&db, "cafe op", None, false);
        opens.sort();
        assert_eq!(opens, [1, 4]);
        assert_eq!(found(&db, "cafe", None, true), [4, 2, 1]);
        assert_eq!(found(&db, "cafe", Some("Message"), false), [4]);
        let mut chats = found(&db, "cafe", Some("Chat::Transcript"), false);
        chats.sort();
        assert_eq!(chats, [1, 2]);
        assert!(found(&db, "cafe", Some("Circle"), false).is_empty());
        assert!(found(&db, "\"", None, false).is_empty());
        // The text shown is the stretch that matched, plain.
        let hit = db
            .search_recordings(1000001, "opens", Some("Message"), false, 50)
            .unwrap();
        assert_eq!(hit[0].plain_text_content, "When the cafe opens");
        assert_eq!(hit[0].recording.title, "Opening plans");
        // Another account sees none of it.
        assert!(
            db.search_recordings(1, "cafe", None, false, 50)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_edit_replaces_the_words_and_trashing_drops_them() {
        let mut db = Database::open_in_memory().unwrap();
        db.upsert_account(&account()).unwrap();
        db.index_recordings(1000001, &[line(1, "first draft", 1)])
            .unwrap();
        db.index_recordings(1000001, &[line(1, "second thoughts", 1)])
            .unwrap();
        assert!(found(&db, "draft", None, false).is_empty());
        assert_eq!(found(&db, "thoughts", None, false), [1]);
        assert_eq!(index_entries(&db, "draft"), 0);

        // A search hit with less in it doesn't replace the whole line.
        let hit = SearchResult {
            recording: line(1, "second thoughts", 1).recording,
            plain_text_content: "second".into(),
        };
        db.index_recordings(1000001, std::slice::from_ref(&hit))
            .unwrap();
        assert_eq!(found(&db, "thoughts", None, false), [1]);
        let mut other = hit;
        other.recording.id = 2;
        other.recording.title = "Another".into();
        db.index_recordings(1000001, &[other]).unwrap();
        assert_eq!(found(&db, "second", None, false).len(), 2);

        let mut trashed = line(1, "second thoughts", 1);
        trashed.recording.status = "trashed".into();
        db.index_recordings(1000001, &[trashed]).unwrap();
        assert_eq!(found(&db, "thoughts", None, false), [] as [i64; 0]);
        assert_eq!(index_entries(&db, "thoughts"), 0);
    }

    #[test]
    fn signing_out_and_pruning_empty_the_index() {
        let mut db = Database::open_in_memory().unwrap();
        db.upsert_account(&account()).unwrap();
        db.index_recordings(
            1000001,
            &[line(1, "old news", 30), line(2, "news today", 1)],
        )
        .unwrap();
        assert_eq!(db.prune_index(1).unwrap(), 1);
        assert_eq!(found(&db, "news", None, false), [2]);
        assert_eq!(index_entries(&db, "news"), 1);
        db.remove_account(1000001).unwrap();
        assert_eq!(index_entries(&db, "news"), 0);
    }

    #[test]
    fn typed_words_are_quoted_for_fts() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("\"\""), None);
        assert_eq!(
            fts_query("say \"hi\" OR NEAR(x"),
            Some("\"say\" \"hi\" \"OR\" \"NEAR(x\"*".into())
        );
    }

    #[test]
    fn meta_is_a_key_value_store() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.meta("x").unwrap(), None);
        db.set_meta("x", "1").unwrap();
        db.set_meta("x", "2").unwrap();
        assert_eq!(db.meta("x").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skali.db");
        Database::open(&path).unwrap();
        let db = Database::open(&path).unwrap();
        let version: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }
}
