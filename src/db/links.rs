//! Legacy link pipeline persistence (tracked messages + last thread per channel).
//!
//! These methods ship with the original link workflow and are frozen: the
//! message TTL is stored as an RFC3339 `TEXT` value in `tracked_messages`.
//! The `/links` archive table (`links`) is owned by the community lane.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use super::Database;

/// One row of the `/links` community archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedLink {
    pub url: String,
    pub domain: String,
    pub channel_id: u64,
    pub message_id: u64,
    pub thread_id: Option<u64>,
    pub author_id: u64,
    pub posted_at: i64,
}

/// Escape `%`, `_` and `\` for a SQL `LIKE ... ESCAPE '\'` pattern.
fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    for ch in raw.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

impl Database {
    /// Track a message for deletion after `ttl_hours`.
    pub fn track_message(&self, message_id: u64, channel_id: u64, ttl_hours: u64) -> Result<()> {
        let delete_at = Utc::now() + chrono::Duration::hours(ttl_hours as i64);
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO tracked_messages (message_id, channel_id, delete_at)
             VALUES (?1, ?2, ?3)",
            params![message_id as i64, channel_id as i64, delete_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// `(message_id, channel_id)` pairs whose TTL has elapsed.
    pub fn get_expired_messages(&self) -> Result<Vec<(u64, u64)>> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT message_id, channel_id FROM tracked_messages WHERE delete_at <= ?1")?;
        let rows = stmt.query_map(params![now], |row| {
            Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    pub fn remove_message(&self, message_id: u64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM tracked_messages WHERE message_id = ?1",
            params![message_id as i64],
        )?;
        Ok(())
    }

    pub fn set_last_thread(&self, channel_id: u64, thread_id: u64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO channel_threads (channel_id, thread_id) VALUES (?1, ?2)",
            params![channel_id as i64, thread_id as i64],
        )?;
        Ok(())
    }

    pub fn get_last_thread(&self, channel_id: u64) -> Result<Option<u64>> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT thread_id FROM channel_threads WHERE channel_id = ?1",
            params![channel_id as i64],
            |row| row.get::<_, i64>(0),
        );
        match result {
            Ok(id) => Ok(Some(id as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Forget a stale thread mapping (thread deleted or inaccessible).
    pub fn clear_last_thread(&self, channel_id: u64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM channel_threads WHERE channel_id = ?1",
            params![channel_id as i64],
        )?;
        Ok(())
    }

    /// Number of messages currently queued for TTL deletion.
    pub fn tracked_message_count(&self) -> Result<u64> {
        let conn = self.conn();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM tracked_messages", [], |row| {
            row.get(0)
        })?;
        Ok(count as u64)
    }
}

impl Database {
    /// Persist one archived link; returns the archive row id.
    #[allow(clippy::too_many_arguments)]
    pub fn archive_link(
        &self,
        guild_id: u64,
        channel_id: u64,
        message_id: u64,
        thread_id: Option<u64>,
        author_id: u64,
        url: &str,
        domain: &str,
        posted_at: i64,
    ) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO links
                 (guild_id, channel_id, message_id, thread_id, author_id, url, domain, posted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                guild_id as i64,
                channel_id as i64,
                message_id as i64,
                thread_id.map(|id| id as i64),
                author_id as i64,
                url,
                domain,
                posted_at
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Oldest archived duplicate of `url` that has a thread:
    /// `(thread_id, message_id, channel_id)`.
    pub fn find_duplicate_link(&self, guild_id: u64, url: &str) -> Result<Option<(u64, u64, u64)>> {
        let conn = self.conn();
        let found = conn
            .query_row(
                "SELECT thread_id, message_id, channel_id FROM links
                 WHERE guild_id = ?1 AND url = ?2 AND thread_id IS NOT NULL
                 ORDER BY posted_at ASC, id ASC
                 LIMIT 1",
                params![guild_id as i64, url],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                    ))
                },
            )
            .optional()?;
        Ok(found)
    }

    /// Case-insensitive substring search over url and domain (max 25 rows).
    pub fn search_links(
        &self,
        guild_id: u64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ArchivedLink>> {
        let limit = limit.clamp(1, 25) as i64;
        let pattern = format!("%{}%", escape_like(query.trim()));
        let conn = self.conn();
        let mut stmt = conn.prepare(
            r"SELECT url, domain, channel_id, message_id, thread_id, author_id, posted_at
              FROM links
              WHERE guild_id = ?1
                AND (url LIKE ?2 ESCAPE '\' OR domain LIKE ?2 ESCAPE '\')
              ORDER BY posted_at DESC, id DESC
              LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, pattern, limit], map_archived)?;
        let mut links = Vec::new();
        for row in rows {
            links.push(row?);
        }
        Ok(links)
    }

    /// Most recent archived links, newest first.
    pub fn recent_links(&self, guild_id: u64, limit: usize) -> Result<Vec<ArchivedLink>> {
        let limit = limit.clamp(1, 25) as i64;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT url, domain, channel_id, message_id, thread_id, author_id, posted_at
             FROM links
             WHERE guild_id = ?1
             ORDER BY posted_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, limit], map_archived)?;
        let mut links = Vec::new();
        for row in rows {
            links.push(row?);
        }
        Ok(links)
    }

    /// `(total links, distinct authors, top domain, top domain count)`.
    pub fn link_stats(&self, guild_id: u64) -> Result<(u64, u64, String, u64)> {
        let conn = self.conn();
        let (total, authors): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT author_id) FROM links WHERE guild_id = ?1",
            params![guild_id as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let top: Option<(String, i64)> = conn
            .query_row(
                "SELECT domain, COUNT(*) AS hits FROM links
                 WHERE guild_id = ?1
                 GROUP BY domain
                 ORDER BY hits DESC, domain ASC
                 LIMIT 1",
                params![guild_id as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let (domain, domain_count) = top
            .map(|(domain, count)| (domain, count as u64))
            .unwrap_or_else(|| ("—".to_owned(), 0));

        Ok((total as u64, authors as u64, domain, domain_count))
    }
}

fn map_archived(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchivedLink> {
    Ok(ArchivedLink {
        url: row.get(0)?,
        domain: row.get(1)?,
        channel_id: row.get::<_, i64>(2)? as u64,
        message_id: row.get::<_, i64>(3)? as u64,
        thread_id: row.get::<_, Option<i64>>(4)?.map(|id| id as u64),
        author_id: row.get::<_, i64>(5)? as u64,
        posted_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("links.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn legacy_link_methods_roundtrip() {
        let (_dir, db) = temp_db();

        db.track_message(10, 20, 0).expect("track");
        let expired = db.get_expired_messages().expect("expired");
        assert!(expired.contains(&(10, 20)));

        db.track_message(11, 21, 24).expect("track future");
        let expired = db.get_expired_messages().expect("expired");
        assert!(!expired.iter().any(|(id, _)| *id == 11));

        db.remove_message(10).expect("remove");
        let expired = db.get_expired_messages().expect("expired");
        assert!(!expired.iter().any(|(id, _)| *id == 10));

        assert!(db.get_last_thread(30).expect("none").is_none());
        db.set_last_thread(30, 99).expect("set");
        assert_eq!(db.get_last_thread(30).expect("get"), Some(99));
        db.set_last_thread(30, 100).expect("overwrite");
        assert_eq!(db.get_last_thread(30).expect("get"), Some(100));
        db.clear_last_thread(30).expect("clear");
        assert_eq!(db.get_last_thread(30).expect("cleared"), None);
    }

    fn archive(db: &Database, url: &str, thread: Option<u64>, posted_at: i64) {
        let domain = crate::util::text::domain_of(url).unwrap_or_else(|| "unknown".to_owned());
        db.archive_link(
            1,
            10,
            100 + posted_at as u64,
            thread,
            5,
            url,
            &domain,
            posted_at,
        )
        .expect("archive");
    }

    #[test]
    fn archive_search_escapes_like_wildcards() {
        let (_dir, db) = temp_db();
        archive(&db, "https://example.com/100%_test", Some(900), 100);
        archive(&db, "https://example.com/other", Some(901), 200);

        let exact = db.search_links(1, "100%_test", 25).expect("search");
        assert_eq!(exact.len(), 1);
        assert!(exact[0].url.contains("100%_test"));

        // `%` and `_` must be literal, not SQL wildcards.
        assert!(
            db.search_links(1, "100Xtest", 25)
                .expect("search")
                .is_empty()
        );
        assert!(
            db.search_links(1, "example.com/other", 25)
                .expect("search")
                .len()
                == 1
        );

        // Backslash is escaped too.
        archive(&db, "https://example.com/a\\b", None, 300);
        let literal = db.search_links(1, "a\\b", 25).expect("search");
        assert_eq!(literal.len(), 1);
    }

    #[test]
    fn duplicate_lookup_returns_oldest_threaded_match() {
        let (_dir, db) = temp_db();
        archive(&db, "https://example.com/x", None, 10);
        archive(&db, "https://example.com/x", Some(500), 20);
        archive(&db, "https://example.com/x", Some(600), 30);

        assert_eq!(
            db.find_duplicate_link(1, "https://example.com/x")
                .expect("lookup"),
            Some((500, 120, 10))
        );
        assert!(
            db.find_duplicate_link(1, "https://example.com/missing")
                .expect("lookup")
                .is_none()
        );
    }

    #[test]
    fn recent_links_are_newest_first() {
        let (_dir, db) = temp_db();
        archive(&db, "https://a.example/1", Some(1), 10);
        archive(&db, "https://b.example/2", Some(2), 30);
        archive(&db, "https://c.example/3", None, 20);

        let recent = db.recent_links(1, 2).expect("recent");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].url, "https://b.example/2");
        assert_eq!(recent[1].url, "https://c.example/3");
        assert_eq!(recent[1].thread_id, None);
    }

    #[test]
    fn stats_count_authors_and_top_domain() {
        let (_dir, db) = temp_db();
        assert_eq!(db.link_stats(1).expect("empty"), (0, 0, "—".to_owned(), 0));

        archive(&db, "https://a.example/1", Some(1), 10);
        archive(&db, "https://a.example/2", Some(2), 20);
        db.archive_link(1, 11, 300, None, 6, "https://b.example/x", "b.example", 30)
            .expect("archive");

        let (total, authors, domain, count) = db.link_stats(1).expect("stats");
        assert_eq!((total, authors), (3, 2));
        assert_eq!(domain, "a.example");
        assert_eq!(count, 2);
    }
}
