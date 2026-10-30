//! Ticket persistence (owned by the ticket lane).
//!
//! One row per private support thread. Rows are written by the `ticket:open`
//! component and flipped to `closed` by `ticket:close` / `/ticket close`; the
//! thread id is the primary key because Discord threads are the ticket itself.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use super::Database;

/// One support ticket, keyed by its Discord thread id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub thread_id: u64,
    pub guild_id: u64,
    pub channel_id: u64,
    pub user_id: u64,
    pub status: String,
    pub created_at: i64,
    pub closed_by: Option<u64>,
    pub closed_at: Option<i64>,
}

const COLUMNS: &str =
    "thread_id, guild_id, channel_id, user_id, status, created_at, closed_by, closed_at";

fn map_ticket(row: &rusqlite::Row<'_>) -> rusqlite::Result<Ticket> {
    Ok(Ticket {
        thread_id: row.get::<_, i64>(0)? as u64,
        guild_id: row.get::<_, i64>(1)? as u64,
        channel_id: row.get::<_, i64>(2)? as u64,
        user_id: row.get::<_, i64>(3)? as u64,
        status: row.get(4)?,
        created_at: row.get(5)?,
        closed_by: row.get::<_, Option<i64>>(6)?.map(|id| id as u64),
        closed_at: row.get(7)?,
    })
}

impl Database {
    /// Record a new open ticket. Fails when the thread id is already known.
    pub fn create_ticket(
        &self,
        guild_id: u64,
        channel_id: u64,
        thread_id: u64,
        user_id: u64,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO tickets
                 (thread_id, guild_id, channel_id, user_id, status, created_at)
             VALUES (?1, ?2, ?3, ?4, 'open', ?5)",
            params![
                thread_id as i64,
                guild_id as i64,
                channel_id as i64,
                user_id as i64,
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// Look a ticket up by its thread id, whatever its status.
    pub fn get_ticket(&self, thread_id: u64) -> Result<Option<Ticket>> {
        let conn = self.conn();
        let sql = format!("SELECT {COLUMNS} FROM tickets WHERE thread_id = ?1");
        let ticket = conn
            .query_row(&sql, params![thread_id as i64], map_ticket)
            .optional()?;
        Ok(ticket)
    }

    /// Newest open ticket opened by `user_id` in `guild_id`, if any.
    pub fn find_open_ticket(&self, guild_id: u64, user_id: u64) -> Result<Option<Ticket>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {COLUMNS} FROM tickets
             WHERE guild_id = ?1 AND user_id = ?2 AND status = 'open'
             ORDER BY created_at DESC, thread_id DESC
             LIMIT 1"
        );
        let ticket = conn
            .query_row(&sql, params![guild_id as i64, user_id as i64], map_ticket)
            .optional()?;
        Ok(ticket)
    }

    /// Mark an open ticket closed. Returns `false` when it was already closed
    /// (or never existed), which makes the operation idempotent.
    pub fn close_ticket(&self, thread_id: u64, closed_by: u64) -> Result<bool> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE tickets
                SET status = 'closed', closed_by = ?1, closed_at = ?2
              WHERE thread_id = ?3 AND status = 'open'",
            params![closed_by as i64, Utc::now().timestamp(), thread_id as i64],
        )?;
        Ok(changed > 0)
    }

    /// Number of open tickets in a guild.
    pub fn open_ticket_count(&self, guild_id: u64) -> Result<u64> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tickets WHERE guild_id = ?1 AND status = 'open'",
            params![guild_id as i64],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Oldest open tickets in a guild, up to `limit` rows.
    pub fn list_open_tickets(&self, guild_id: u64, limit: usize) -> Result<Vec<Ticket>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn();
        let sql = format!(
            "SELECT {COLUMNS} FROM tickets
             WHERE guild_id = ?1 AND status = 'open'
             ORDER BY created_at ASC, thread_id ASC
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![guild_id as i64, limit], map_ticket)?;
        let mut tickets = Vec::new();
        for row in rows {
            tickets.push(row?);
        }
        Ok(tickets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tickets.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn create_and_read_back() {
        let (_dir, db) = temp_db();
        db.create_ticket(10, 20, 30, 40).expect("create");

        let ticket = db.get_ticket(30).expect("get").expect("present");
        assert_eq!(ticket.thread_id, 30);
        assert_eq!(ticket.guild_id, 10);
        assert_eq!(ticket.channel_id, 20);
        assert_eq!(ticket.user_id, 40);
        assert_eq!(ticket.status, "open");
        assert!(ticket.created_at > 0);
        assert_eq!(ticket.closed_by, None);
        assert_eq!(ticket.closed_at, None);
        assert_eq!(db.get_ticket(999).expect("missing"), None);
    }

    #[test]
    fn duplicate_thread_id_is_rejected() {
        let (_dir, db) = temp_db();
        db.create_ticket(1, 2, 3, 4).expect("first");
        assert!(
            db.create_ticket(1, 2, 3, 5).is_err(),
            "thread ids are primary keys"
        );
        assert_eq!(db.open_ticket_count(1).expect("count"), 1);
    }

    #[test]
    fn find_open_ticket_ignores_closed_and_other_guilds() {
        let (_dir, db) = temp_db();
        db.create_ticket(1, 100, 1_000, 42).expect("open");
        db.create_ticket(1, 100, 1_001, 42).expect("closed");
        assert!(db.close_ticket(1_001, 42).expect("close"));
        db.create_ticket(2, 200, 2_000, 42).expect("other guild");

        let found = db.find_open_ticket(1, 42).expect("find").expect("some");
        assert_eq!(found.thread_id, 1_000);
        assert_eq!(db.find_open_ticket(3, 42).expect("none"), None);
        assert_eq!(db.find_open_ticket(1, 7).expect("none"), None);
    }

    #[test]
    fn close_is_idempotent() {
        let (_dir, db) = temp_db();
        db.create_ticket(1, 2, 3, 4).expect("create");

        assert!(db.close_ticket(3, 9).expect("first close"));
        let ticket = db.get_ticket(3).expect("get").expect("present");
        assert_eq!(ticket.status, "closed");
        assert_eq!(ticket.closed_by, Some(9));
        assert!(ticket.closed_at.is_some());

        assert!(!db.close_ticket(3, 10).expect("second close"));
        assert!(!db.close_ticket(404, 10).expect("unknown ticket"));
        assert_eq!(
            db.get_ticket(3).expect("get").expect("present").closed_by,
            Some(9)
        );
    }

    #[test]
    fn counts_and_lists_open_tickets() {
        let (_dir, db) = temp_db();
        db.create_ticket(1, 10, 100, 7).expect("a");
        db.create_ticket(1, 10, 101, 8).expect("b");
        db.create_ticket(2, 20, 200, 9).expect("other guild");
        assert!(db.close_ticket(101, 1).expect("close"));

        assert_eq!(db.open_ticket_count(1).expect("count"), 1);
        assert_eq!(db.open_ticket_count(2).expect("count"), 1);
        assert_eq!(db.open_ticket_count(3).expect("count"), 0);

        let open = db.list_open_tickets(1, 25).expect("list");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].thread_id, 100);
        assert!(db.list_open_tickets(1, 25).expect("list").len() <= 25);
    }
}
