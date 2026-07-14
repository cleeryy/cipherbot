use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        tracing::info!("Opening database at '{}'", path);
        let conn = Connection::open(path)?;
        tracing::info!("Database opened successfully at '{}'", path);
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn initialize(&self) -> anyhow::Result<()> {
        tracing::debug!("Initializing database schema...");
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracked_messages (
                message_id INTEGER PRIMARY KEY,
                channel_id INTEGER NOT NULL,
                delete_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tracked_delete_at
                ON tracked_messages(delete_at);
            CREATE TABLE IF NOT EXISTS channel_threads (
                channel_id INTEGER PRIMARY KEY,
                thread_id INTEGER NOT NULL
            );",
        )?;
        tracing::debug!("Database schema initialized");
        Ok(())
    }

    #[allow(dead_code)]
    pub fn track_message(
        &self,
        message_id: u64,
        channel_id: u64,
        ttl_hours: u64,
    ) -> anyhow::Result<()> {
        let delete_at = Utc::now() + chrono::Duration::hours(ttl_hours as i64);
        tracing::debug!(
            "DB: tracking message {} in channel {}, will delete at {}",
            message_id,
            channel_id,
            delete_at.to_rfc3339()
        );
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO tracked_messages (message_id, channel_id, delete_at) \
             VALUES (?1, ?2, ?3)",
            params![message_id as i64, channel_id as i64, delete_at.to_rfc3339()],
        )?;
        tracing::debug!("DB: message {} tracked successfully", message_id);
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
        tracing::debug!("DB: found {} expired message(s) to delete", messages.len());
        Ok(messages)
    }

    pub fn remove_message(&self, message_id: u64) -> anyhow::Result<()> {
        tracing::debug!("DB: removing message {} from tracking", message_id);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tracked_messages WHERE message_id = ?1",
            params![message_id as i64],
        )?;
        tracing::debug!("DB: message {} removed from tracking", message_id);
        Ok(())
    }

    pub fn set_last_thread(&self, channel_id: u64, thread_id: u64) -> anyhow::Result<()> {
        tracing::debug!("DB: setting last thread for channel {} to {}", channel_id, thread_id);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO channel_threads (channel_id, thread_id) VALUES (?1, ?2)",
            params![channel_id as i64, thread_id as i64],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_last_thread(&self, channel_id: u64) -> anyhow::Result<Option<u64>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT thread_id FROM channel_threads WHERE channel_id = ?1",
            params![channel_id as i64],
            |row| row.get::<_, i64>(0),
        );
        match result {
            Ok(id) => Ok(Some(id as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn load_all_last_threads(&self) -> anyhow::Result<Vec<(u64, u64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT channel_id, thread_id FROM channel_threads")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        tracing::debug!("DB: loaded {} last_thread entries", result.len());
        Ok(result)
    }

    #[allow(dead_code)]
    pub fn message_count(&self) -> anyhow::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: u64 = conn
            .query_row("SELECT COUNT(*) FROM tracked_messages", [], |row| {
                row.get(0)
            })?;
        tracing::debug!("DB: total tracked messages: {}", count);
        Ok(count)
    }
}
