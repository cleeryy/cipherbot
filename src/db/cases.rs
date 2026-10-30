//! Moderation cases (`cases`) and phishing feed bookkeeping (`feed_state`).
//!
//! Case numbers are per guild and monotonic: `add_case` reserves
//! `MAX(case_number) + 1` inside a transaction so concurrent moderators never
//! collide on the `UNIQUE(guild_id, case_number)` constraint.
//!
//! `count_active_warns` is also used by the community lane (`/profile`), so its
//! signature is frozen.

use anyhow::Result;
use rusqlite::{OptionalExtension, Row, params};

use crate::util::time::unix_now;

use super::Database;

/// One moderation case, owned (no borrow of the connection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: i64,
    pub guild_id: u64,
    pub case_number: i64,
    pub kind: String,
    pub target_id: u64,
    pub moderator_id: u64,
    pub reason: String,
    pub evidence: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub active: bool,
}

impl Database {
    /// Record a case and return `(row id, case number)`.
    ///
    /// The case number is `MAX(case_number) + 1` for the guild, computed and
    /// inserted in the same transaction.
    // Flat insert signature mirrors the `cases` table one-to-one; a builder
    // struct here would add noise without making call sites clearer.
    #[allow(clippy::too_many_arguments)]
    pub fn add_case(
        &self,
        guild_id: u64,
        kind: &str,
        target_id: u64,
        moderator_id: u64,
        reason: &str,
        evidence: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<(i64, i64)> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;

        let case_number: i64 = tx.query_row(
            "SELECT COALESCE(MAX(case_number), 0) + 1 FROM cases WHERE guild_id = ?1",
            params![guild_id as i64],
            |row| row.get(0),
        )?;

        tx.execute(
            "INSERT INTO cases (
                 guild_id, case_number, kind, target_id, moderator_id,
                 reason, evidence, created_at, expires_at, active
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)",
            params![
                guild_id as i64,
                case_number,
                kind,
                target_id as i64,
                moderator_id as i64,
                reason,
                evidence,
                unix_now(),
                expires_at,
            ],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok((id, case_number))
    }

    pub fn get_case(&self, guild_id: u64, case_number: i64) -> Result<Option<Case>> {
        let conn = self.conn();
        let case = conn
            .query_row(
                "SELECT id, guild_id, case_number, kind, target_id, moderator_id,
                        reason, evidence, created_at, expires_at, active
                 FROM cases
                 WHERE guild_id = ?1 AND case_number = ?2",
                params![guild_id as i64, case_number],
                map_case,
            )
            .optional()?;
        Ok(case)
    }

    /// Most recent cases first, optionally filtered by target member.
    pub fn list_cases(
        &self,
        guild_id: u64,
        target: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Case>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn();

        let mut cases = Vec::new();
        match target {
            Some(target_id) => {
                let mut stmt = conn.prepare(
                    "SELECT id, guild_id, case_number, kind, target_id, moderator_id,
                            reason, evidence, created_at, expires_at, active
                     FROM cases
                     WHERE guild_id = ?1 AND target_id = ?2
                     ORDER BY case_number DESC
                     LIMIT ?3",
                )?;
                let rows =
                    stmt.query_map(params![guild_id as i64, target_id as i64, limit], map_case)?;
                for row in rows {
                    cases.push(row?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, guild_id, case_number, kind, target_id, moderator_id,
                            reason, evidence, created_at, expires_at, active
                     FROM cases
                     WHERE guild_id = ?1
                     ORDER BY case_number DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![guild_id as i64, limit], map_case)?;
                for row in rows {
                    cases.push(row?);
                }
            }
        }

        Ok(cases)
    }

    /// Mark a case inactive (e.g. a timeout that expired). Returns whether a row changed.
    pub fn deactivate_case(&self, guild_id: u64, case_number: i64) -> Result<bool> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE cases SET active = 0
              WHERE guild_id = ?1 AND case_number = ?2 AND active = 1",
            params![guild_id as i64, case_number],
        )?;
        Ok(changed > 0)
    }

    /// Number of active warnings for a member.
    pub fn count_active_warns(&self, guild_id: u64, target_id: u64) -> Result<u64> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM cases
             WHERE guild_id = ?1 AND target_id = ?2 AND kind = 'warn' AND active = 1",
            params![guild_id as i64, target_id as i64],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Upsert feed bookkeeping (`feed_state`): last check time and entry count.
    pub fn set_feed_state(&self, name: &str, entries: usize) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO feed_state (name, last_checked, entries)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET
                 last_checked = excluded.last_checked,
                 entries = excluded.entries",
            params![name, unix_now(), entries as i64],
        )?;
        Ok(())
    }
}

