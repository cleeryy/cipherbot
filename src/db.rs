use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn initialize(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracked_messages (
                message_id INTEGER PRIMARY KEY,
                channel_id INTEGER NOT NULL,
                delete_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tracked_delete_at
                ON tracked_messages(delete_at);",
        )?;
        Ok(())
    }

    pub fn track_message(
        &self,
        message_id: u64,
        channel_id: u64,
        ttl_hours: u64,
    ) -> anyhow::Result<()> {
        let delete_at = Utc::now() + chrono::Duration::hours(ttl_hours as i64);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO tracked_messages (message_id, channel_id, delete_at) \
             VALUES (?1, ?2, ?3)",
            params![message_id as i64, channel_id as i64, delete_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn get_expired_messages(&self) -> anyhow::Result<Vec<(u64, u64)>> {
        let now = Utc::now();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT message_id, channel_id FROM tracked_messages WHERE delete_at <= ?1",
        )?;
        let rows = stmt.query_map(params![now.to_rfc3339()], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
            ))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    pub fn remove_message(&self, message_id: u64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tracked_messages WHERE message_id = ?1",
            params![message_id as i64],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn message_count(&self) -> anyhow::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: u64 = conn
            .query_row("SELECT COUNT(*) FROM tracked_messages", [], |row| {
                row.get(0)
            })?;
        Ok(count)
    }
}
