//! The SQLite cache: accounts, and later everything Basecamp shows so the
//! window opens on what it last saw. One connection, on the main thread.
//! Migrations are append-only, tracked by `PRAGMA user_version`.

use crate::error::Result;
use crate::models::Account;
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
];

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
        let path = dir.path().join("campfe.db");
        Database::open(&path).unwrap();
        let db = Database::open(&path).unwrap();
        let version: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }
}
