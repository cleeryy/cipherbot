//! Per-user XP / message / quiz counters (`users` table).
//!
//! Owned by the community lane. One lock per public method, owned data out.
//! Message XP is throttled in-process (one point per member per minute) so a
//! chatty channel cannot be farmed; the message counter itself always advances.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use parking_lot::Mutex;
use rusqlite::{OptionalExtension, params};
use serenity::all::{Context, Message};

use super::Database;
use crate::bot::SharedState;
use crate::handler::HookResult;

/// One quiz submission result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuizOutcome {
    pub already_answered: bool,
    pub streak: i64,
    pub xp_awarded: u32,
}

/// Aggregated profile for one member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserProfile {
    pub xp: i64,
    pub messages: i64,
    pub quiz_correct: i64,
    pub quiz_total: i64,
    pub streak: i64,
    pub ctf_points: i64,
    pub first_seen: i64,
}

/// Level derived from total XP: `floor(sqrt(xp / 50)) + 1`.
pub fn level_for_xp(xp: i64) -> i64 {
    let xp = xp.max(0);
    ((xp / 50) as f64).sqrt().floor() as i64 + 1
}

/// First XP value of `level` (inverse of [`level_for_xp`]).
pub fn xp_for_level(level: i64) -> i64 {
    let level = level.max(1) - 1;
    level * level * 50
}

fn xp_throttle() -> &'static Mutex<HashMap<(u64, u64), Instant>> {
    static THROTTLE: OnceLock<Mutex<HashMap<(u64, u64), Instant>>> = OnceLock::new();
    THROTTLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reserve the XP slot for `(guild, user)`; true when at least a minute elapsed.
fn take_xp_slot(guild_id: u64, user_id: u64) -> bool {
    const WINDOW: Duration = Duration::from_secs(60);
    let mut map = xp_throttle().lock();
    let now = Instant::now();
    let allowed = map
        .get(&(guild_id, user_id))
        .map(|last| now.saturating_duration_since(*last) >= WINDOW)
        .unwrap_or(true);

    if allowed {
        map.insert((guild_id, user_id), now);
    }
    if map.len() > 4_096 {
        map.retain(|_, last| now.saturating_duration_since(*last) < Duration::from_secs(600));
    }
    allowed
}

/// Previous UTC day for a `YYYY-MM-DD` string.
fn previous_day(day: &str) -> Option<String> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.pred_opt())
        .map(|date| date.format("%Y-%m-%d").to_string())
}

