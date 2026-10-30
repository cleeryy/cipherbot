//! SQLite persistence.
//!
//! One connection behind a `parking_lot::Mutex`; every public helper takes the
//! lock once and returns owned data. No public helper calls another one.
//!
//! Timestamps are unix seconds (`INTEGER`) for all v1 tables. The two legacy
//! link-pipeline tables are the exception: `tracked_messages.delete_at` is a
//! RFC3339 `TEXT` string because the shipped pipeline has always written that
//! format. Do not "fix" it with a destructive migration — existing dev
//! databases must keep working.

use std::path::Path;

use anyhow::{Context as _, Result};
use parking_lot::{Mutex, MutexGuard};
use rusqlite::Connection;

pub mod cases;
pub mod community;
pub mod ctf;
pub mod evidence;
pub mod guild;
pub mod intel;
pub mod links;
pub mod users;

#[allow(unused_imports)]
pub use guild::GuildSettings;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS guild_config (
    guild_id      INTEGER PRIMARY KEY,
    settings_json TEXT NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cases (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id     INTEGER NOT NULL,
    case_number  INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    target_id    INTEGER NOT NULL,
    moderator_id INTEGER NOT NULL,
    reason       TEXT NOT NULL,
    evidence     TEXT,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER,
    active       INTEGER NOT NULL DEFAULT 1,
    UNIQUE(guild_id, case_number)
);

CREATE TABLE IF NOT EXISTS ctf_challenges (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id       INTEGER NOT NULL,
    name           TEXT NOT NULL,
    category       TEXT NOT NULL,
    points         INTEGER NOT NULL,
    flag_hash      TEXT NOT NULL,
    channel_id     INTEGER,
    description    TEXT,
    created_by     INTEGER NOT NULL,
    created_at     INTEGER NOT NULL,
    first_blood_id INTEGER,
    first_blood_at INTEGER,
    solved_count   INTEGER NOT NULL DEFAULT 0,
    UNIQUE(guild_id, name)
);

CREATE TABLE IF NOT EXISTS ctf_solves (
    challenge_id INTEGER NOT NULL,
    guild_id     INTEGER NOT NULL,
    user_id      INTEGER NOT NULL,
    points       INTEGER NOT NULL,
    solved_at    INTEGER NOT NULL,
    PRIMARY KEY(challenge_id, user_id)
);

CREATE TABLE IF NOT EXISTS users (
    guild_id     INTEGER NOT NULL,
    user_id      INTEGER NOT NULL,
    xp           INTEGER NOT NULL DEFAULT 0,
    messages     INTEGER NOT NULL DEFAULT 0,
    quiz_correct INTEGER NOT NULL DEFAULT 0,
    quiz_total   INTEGER NOT NULL DEFAULT 0,
    streak       INTEGER NOT NULL DEFAULT 0,
    last_quiz_day TEXT,
    first_seen   INTEGER NOT NULL,
    PRIMARY KEY(guild_id, user_id)
);

CREATE TABLE IF NOT EXISTS links (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id   INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    thread_id  INTEGER,
    author_id  INTEGER NOT NULL,
    url        TEXT NOT NULL,
    domain     TEXT NOT NULL,
    posted_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS intel_cache (
    cache_key  TEXT PRIMARY KEY,
    payload    TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS feed_state (
    name         TEXT PRIMARY KEY,
    last_checked INTEGER,
    entries      INTEGER
);

CREATE TABLE IF NOT EXISTS reports (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id    INTEGER NOT NULL,
    reporter_id INTEGER NOT NULL,
    target_id   INTEGER NOT NULL,
    message_id  INTEGER,
    reason      TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'open',
    created_at  INTEGER NOT NULL,
    handled_by  INTEGER,
    handled_at  INTEGER
);

CREATE TABLE IF NOT EXISTS suggestions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id    INTEGER NOT NULL,
    author_id   INTEGER NOT NULL,
    content     TEXT NOT NULL,
    message_id  INTEGER,
    upvotes     INTEGER NOT NULL DEFAULT 0,
    downvotes   INTEGER NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'open',
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS suggestion_votes (
    suggestion_id INTEGER NOT NULL,
    guild_id      INTEGER NOT NULL,
    user_id       INTEGER NOT NULL,
    value         INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY(suggestion_id, user_id)
);

CREATE TABLE IF NOT EXISTS evidence (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id   INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    author_id  INTEGER NOT NULL,
    content    TEXT NOT NULL,
    reason     TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- Legacy link pipeline (delete_at is RFC3339 TEXT on purpose, see module docs).
CREATE TABLE IF NOT EXISTS tracked_messages (
    message_id INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL,
    delete_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS channel_threads (
    channel_id INTEGER PRIMARY KEY,
    thread_id  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cases_guild ON cases(guild_id, case_number);
CREATE INDEX IF NOT EXISTS idx_ctf_solves_guild ON ctf_solves(guild_id, user_id);
CREATE INDEX IF NOT EXISTS idx_links_guild ON links(guild_id, posted_at);
CREATE INDEX IF NOT EXISTS idx_intel_cache_expires ON intel_cache(expires_at);
CREATE INDEX IF NOT EXISTS idx_tracked_delete_at ON tracked_messages(delete_at);
"#;

/// Connection manager for the bot's single SQLite database.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &str) -> Result<Self> {
        if path != ":memory:"
            && let Some(parent) = Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory {parent:?}"))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening SQLite database at '{path}'"))?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        tracing::debug!("Database opened at '{path}'");
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create every v1 table and bump `PRAGMA user_version`.
    ///
    /// Idempotent: safe to call on every boot and on a database already at v1.
    pub fn initialize(&self) -> Result<()> {
        let conn = self.conn();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        conn.execute_batch(SCHEMA_V1)
            .context("applying schema v1")?;

        if version < 1 {
            conn.pragma_update(None, "user_version", 1)?;
            tracing::info!("Database migrated to schema v1");
        } else {
            tracing::debug!("Database schema already at v{version}");
        }

        Ok(())
    }

    /// Cheap liveness probe used by `/healthz`.
    pub fn health_check(&self) -> bool {
        let conn = self.conn();
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map(|value| value == 1)
            .unwrap_or(false)
    }

    /// Take the connection lock. Visible to `db` submodules only.
    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open db");
        (dir, db)
    }

    #[test]
    fn initialize_is_idempotent() {
        let (_dir, db) = temp_db();
        db.initialize().expect("first init");
        db.initialize().expect("second init");

        let conn = db.conn();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        assert_eq!(version, 1);

        for table in [
            "guild_config",
            "cases",
            "ctf_challenges",
            "ctf_solves",
            "users",
            "links",
            "intel_cache",
            "feed_state",
            "reports",
            "suggestions",
            "suggestion_votes",
            "evidence",
            "tracked_messages",
            "channel_threads",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("table lookup");
            assert_eq!(count, 1, "missing table {table}");
        }
    }

    #[test]
    fn health_check_works() {
        let (_dir, db) = temp_db();
        db.initialize().expect("init");
        assert!(db.health_check());
    }
}
