//! CTF challenge + solve persistence (`ctf_challenges`, `ctf_solves`).
//!
//! Owned by the CTF lane. The `ctf_scoreboard` helper is also used by the
//! community lane (`/leaderboard`), so keep that exact signature.
//!
//! Flags are never persisted in plaintext: callers only ever store a SHA-256
//! hash and `flag_hash` is internal (never rendered in an embed).

use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{TransactionBehavior, params};

use super::Database;

/// One CTF challenge as stored in `ctf_challenges`.
///
/// `flag_hash` is internal. Do not put it in embeds, logs or user replies.
#[derive(Debug, Clone)]
pub struct Challenge {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub points: i64,
    pub flag_hash: String,
    pub channel_id: Option<u64>,
    pub description: Option<String>,
    pub created_by: u64,
    pub created_at: i64,
    pub first_blood_id: Option<u64>,
    pub first_blood_at: Option<i64>,
    pub solved_count: i64,
}

/// What a successful flag submission did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveOutcome {
    FirstBlood,
    Solved,
    AlreadySolved,
}

/// Column list matching [`row_to_challenge`]'s positional order.
const COLUMNS: &str = "id, name, category, points, flag_hash, channel_id, description, \
                       created_by, created_at, first_blood_id, first_blood_at, solved_count";

fn row_to_challenge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Challenge> {
    Ok(Challenge {
        id: row.get(0)?,
        name: row.get(1)?,
        category: row.get(2)?,
        points: row.get(3)?,
        flag_hash: row.get(4)?,
        channel_id: row.get::<_, Option<i64>>(5)?.map(|id| id as u64),
        description: row.get(6)?,
        created_by: row.get::<_, i64>(7)? as u64,
        created_at: row.get(8)?,
        first_blood_id: row.get::<_, Option<i64>>(9)?.map(|id| id as u64),
        first_blood_at: row.get(10)?,
        solved_count: row.get(11)?,
    })
}

