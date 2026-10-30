//! Captured evidence for automod and moderation actions (`evidence`).
//!
//! Content is truncated to 4000 characters before it reaches SQLite so a
//! runaway paste can never bloat the database or the WAL. Callers that handle
//! secrets (tokens, flags) redact them before calling `add_evidence`.

use anyhow::Result;
use rusqlite::{Row, params};

use crate::util::text::truncate;
use crate::util::time::unix_now;

use super::Database;

/// Hard cap on stored content, well under Discord's 2000-char message limit
/// plus edits/markdown overhead.
const MAX_CONTENT: usize = 4_000;

/// One captured message, owned (no borrow of the connection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub id: i64,
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub author_id: u64,
    pub content: String,
    pub reason: String,
    pub created_at: i64,
}

impl Database {
    /// Store a message snapshot as evidence. Returns the new row id.
    pub fn add_evidence(
        &self,
        guild_id: u64,
        channel_id: u64,
        message_id: u64,
        author_id: u64,
        content: &str,
        reason: &str,
    ) -> Result<i64> {
        let content = truncate(content, MAX_CONTENT);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO evidence (
                 guild_id, channel_id, message_id, author_id, content, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                guild_id as i64,
                channel_id as i64,
                message_id as i64,
                author_id as i64,
                content,
                reason,
                unix_now(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Most recent evidence rows first. `limit` is clamped to `1..=100`.
    pub fn list_evidence(&self, guild_id: u64, limit: usize) -> Result<Vec<Evidence>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, channel_id, message_id, author_id, content, reason, created_at
             FROM evidence
             WHERE guild_id = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, limit], map_evidence)?;
        let mut evidence = Vec::new();
        for row in rows {
            evidence.push(row?);
        }
        Ok(evidence)
    }
}

fn map_evidence(row: &Row<'_>) -> rusqlite::Result<Evidence> {
    Ok(Evidence {
        id: row.get(0)?,
        guild_id: row.get::<_, i64>(1)? as u64,
        channel_id: row.get::<_, i64>(2)? as u64,
        message_id: row.get::<_, i64>(3)? as u64,
        author_id: row.get::<_, i64>(4)? as u64,
        content: row.get(5)?,
        reason: row.get(6)?,
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("evidence.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn evidence_roundtrip() {
        let (_dir, db) = temp_db();
        let id = db
            .add_evidence(1, 10, 20, 30, "buy cheap nitro", "Automod: invite link")
            .expect("insert");
        assert!(id > 0);

        let rows = db.list_evidence(1, 10).expect("list");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.guild_id, 1);
        assert_eq!(row.channel_id, 10);
        assert_eq!(row.message_id, 20);
        assert_eq!(row.author_id, 30);
        assert_eq!(row.content, "buy cheap nitro");
        assert_eq!(row.reason, "Automod: invite link");
        assert!(row.created_at > 0);

        assert!(db.list_evidence(2, 10).expect("other guild").is_empty());
    }

    #[test]
    fn evidence_is_ordered_and_limited() {
        let (_dir, db) = temp_db();
        for index in 0..3 {
            db.add_evidence(1, 10, index, 30, "spam", "test")
                .expect("insert");
        }
        let rows = db.list_evidence(1, 2).expect("list");
        assert_eq!(rows.len(), 2);
        assert!(rows[0].id > rows[1].id, "newest first");
    }

    #[test]
    fn content_is_truncated() {
        let (_dir, db) = temp_db();
        let huge = "a".repeat(10_000);
        db.add_evidence(1, 1, 1, 1, &huge, "test").expect("insert");

        let rows = db.list_evidence(1, 1).expect("list");
        assert_eq!(rows[0].content.chars().count(), MAX_CONTENT);
    }
}