impl Database {
    /// Create the row if the member was never seen.
    pub fn ensure_user(&self, guild_id: u64, user_id: u64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO users (guild_id, user_id, first_seen)
             VALUES (?1, ?2, ?3)",
            params![guild_id as i64, user_id as i64, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Flat XP grant (rewards, manual grants).
    pub fn add_xp(&self, guild_id: u64, user_id: u64, amount: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO users (guild_id, user_id, xp, first_seen)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(guild_id, user_id) DO UPDATE SET xp = xp + excluded.xp",
            params![
                guild_id as i64,
                user_id as i64,
                amount.max(0),
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// Count one message and (at most once a minute) grant one XP.
    pub fn record_message(&self, guild_id: u64, user_id: u64) -> Result<()> {
        let xp = i64::from(take_xp_slot(guild_id, user_id));
        let conn = self.conn();
        conn.execute(
            "INSERT INTO users (guild_id, user_id, xp, messages, first_seen)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(guild_id, user_id) DO UPDATE SET
                 messages = messages + 1,
                 xp = xp + excluded.xp",
            params![guild_id as i64, user_id as i64, xp, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Record one quiz answer, enforcing one answer per UTC day.
    pub fn record_quiz_answer(
        &self,
        guild_id: u64,
        user_id: u64,
        correct: bool,
        day: &str,
    ) -> Result<QuizOutcome> {
        let day = day.trim();
        anyhow::ensure!(!day.is_empty(), "quiz day must not be empty");

        let conn = self.conn();
        let existing: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT streak, last_quiz_day FROM users
                 WHERE guild_id = ?1 AND user_id = ?2",
                params![guild_id as i64, user_id as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((streak, Some(last))) = &existing
            && last == day
        {
            return Ok(QuizOutcome {
                already_answered: true,
                streak: *streak,
                xp_awarded: 0,
            });
        }

        let previous_streak = existing.as_ref().map(|(streak, _)| *streak).unwrap_or(0);
        let continues = matches!(
            &existing,
            Some((_, Some(last))) if previous_day(day).as_deref() == Some(last.as_str())
        );
        let streak = if continues { previous_streak + 1 } else { 1 };
        let xp_awarded: u32 = if correct { 10 } else { 2 };

        conn.execute(
            "INSERT INTO users
                 (guild_id, user_id, xp, quiz_correct, quiz_total, streak, last_quiz_day, first_seen)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7)
             ON CONFLICT(guild_id, user_id) DO UPDATE SET
                 xp = xp + excluded.xp,
                 quiz_correct = quiz_correct + excluded.quiz_correct,
                 quiz_total = quiz_total + 1,
                 streak = excluded.streak,
                 last_quiz_day = excluded.last_quiz_day",
            params![
                guild_id as i64,
                user_id as i64,
                i64::from(xp_awarded),
                i64::from(correct),
                streak,
                day,
                Utc::now().timestamp()
            ],
        )?;

        Ok(QuizOutcome {
            already_answered: false,
            streak,
            xp_awarded,
        })
    }

    /// Profile counters plus CTF points read straight from `ctf_solves`.
    pub fn get_profile(&self, guild_id: u64, user_id: u64) -> Result<UserProfile> {
        let conn = self.conn();
        let profile: Option<UserProfile> = conn
            .query_row(
                "SELECT xp, messages, quiz_correct, quiz_total, streak, first_seen
                 FROM users WHERE guild_id = ?1 AND user_id = ?2",
                params![guild_id as i64, user_id as i64],
                |row| {
                    Ok(UserProfile {
                        xp: row.get(0)?,
                        messages: row.get(1)?,
                        quiz_correct: row.get(2)?,
                        quiz_total: row.get(3)?,
                        streak: row.get(4)?,
                        ctf_points: 0,
                        first_seen: row.get(5)?,
                    })
                },
            )
            .optional()?;

        let ctf_points: i64 = conn.query_row(
            "SELECT COALESCE(SUM(points), 0) FROM ctf_solves
             WHERE guild_id = ?1 AND user_id = ?2",
            params![guild_id as i64, user_id as i64],
            |row| row.get(0),
        )?;

        let mut profile = profile.unwrap_or_default();
        profile.ctf_points = ctf_points;
        Ok(profile)
    }

    /// `(user_id, xp)` ordered by XP desc, then oldest account id.
    pub fn leaderboard_xp(&self, guild_id: u64, limit: usize) -> Result<Vec<(u64, i64)>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT user_id, xp FROM users
             WHERE guild_id = ?1 AND xp > 0
             ORDER BY xp DESC, user_id ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, limit], |row| {
            Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)?))
        })?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        Ok(values)
    }

    /// `(user_id, correct, total)` for members with at least five answers.
    pub fn leaderboard_quiz(&self, guild_id: u64, limit: usize) -> Result<Vec<(u64, i64, i64)>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT user_id, quiz_correct, quiz_total FROM users
             WHERE guild_id = ?1 AND quiz_total >= 5
             ORDER BY (CAST(quiz_correct AS REAL) / quiz_total) DESC,
                      quiz_correct DESC, quiz_total ASC, user_id ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, limit], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        Ok(values)
    }
}