impl Database {
    /// Create a challenge. Names are unique per guild, case-insensitively.
    ///
    /// Returns the new challenge id. Duplicate names produce a friendly error.
    // Mirrors the `ctf_challenges` insert; see `add_case` for the same rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn create_challenge(
        &self,
        guild_id: u64,
        name: &str,
        category: &str,
        points: i64,
        flag_hash: &str,
        channel_id: Option<u64>,
        description: Option<&str>,
        created_by: u64,
    ) -> Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let duplicate: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM ctf_challenges WHERE guild_id = ?1 AND LOWER(name) = LOWER(?2)
             )",
            params![guild_id as i64, name],
            |row| row.get(0),
        )?;
        if duplicate {
            bail!("A challenge named `{name}` already exists in this server.");
        }

        let inserted = tx.execute(
            "INSERT INTO ctf_challenges
                 (guild_id, name, category, points, flag_hash, channel_id, description,
                  created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                guild_id as i64,
                name,
                category,
                points,
                flag_hash,
                channel_id.map(|id| id as i64),
                description,
                created_by as i64,
                Utc::now().timestamp(),
            ],
        );

        let id = match inserted {
            Ok(_) => tx.last_insert_rowid(),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                bail!("A challenge named `{name}` already exists in this server.");
            }
            Err(error) => return Err(error.into()),
        };

        tx.commit()?;
        Ok(id)
    }

    /// Fetch one challenge by name, case-insensitively.
    pub fn get_challenge_by_name(&self, guild_id: u64, name: &str) -> Result<Option<Challenge>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {COLUMNS} FROM ctf_challenges
             WHERE guild_id = ?1 AND LOWER(name) = LOWER(?2)"
        );
        let result = conn.query_row(&sql, params![guild_id as i64, name], row_to_challenge);
        match result {
            Ok(challenge) => Ok(Some(challenge)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Every challenge in a guild, newest first.
    pub fn list_challenges(&self, guild_id: u64) -> Result<Vec<Challenge>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {COLUMNS} FROM ctf_challenges WHERE guild_id = ?1 ORDER BY created_at DESC, id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![guild_id as i64], row_to_challenge)?;

        let mut challenges = Vec::new();
        for row in rows {
            challenges.push(row?);
        }
        Ok(challenges)
    }

    /// Delete a challenge and its solves in one transaction.
    ///
    /// Returns `false` when no challenge matched the (case-insensitive) name.
    pub fn delete_challenge(&self, guild_id: u64, name: &str) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let id = match tx.query_row(
            "SELECT id FROM ctf_challenges WHERE guild_id = ?1 AND LOWER(name) = LOWER(?2)",
            params![guild_id as i64, name],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
            Err(error) => return Err(error.into()),
        };

        tx.execute(
            "DELETE FROM ctf_solves WHERE challenge_id = ?1 AND guild_id = ?2",
            params![id, guild_id as i64],
        )?;
        tx.execute(
            "DELETE FROM ctf_challenges WHERE id = ?1 AND guild_id = ?2",
            params![id, guild_id as i64],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Record a solve. `None` means the challenge does not exist in this guild.
    pub fn record_solve(
        &self,
        challenge_id: i64,
        guild_id: u64,
        user_id: u64,
        points: i64,
    ) -> Result<Option<SolveOutcome>> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let first_blood = match tx.query_row(
            "SELECT first_blood_id FROM ctf_challenges WHERE id = ?1 AND guild_id = ?2",
            params![challenge_id, guild_id as i64],
            |row| row.get::<_, Option<i64>>(0),
        ) {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        let inserted = tx.execute(
            "INSERT OR IGNORE INTO ctf_solves (challenge_id, guild_id, user_id, points, solved_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                challenge_id,
                guild_id as i64,
                user_id as i64,
                points,
                Utc::now().timestamp(),
            ],
        )?;
        if inserted == 0 {
            return Ok(Some(SolveOutcome::AlreadySolved));
        }

        let outcome = if first_blood.is_none() {
            tx.execute(
                "UPDATE ctf_challenges
                    SET solved_count = solved_count + 1,
                        first_blood_id = ?2,
                        first_blood_at = ?3
                  WHERE id = ?1 AND guild_id = ?4",
                params![
                    challenge_id,
                    user_id as i64,
                    Utc::now().timestamp(),
                    guild_id as i64
                ],
            )?;
            SolveOutcome::FirstBlood
        } else {
            tx.execute(
                "UPDATE ctf_challenges SET solved_count = solved_count + 1
                  WHERE id = ?1 AND guild_id = ?2",
                params![challenge_id, guild_id as i64],
            )?;
            SolveOutcome::Solved
        };

        tx.commit()?;
        Ok(Some(outcome))
    }

    /// `(user_id, points)` ordered by points desc, then earliest last solve.
    pub fn ctf_scoreboard(&self, guild_id: u64, limit: usize) -> Result<Vec<(u64, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT user_id, SUM(points) AS total, MAX(solved_at) AS last_solve
               FROM ctf_solves
              WHERE guild_id = ?1
              GROUP BY user_id
              ORDER BY total DESC, last_solve ASC, user_id ASC
              LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)?))
        })?;

        let mut scoreboard = Vec::new();
        for row in rows {
            scoreboard.push(row?);
        }
        Ok(scoreboard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ctf.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    fn create(db: &Database, guild_id: u64, name: &str) -> i64 {
        db.create_challenge(
            guild_id,
            name,
            "web",
            100,
            &format!("hash-for-{name}"),
            None,
            None,
            999,
        )
        .expect("create challenge")
    }

    #[test]
    fn create_rejects_duplicates_case_insensitively() {
        let (_dir, db) = temp_db();
        create(&db, 1, "Web Login");

        let error = db
            .create_challenge(1, "web login", "web", 100, "h", None, None, 1)
            .expect_err("duplicate rejected");
        assert!(error.to_string().contains("already exists"), "{error}");

        // The same name in another guild is a different challenge.
        db.create_challenge(2, "web login", "web", 50, "h", None, None, 1)
            .expect("other guild");
    }

    #[test]
    fn lookups_are_case_insensitive_and_guild_scoped() {
        let (_dir, db) = temp_db();
        let id = create(&db, 1, "Forensics 101");

        let found = db
            .get_challenge_by_name(1, "forensics 101")
            .expect("query")
            .expect("found");
        assert_eq!(found.id, id);
        assert_eq!(found.name, "Forensics 101");

        assert!(
            db.get_challenge_by_name(2, "Forensics 101")
                .expect("query")
                .is_none()
        );
    }

    #[test]
    fn list_is_newest_first() {
        let (_dir, db) = temp_db();
        create(&db, 1, "First");
        create(&db, 1, "Second");

        let names: Vec<String> = db
            .list_challenges(1)
            .expect("list")
            .into_iter()
            .map(|challenge| challenge.name)
            .collect();
        assert_eq!(names, vec!["Second".to_owned(), "First".to_owned()]);
        assert!(db.list_challenges(2).expect("other guild").is_empty());
    }

    #[test]
    fn solve_flow_tracks_first_blood_and_duplicates() {
        let (_dir, db) = temp_db();
        let id = create(&db, 1, "Heap");

        assert_eq!(
            db.record_solve(id, 1, 10, 100).expect("first solve"),
            Some(SolveOutcome::FirstBlood)
        );

        let challenge = db
            .get_challenge_by_name(1, "Heap")
            .expect("query")
            .expect("found");
        assert_eq!(challenge.first_blood_id, Some(10));
        assert!(challenge.first_blood_at.is_some());
        assert_eq!(challenge.solved_count, 1);

        assert_eq!(
            db.record_solve(id, 1, 20, 100).expect("second solve"),
            Some(SolveOutcome::Solved)
        );
        assert_eq!(
            db.record_solve(id, 1, 10, 100).expect("duplicate solve"),
            Some(SolveOutcome::AlreadySolved)
        );

        let challenge = db
            .get_challenge_by_name(1, "Heap")
            .expect("query")
            .expect("found");
        assert_eq!(challenge.first_blood_id, Some(10));
        assert_eq!(challenge.solved_count, 2);

        assert_eq!(db.record_solve(9_999, 1, 30, 100).expect("unknown"), None);
    }

    #[test]
    fn scoreboard_orders_by_points_then_earliest_last_solve() {
        let (_dir, db) = temp_db();
        let easy = create(&db, 1, "Easy");
        let hard = create(&db, 1, "Hard");

        // users 10 and 20 both score 300; user 30 scores 200.
        for user in [10_u64, 20] {
            db.record_solve(easy, 1, user, 100).expect("solve");
            db.record_solve(hard, 1, user, 200).expect("solve");
        }
        db.record_solve(hard, 1, 30, 200).expect("solve");

        // Force distinct last-solve timestamps: user 10 finished earlier.
        {
            let conn = db.conn();
            conn.execute(
                "UPDATE ctf_solves SET solved_at = 1000 WHERE guild_id = 1 AND user_id = 10",
                [],
            )
            .expect("retime user 10");
            conn.execute(
                "UPDATE ctf_solves SET solved_at = 2000 WHERE guild_id = 1 AND user_id = 20",
                [],
            )
            .expect("retime user 20");
            conn.execute(
                "UPDATE ctf_solves SET solved_at = 3000 WHERE guild_id = 1 AND user_id = 30",
                [],
            )
            .expect("retime user 30");
        }

        assert_eq!(
            db.ctf_scoreboard(1, 10).expect("scoreboard"),
            vec![(10, 300), (20, 300), (30, 200)]
        );
        assert_eq!(
            db.ctf_scoreboard(1, 2).expect("limited"),
            vec![(10, 300), (20, 300)]
        );
        assert!(db.ctf_scoreboard(2, 10).expect("other guild").is_empty());
    }

    #[test]
    fn delete_cascades_solves() {
        let (_dir, db) = temp_db();
        let id = create(&db, 1, "Crypto");
        db.record_solve(id, 1, 7, 100).expect("solve");

        assert!(db.delete_challenge(1, "crypto").expect("delete"));
        assert!(!db.delete_challenge(1, "crypto").expect("already gone"));
        assert!(
            db.get_challenge_by_name(1, "Crypto")
                .expect("query")
                .is_none()
        );
        assert!(db.list_challenges(1).expect("list").is_empty());
    }
}