fn map_case(row: &Row<'_>) -> rusqlite::Result<Case> {
    Ok(Case {
        id: row.get(0)?,
        guild_id: row.get::<_, i64>(1)? as u64,
        case_number: row.get(2)?,
        kind: row.get(3)?,
        target_id: row.get::<_, i64>(4)? as u64,
        moderator_id: row.get::<_, i64>(5)? as u64,
        reason: row.get(6)?,
        evidence: row.get(7)?,
        created_at: row.get(8)?,
        expires_at: row.get(9)?,
        active: row.get::<_, i64>(10)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cases.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn case_numbers_are_per_guild_and_monotonic() {
        let (_dir, db) = temp_db();
        let guild_a = 1u64;
        let guild_b = 2u64;

        for expected in 1..=3 {
            let (_id, number) = db
                .add_case(guild_a, "warn", 10, 99, "spam", None, None)
                .expect("case");
            assert_eq!(number, expected);
        }
        let (_id, number) = db
            .add_case(guild_b, "kick", 11, 99, "raid", None, None)
            .expect("case");
        assert_eq!(number, 1, "other guilds have their own numbering");

        let (_id, number) = db
            .add_case(guild_a, "ban", 12, 99, "abuse", Some("link"), None)
            .expect("case");
        assert_eq!(number, 4);
    }

    #[test]
    fn case_numbering_survives_gaps() {
        let (_dir, db) = temp_db();
        let (_id, first) = db.add_case(7, "warn", 1, 2, "a", None, None).expect("case");
        assert_eq!(first, 1);

        // Simulate a deleted/never-reused high number.
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO cases (guild_id, case_number, kind, target_id, moderator_id, reason, created_at)
                 VALUES (7, 99, 'warn', 1, 2, 'legacy', 0)",
                [],
            )
            .expect("insert");
        }

        let (_id, next) = db.add_case(7, "warn", 1, 2, "b", None, None).expect("case");
        assert_eq!(
            next, 100,
            "numbering continues after the highest case number"
        );
    }

    #[test]
    fn get_list_and_deactivate_roundtrip() {
        let (_dir, db) = temp_db();
        let (_id, number) = db
            .add_case(
                1,
                "timeout",
                42,
                7,
                "flooding",
                Some("msg link"),
                Some(1_000),
            )
            .expect("case");

        let case = db.get_case(1, number).expect("get").expect("present");
        assert_eq!(case.kind, "timeout");
        assert_eq!(case.target_id, 42);
        assert_eq!(case.moderator_id, 7);
        assert_eq!(case.evidence.as_deref(), Some("msg link"));
        assert_eq!(case.expires_at, Some(1_000));
        assert!(case.active);

        assert!(db.get_case(1, 999).expect("missing").is_none());
        assert!(db.get_case(2, number).expect("other guild").is_none());

        assert!(db.deactivate_case(1, number).expect("deactivate"));
        assert!(!db.deactivate_case(1, number).expect("already inactive"));
        assert!(
            !db.get_case(1, number)
                .expect("get")
                .expect("present")
                .active
        );

        db.add_case(1, "warn", 42, 7, "second", None, None)
            .expect("case");
        let listed = db.list_cases(1, Some(42), 10).expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].case_number, 2, "newest first");
        assert_eq!(db.list_cases(1, Some(999), 10).expect("list").len(), 0);
        assert_eq!(db.list_cases(1, None, 1).expect("list").len(), 1);
    }

    #[test]
    fn warn_count_only_counts_active_warns() {
        let (_dir, db) = temp_db();
        let (_id, first) = db.add_case(1, "warn", 5, 2, "a", None, None).expect("warn");
        db.add_case(1, "warn", 5, 2, "b", None, None).expect("warn");
        db.add_case(1, "warn", 6, 2, "c", None, None)
            .expect("other");
        db.add_case(1, "kick", 5, 2, "d", None, None).expect("kick");

        assert_eq!(db.count_active_warns(1, 5).expect("count"), 2);

        db.deactivate_case(1, first).expect("deactivate");
        assert_eq!(db.count_active_warns(1, 5).expect("count"), 1);
        assert_eq!(db.count_active_warns(2, 5).expect("other guild"), 0);
    }

    #[test]
    fn feed_state_upserts() {
        let (_dir, db) = temp_db();
        db.set_feed_state("urlhaus", 10).expect("insert");
        db.set_feed_state("urlhaus", 25).expect("update");

        let conn = db.conn();
        let entries: i64 = conn
            .query_row(
                "SELECT entries FROM feed_state WHERE name = 'urlhaus'",
                [],
                |row| row.get(0),
            )
            .expect("row");
        let checked: Option<i64> = conn
            .query_row(
                "SELECT last_checked FROM feed_state WHERE name = 'urlhaus'",
                [],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(entries, 25);
        assert!(checked.is_some());
    }
}
