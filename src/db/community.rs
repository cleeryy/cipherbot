//! Community persistence: reports, suggestions and suggestion votes.
//!
//! Owned by the community lane. Vote counters are recomputed from
//! `suggestion_votes` inside the same transaction so they can never drift.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use super::Database;

/// One user report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: i64,
    pub guild_id: u64,
    pub reporter_id: u64,
    pub target_id: u64,
    pub message_id: Option<u64>,
    pub reason: String,
    pub status: String,
    pub created_at: i64,
    pub handled_by: Option<u64>,
    pub handled_at: Option<i64>,
}

/// One server suggestion with its live vote counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub id: i64,
    pub guild_id: u64,
    pub author_id: u64,
    pub content: String,
    pub message_id: Option<u64>,
    pub upvotes: i64,
    pub downvotes: i64,
    pub status: String,
    pub created_at: i64,
}

impl Database {
    /// Store a report; returns its id.
    pub fn add_report(
        &self,
        guild_id: u64,
        reporter_id: u64,
        target_id: u64,
        message_id: Option<u64>,
        reason: &str,
    ) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO reports
                 (guild_id, reporter_id, target_id, message_id, reason, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6)",
            params![
                guild_id as i64,
                reporter_id as i64,
                target_id as i64,
                message_id.map(|id| id as i64),
                reason,
                Utc::now().timestamp()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Mark a report handled. Returns whether the status changed.
    pub fn resolve_report(&self, guild_id: u64, report_id: i64, moderator_id: u64) -> Result<bool> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE reports
                SET status = 'handled', handled_by = ?1, handled_at = ?2
              WHERE guild_id = ?3 AND id = ?4 AND status != 'handled'",
            params![
                moderator_id as i64,
                Utc::now().timestamp(),
                guild_id as i64,
                report_id
            ],
        )?;
        Ok(changed > 0)
    }

    /// Reports for a guild, optionally filtered by status (`""`/`all` = any).
    pub fn list_reports(&self, guild_id: u64, status: &str, limit: usize) -> Result<Vec<Report>> {
        let limit = limit.clamp(1, 100) as i64;
        let status = status.trim();
        let conn = self.conn();

        let filter = if status.is_empty() || status.eq_ignore_ascii_case("all") {
            None
        } else {
            Some(status)
        };

        let mut reports = Vec::new();
        if let Some(status) = filter {
            let mut stmt = conn.prepare(
                "SELECT id, guild_id, reporter_id, target_id, message_id, reason, status,
                        created_at, handled_by, handled_at
                 FROM reports
                 WHERE guild_id = ?1 AND status = ?2
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![guild_id as i64, status, limit], map_report)?;
            for row in rows {
                reports.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, guild_id, reporter_id, target_id, message_id, reason, status,
                        created_at, handled_by, handled_at
                 FROM reports
                 WHERE guild_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![guild_id as i64, limit], map_report)?;
            for row in rows {
                reports.push(row?);
            }
        }

        Ok(reports)
    }

    /// Store a suggestion; returns its id.
    pub fn add_suggestion(&self, guild_id: u64, author_id: u64, content: &str) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO suggestions (guild_id, author_id, content, status, created_at)
             VALUES (?1, ?2, ?3, 'open', ?4)",
            params![
                guild_id as i64,
                author_id as i64,
                content,
                Utc::now().timestamp()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Attach the Discord message carrying the vote buttons.
    pub fn set_suggestion_message(&self, id: i64, message_id: u64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE suggestions SET message_id = ?2 WHERE id = ?1",
            params![id, message_id as i64],
        )?;
        Ok(())
    }

    /// Apply one vote.
    ///
    /// `value` is `+1` (up) or `-1` (down). Repeating the same vote removes it;
    /// the opposite vote switches it. Returns the fresh `(up, down)` counters and
    /// the user's previous vote (if any).
    pub fn vote_suggestion(
        &self,
        guild_id: u64,
        suggestion_id: i64,
        user_id: u64,
        value: i8,
    ) -> Result<(i64, i64, Option<i8>)> {
        anyhow::ensure!(value == 1 || value == -1, "vote must be +1 or -1");

        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;

        let exists: Option<i64> = tx
            .query_row(
                "SELECT id FROM suggestions WHERE id = ?1 AND guild_id = ?2",
                params![suggestion_id, guild_id as i64],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            anyhow::bail!("Unknown suggestion #{suggestion_id}.");
        }

        let previous: Option<i8> = tx
            .query_row(
                "SELECT value FROM suggestion_votes
                 WHERE suggestion_id = ?1 AND user_id = ?2",
                params![suggestion_id, user_id as i64],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(|value| value as i8);

        match previous {
            Some(old) if old == value => {
                tx.execute(
                    "DELETE FROM suggestion_votes WHERE suggestion_id = ?1 AND user_id = ?2",
                    params![suggestion_id, user_id as i64],
                )?;
            }
            Some(_) => {
                tx.execute(
                    "UPDATE suggestion_votes SET value = ?3
                     WHERE suggestion_id = ?1 AND user_id = ?2",
                    params![suggestion_id, user_id as i64, value as i64],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO suggestion_votes
                         (suggestion_id, guild_id, user_id, value, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        suggestion_id,
                        guild_id as i64,
                        user_id as i64,
                        value as i64,
                        Utc::now().timestamp()
                    ],
                )?;
            }
        }

        let up: i64 = tx.query_row(
            "SELECT COUNT(*) FROM suggestion_votes WHERE suggestion_id = ?1 AND value = 1",
            params![suggestion_id],
            |row| row.get(0),
        )?;
        let down: i64 = tx.query_row(
            "SELECT COUNT(*) FROM suggestion_votes WHERE suggestion_id = ?1 AND value = -1",
            params![suggestion_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE suggestions SET upvotes = ?2, downvotes = ?3 WHERE id = ?1",
            params![suggestion_id, up, down],
        )?;
        tx.commit()?;

        Ok((up, down, previous))
    }

    /// Fetch one suggestion (guild scoped).
    pub fn get_suggestion(&self, guild_id: u64, id: i64) -> Result<Option<Suggestion>> {
        let conn = self.conn();
        let suggestion = conn
            .query_row(
                "SELECT id, guild_id, author_id, content, message_id, upvotes, downvotes,
                        status, created_at
                 FROM suggestions
                 WHERE id = ?1 AND guild_id = ?2",
                params![id, guild_id as i64],
                |row| {
                    Ok(Suggestion {
                        id: row.get(0)?,
                        guild_id: row.get::<_, i64>(1)? as u64,
                        author_id: row.get::<_, i64>(2)? as u64,
                        content: row.get(3)?,
                        message_id: row.get::<_, Option<i64>>(4)?.map(|id| id as u64),
                        upvotes: row.get(5)?,
                        downvotes: row.get(6)?,
                        status: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(suggestion)
    }
}

fn map_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
    Ok(Report {
        id: row.get(0)?,
        guild_id: row.get::<_, i64>(1)? as u64,
        reporter_id: row.get::<_, i64>(2)? as u64,
        target_id: row.get::<_, i64>(3)? as u64,
        message_id: row.get::<_, Option<i64>>(4)?.map(|id| id as u64),
        reason: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        handled_by: row.get::<_, Option<i64>>(8)?.map(|id| id as u64),
        handled_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("community.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn report_roundtrip() {
        let (_dir, db) = temp_db();
        let id = db
            .add_report(1, 10, 20, Some(300), "spamming invites")
            .expect("add");
        assert!(id > 0);

        let reports = db.list_reports(1, "open", 10).expect("list");
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.id, id);
        assert_eq!(report.reporter_id, 10);
        assert_eq!(report.target_id, 20);
        assert_eq!(report.message_id, Some(300));
        assert_eq!(report.reason, "spamming invites");
        assert_eq!(report.status, "open");
        assert!(report.handled_by.is_none());

        assert!(db.list_reports(1, "closed", 10).expect("closed").is_empty());
        assert!(
            db.list_reports(2, "all", 10)
                .expect("other guild")
                .is_empty()
        );
    }

    #[test]
    fn report_resolution_is_idempotent() {
        let (_dir, db) = temp_db();
        let id = db.add_report(1, 10, 20, None, "raid").expect("add");

        assert!(db.resolve_report(1, id, 99).expect("resolve"));
        let handled = db.list_reports(1, "handled", 10).expect("list");
        assert_eq!(handled.len(), 1);
        assert_eq!(handled[0].handled_by, Some(99));
        assert!(handled[0].handled_at.is_some());

        // Second resolution is a no-op, not an error.
        assert!(!db.resolve_report(1, id, 99).expect("resolve twice"));
        assert!(!db.resolve_report(1, 999, 99).expect("unknown"));
    }

    #[test]
    fn votes_toggle_and_switch() {
        let (_dir, db) = temp_db();
        let id = db.add_suggestion(1, 30, "add a CTF night").expect("add");

        // First upvote.
        let (up, down, previous) = db.vote_suggestion(1, id, 40, 1).expect("up");
        assert_eq!((up, down), (1, 0));
        assert_eq!(previous, None);

        // Same vote again -> removed.
        let (up, down, previous) = db.vote_suggestion(1, id, 40, 1).expect("toggle off");
        assert_eq!((up, down), (0, 0));
        assert_eq!(previous, Some(1));

        // Up then switch to down.
        db.vote_suggestion(1, id, 40, 1).expect("up again");
        let (up, down, previous) = db.vote_suggestion(1, id, 40, -1).expect("switch");
        assert_eq!((up, down), (0, 1));
        assert_eq!(previous, Some(1));

        // A second member votes independently.
        let (up, down, _) = db.vote_suggestion(1, id, 41, 1).expect("second member");
        assert_eq!((up, down), (1, 1));

        let suggestion = db.get_suggestion(1, id).expect("get").expect("exists");
        assert_eq!((suggestion.upvotes, suggestion.downvotes), (1, 1));
        assert_eq!(suggestion.author_id, 30);
        assert_eq!(suggestion.content, "add a CTF night");
    }

    #[test]
    fn unknown_suggestions_error() {
        let (_dir, db) = temp_db();
        assert!(db.get_suggestion(1, 999).expect("lookup").is_none());
        assert!(db.vote_suggestion(1, 999, 40, 1).is_err());

        let id = db.add_suggestion(1, 30, "idea").expect("add");
        assert!(db.vote_suggestion(2, id, 40, 1).is_err());
    }

    #[test]
    fn suggestion_message_id_is_saved() {
        let (_dir, db) = temp_db();
        let id = db.add_suggestion(1, 30, "idea").expect("add");
        db.set_suggestion_message(id, 777).expect("set");
        let suggestion = db.get_suggestion(1, id).expect("get").expect("exists");
        assert_eq!(suggestion.message_id, Some(777));
    }
}
