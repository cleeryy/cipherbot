//! Poll persistence: polls and single-choice votes.
//!
//! Owned by the poll lane. Options live in `polls.options_json` as a JSON array
//! of strings; votes live in `poll_votes`, whose primary key is
//! `(poll_id, user_id)` — so a member can only ever hold exactly one choice.

use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use super::Database;

/// Minimum number of options a poll accepts.
pub const MIN_OPTIONS: usize = 2;
/// Maximum number of options a poll accepts (a Discord button row holds 5).
pub const MAX_OPTIONS: usize = 5;
/// Maximum question length, in characters.
pub const MAX_QUESTION_CHARS: usize = 200;
/// Maximum option length, in characters.
pub const MAX_OPTION_CHARS: usize = 80;

/// One stored poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Poll {
    pub id: i64,
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: Option<u64>,
    pub author_id: u64,
    pub question: String,
    pub options: Vec<String>,
    pub created_at: i64,
    pub closed_at: Option<i64>,
}

/// What happened to a vote attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollVoteOutcome {
    /// A new vote was recorded.
    Voted,
    /// The member switched from another option.
    Changed,
    /// The poll is closed; nothing was written.
    Closed,
    /// The choice index does not exist on this poll.
    InvalidChoice,
}

impl Database {
    /// Store a poll; returns its id. Validates the question and the options.
    pub fn create_poll(
        &self,
        guild_id: u64,
        channel_id: u64,
        author_id: u64,
        question: &str,
        options: &[String],
    ) -> Result<i64> {
        let question = question.trim();
        anyhow::ensure!(!question.is_empty(), "the poll question must not be empty");
        anyhow::ensure!(
            question.chars().count() <= MAX_QUESTION_CHARS,
            "the poll question must be at most {MAX_QUESTION_CHARS} characters"
        );
        anyhow::ensure!(
            (MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len()),
            "a poll needs between {MIN_OPTIONS} and {MAX_OPTIONS} options"
        );

        let mut cleaned = Vec::with_capacity(options.len());
        for option in options {
            let option = option.trim();
            anyhow::ensure!(!option.is_empty(), "poll options must not be empty");
            anyhow::ensure!(
                option.chars().count() <= MAX_OPTION_CHARS,
                "poll options must be at most {MAX_OPTION_CHARS} characters"
            );
            cleaned.push(option.to_owned());
        }

        let options_json = serde_json::to_string(&cleaned).context("serializing poll options")?;

        let conn = self.conn();
        conn.execute(
            "INSERT INTO polls
                 (guild_id, channel_id, author_id, question, options_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                guild_id as i64,
                channel_id as i64,
                author_id as i64,
                question,
                options_json,
                Utc::now().timestamp()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Attach the Discord message that carries the vote buttons.
    pub fn set_poll_message(&self, poll_id: i64, message_id: u64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE polls SET message_id = ?2 WHERE id = ?1",
            params![poll_id, message_id as i64],
        )?;
        Ok(())
    }

    /// Fetch one poll. A corrupt `options_json` column is an error, not `None`.
    pub fn get_poll(&self, poll_id: i64) -> Result<Option<Poll>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT id, guild_id, channel_id, message_id, author_id, question,
                        options_json, created_at, closed_at
                 FROM polls
                 WHERE id = ?1",
                params![poll_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            id,
            guild_id,
            channel_id,
            message_id,
            author_id,
            question,
            options_json,
            created_at,
            closed_at,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(Poll {
            id,
            guild_id: guild_id as u64,
            channel_id: channel_id as u64,
            message_id: message_id.map(|id| id as u64),
            author_id: author_id as u64,
            question,
            options: parse_options(poll_id, &options_json)?,
            created_at,
            closed_at,
        }))
    }

    /// Record (or switch) one member's vote. Re-voting the same option is a no-op.
    pub fn vote_poll(&self, poll_id: i64, user_id: u64, choice: usize) -> Result<PollVoteOutcome> {
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;

        let row = tx
            .query_row(
                "SELECT options_json, closed_at FROM polls WHERE id = ?1",
                params![poll_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        let Some((options_json, closed_at)) = row else {
            anyhow::bail!("Unknown poll #{poll_id}.");
        };
        if closed_at.is_some() {
            return Ok(PollVoteOutcome::Closed);
        }

        let option_count = parse_options(poll_id, &options_json)?.len();
        if choice >= option_count {
            return Ok(PollVoteOutcome::InvalidChoice);
        }

        let previous: Option<i64> = tx
            .query_row(
                "SELECT choice FROM poll_votes WHERE poll_id = ?1 AND user_id = ?2",
                params![poll_id, user_id as i64],
                |row| row.get(0),
            )
            .optional()?;

        let outcome = match previous {
            Some(old) if old == choice as i64 => PollVoteOutcome::Voted,
            Some(_) => {
                tx.execute(
                    "UPDATE poll_votes SET choice = ?3, created_at = ?4
                     WHERE poll_id = ?1 AND user_id = ?2",
                    params![
                        poll_id,
                        user_id as i64,
                        choice as i64,
                        Utc::now().timestamp()
                    ],
                )?;
                PollVoteOutcome::Changed
            }
            None => {
                tx.execute(
                    "INSERT INTO poll_votes (poll_id, user_id, choice, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        poll_id,
                        user_id as i64,
                        choice as i64,
                        Utc::now().timestamp()
                    ],
                )?;
                PollVoteOutcome::Voted
            }
        };

        tx.commit()?;
        Ok(outcome)
    }

    /// Vote counts per option, in declaration order (length = option count).
    pub fn poll_counts(&self, poll_id: i64) -> Result<Vec<i64>> {
        let conn = self.conn();
        let options_json: String = conn
            .query_row(
                "SELECT options_json FROM polls WHERE id = ?1",
                params![poll_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("Unknown poll #{poll_id}."))?;

        let mut counts = vec![0i64; parse_options(poll_id, &options_json)?.len()];
        let mut stmt = conn.prepare(
            "SELECT choice, COUNT(*) FROM poll_votes WHERE poll_id = ?1 GROUP BY choice",
        )?;
        let rows = stmt.query_map(params![poll_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (choice, count) = row?;
            if let Ok(index) = usize::try_from(choice)
                && let Some(slot) = counts.get_mut(index)
            {
                *slot = count;
            }
        }

        Ok(counts)
    }

    /// Sum of every vote on the poll.
    pub fn total_votes(&self, poll_id: i64) -> Result<i64> {
        let conn = self.conn();
        let total = conn.query_row(
            "SELECT COUNT(*) FROM poll_votes WHERE poll_id = ?1",
            params![poll_id],
            |row| row.get(0),
        )?;
        Ok(total)
    }

    /// Close a poll. Idempotent: returns `true` only when it was still open.
    pub fn close_poll(&self, poll_id: i64) -> Result<bool> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE polls SET closed_at = ?2 WHERE id = ?1 AND closed_at IS NULL",
            params![poll_id, Utc::now().timestamp()],
        )?;
        Ok(changed > 0)
    }
}

fn parse_options(poll_id: i64, options_json: &str) -> Result<Vec<String>> {
    serde_json::from_str(options_json)
        .with_context(|| format!("parsing options of poll #{poll_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("poll.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    fn options() -> Vec<String> {
        vec![
            "nmap".to_owned(),
            "rustscan".to_owned(),
            "masscan".to_owned(),
        ]
    }

    fn new_poll(db: &Database) -> i64 {
        db.create_poll(1, 2, 3, "Favourite scanner?", &options())
            .expect("create")
    }

    #[test]
    fn create_and_get_roundtrip() {
        let (_dir, db) = temp_db();
        let id = new_poll(&db);

        let poll = db.get_poll(id).expect("get").expect("exists");
        assert_eq!(poll.id, id);
        assert_eq!(poll.guild_id, 1);
        assert_eq!(poll.channel_id, 2);
        assert_eq!(poll.author_id, 3);
        assert_eq!(poll.question, "Favourite scanner?");
        assert_eq!(poll.options, options());
        assert!(poll.message_id.is_none());
        assert!(poll.closed_at.is_none());
        assert!(poll.created_at > 0);

        assert!(db.get_poll(id + 1).expect("missing").is_none());
    }

    #[test]
    fn message_id_is_saved() {
        let (_dir, db) = temp_db();
        let id = new_poll(&db);
        db.set_poll_message(id, 777).expect("set");
        let poll = db.get_poll(id).expect("get").expect("exists");
        assert_eq!(poll.message_id, Some(777));
    }

    #[test]
    fn create_rejects_bad_input() {
        let (_dir, db) = temp_db();
        let two = vec!["a".to_owned(), "b".to_owned()];

        assert!(db.create_poll(1, 2, 3, "", &two).is_err());
        assert!(db.create_poll(1, 2, 3, "   ", &two).is_err());
        assert!(
            db.create_poll(1, 2, 3, &"q".repeat(MAX_QUESTION_CHARS + 1), &two)
                .is_err()
        );
        assert!(db.create_poll(1, 2, 3, "q", &[]).is_err());
        assert!(db.create_poll(1, 2, 3, "q", &["only".to_owned()]).is_err());
        let six = ["1", "2", "3", "4", "5", "6"].map(str::to_owned);
        assert!(db.create_poll(1, 2, 3, "q", &six).is_err());
        assert!(
            db.create_poll(1, 2, 3, "q", &["ok".to_owned(), "   ".to_owned()])
                .is_err()
        );
        assert!(
            db.create_poll(
                1,
                2,
                3,
                "q",
                &["ok".to_owned(), "x".repeat(MAX_OPTION_CHARS + 1)]
            )
            .is_err()
        );

        // Trimmed but valid input is stored trimmed.
        let id = db
            .create_poll(1, 2, 3, "  padded  ", &[" a ".to_owned(), "b".to_owned()])
            .expect("create");
        let poll = db.get_poll(id).expect("get").expect("exists");
        assert_eq!(poll.question, "padded");
        assert_eq!(poll.options, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn votes_are_single_choice_and_ordered() {
        let (_dir, db) = temp_db();
        let id = new_poll(&db);

        assert_eq!(
            db.vote_poll(id, 10, 1).expect("vote"),
            PollVoteOutcome::Voted
        );
        assert_eq!(
            db.vote_poll(id, 11, 1).expect("vote"),
            PollVoteOutcome::Voted
        );
        assert_eq!(
            db.vote_poll(id, 12, 0).expect("vote"),
            PollVoteOutcome::Voted
        );

        assert_eq!(db.poll_counts(id).expect("counts"), vec![1, 2, 0]);
        assert_eq!(db.total_votes(id).expect("total"), 3);

        // Same choice again: still one vote, outcome Voted.
        assert_eq!(
            db.vote_poll(id, 10, 1).expect("repeat"),
            PollVoteOutcome::Voted
        );
        assert_eq!(db.poll_counts(id).expect("counts"), vec![1, 2, 0]);
        assert_eq!(db.total_votes(id).expect("total"), 3);

        // Switching moves the vote, never adds a second row.
        assert_eq!(
            db.vote_poll(id, 10, 0).expect("switch"),
            PollVoteOutcome::Changed
        );
        assert_eq!(db.poll_counts(id).expect("counts"), vec![2, 1, 0]);
        assert_eq!(db.total_votes(id).expect("total"), 3);
    }

    #[test]
    fn invalid_choice_is_rejected() {
        let (_dir, db) = temp_db();
        let id = new_poll(&db);

        assert_eq!(
            db.vote_poll(id, 10, 3).expect("out of range"),
            PollVoteOutcome::InvalidChoice
        );
        assert_eq!(
            db.vote_poll(id, 10, usize::MAX).expect("out of range"),
            PollVoteOutcome::InvalidChoice
        );
        assert_eq!(db.poll_counts(id).expect("counts"), vec![0, 0, 0]);
        assert_eq!(db.total_votes(id).expect("total"), 0);
    }

    #[test]
    fn closed_polls_reject_votes_and_close_is_idempotent() {
        let (_dir, db) = temp_db();
        let id = new_poll(&db);
        db.vote_poll(id, 10, 0).expect("vote");

        assert!(db.close_poll(id).expect("close"));
        assert!(!db.close_poll(id).expect("close twice"));

        let poll = db.get_poll(id).expect("get").expect("exists");
        assert!(poll.closed_at.is_some());

        assert_eq!(
            db.vote_poll(id, 11, 1).expect("closed vote"),
            PollVoteOutcome::Closed
        );
        assert_eq!(db.poll_counts(id).expect("counts"), vec![1, 0, 0]);
        assert_eq!(db.total_votes(id).expect("total"), 1);

        assert!(!db.close_poll(id + 1).expect("unknown poll"));
    }

    #[test]
    fn unknown_polls_error() {
        let (_dir, db) = temp_db();
        assert!(db.vote_poll(999, 10, 0).is_err());
        assert!(db.poll_counts(999).is_err());
        assert_eq!(db.total_votes(999).expect("total"), 0);
    }

    #[test]
    fn corrupt_options_json_is_an_error() {
        let (_dir, db) = temp_db();
        let id = new_poll(&db);
        db.conn()
            .execute(
                "UPDATE polls SET options_json = 'not json' WHERE id = ?1",
                params![id],
            )
            .expect("corrupt");

        assert!(db.get_poll(id).is_err());
        assert!(db.poll_counts(id).is_err());
        assert!(db.vote_poll(id, 10, 0).is_err());
    }
}