/// Message pipeline hook: count the message and grant throttled XP.
///
/// Never fails the pipeline and never stops message processing.
pub async fn on_message(state: &SharedState, _ctx: &Context, msg: &Message) -> HookResult {
    if msg.author.bot {
        return HookResult::Continue;
    }
    let Some(guild_id) = msg.guild_id else {
        return HookResult::Continue;
    };

    if let Err(error) = state.db.record_message(guild_id.get(), msg.author.id.get()) {
        tracing::debug!("Failed to record XP for {}: {error}", msg.author.id);
    }
    HookResult::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("users.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn ensure_user_is_idempotent() {
        let (_dir, db) = temp_db();
        db.ensure_user(1, 2).expect("first");
        db.ensure_user(1, 2).expect("second");

        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM users WHERE guild_id = 1 AND user_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn level_thresholds_roundtrip() {
        assert_eq!(level_for_xp(0), 1);
        assert_eq!(level_for_xp(49), 1);
        assert_eq!(level_for_xp(50), 2);
        assert_eq!(level_for_xp(199), 2);
        assert_eq!(level_for_xp(200), 3);
        assert_eq!(level_for_xp(-10), 1);

        for level in 1..=20 {
            let xp = xp_for_level(level);
            assert_eq!(level_for_xp(xp), level, "level {level} at {xp} xp");
            if level > 1 {
                assert_eq!(level_for_xp(xp - 1), level - 1);
            }
        }
    }

    #[test]
    fn quiz_is_once_per_day_and_tracks_streaks() {
        let (_dir, db) = temp_db();
        let (guild, user) = (10, 20);

        let first = db
            .record_quiz_answer(guild, user, true, "2026-10-01")
            .expect("first answer");
        assert!(!first.already_answered);
        assert_eq!(first.streak, 1);
        assert_eq!(first.xp_awarded, 10);

        let again = db
            .record_quiz_answer(guild, user, false, "2026-10-01")
            .expect("repeat");
        assert!(again.already_answered);
        assert_eq!(again.xp_awarded, 0);

        let next = db
            .record_quiz_answer(guild, user, false, "2026-10-02")
            .expect("next day");
        assert!(!next.already_answered);
        assert_eq!(next.streak, 2);
        assert_eq!(next.xp_awarded, 2);

        let after_gap = db
            .record_quiz_answer(guild, user, true, "2026-10-05")
            .expect("after gap");
        assert_eq!(after_gap.streak, 1);
        assert_eq!(after_gap.xp_awarded, 10);

        let profile = db.get_profile(guild, user).expect("profile");
        assert_eq!(profile.quiz_total, 3);
        assert_eq!(profile.quiz_correct, 2);
        assert_eq!(profile.streak, 1);
        assert_eq!(profile.xp, 22);
        assert!(profile.first_seen > 0);
    }

    #[test]
    fn xp_leaderboard_is_ordered() {
        let (_dir, db) = temp_db();
        db.add_xp(1, 100, 50).expect("a");
        db.add_xp(1, 101, 50).expect("b");
        db.add_xp(1, 102, 250).expect("c");
        db.add_xp(1, 103, 0).expect("zero");
        db.add_xp(2, 999, 10_000).expect("other guild");

        let board = db.leaderboard_xp(1, 10).expect("board");
        assert_eq!(board, vec![(102, 250), (100, 50), (101, 50)]);
    }

    #[test]
    fn quiz_leaderboard_requires_five_answers() {
        let (_dir, db) = temp_db();
        for day in [
            "2026-01-01",
            "2026-01-02",
            "2026-01-03",
            "2026-01-04",
            "2026-01-05",
        ] {
            db.record_quiz_answer(1, 100, day != "2026-01-03", day)
                .expect("answers");
        }
        db.record_quiz_answer(1, 101, true, "2026-01-01")
            .expect("short history");

        let board = db.leaderboard_quiz(1, 10).expect("board");
        assert_eq!(board, vec![(100, 4, 5)]);
    }

    #[test]
    fn record_message_counts_every_message_but_throttles_xp() {
        let (_dir, db) = temp_db();
        db.record_message(7, 8).expect("first");
        db.record_message(7, 8).expect("second");

        let profile = db.get_profile(7, 8).expect("profile");
        assert_eq!(profile.messages, 2);
        assert_eq!(profile.xp, 1);
    }
}
