//! Intel response cache (`intel_cache`, `feed_state`).
//!
//! Payloads are opaque JSON strings: the intel services serialise their own
//! structs and deserialise them back, so this module stays dependency-free.
//! A row is only served while `expires_at > now`; expired rows are deleted
//! opportunistically on read and in bulk by [`Database::purge_expired_intel_cache`].

use anyhow::{Context as _, Result};
use rusqlite::OptionalExtension as _;

use super::Database;
use crate::util::time::unix_now;

impl Database {
    /// Fetch a cached payload, evicting the row when it has expired.
    pub fn cache_get(&self, key: &str) -> Result<Option<String>> {
        let now = unix_now();
        let conn = self.conn();
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT payload, expires_at FROM intel_cache WHERE cache_key = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("reading intel cache")?;

        match row {
            Some((payload, expires_at)) if expires_at > now => Ok(Some(payload)),
            Some(_) => {
                conn.execute("DELETE FROM intel_cache WHERE cache_key = ?1", [key])
                    .context("evicting expired intel cache row")?;
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Insert or refresh a payload with a TTL in seconds.
    ///
    /// A zero or negative TTL stores an already-expired row, which is useful in
    /// tests and for "do not cache" call sites.
    pub fn cache_put(&self, key: &str, payload: &str, ttl_secs: i64) -> Result<()> {
        let now = unix_now();
        let expires_at = now.saturating_add(ttl_secs);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO intel_cache (cache_key, payload, fetched_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(cache_key) DO UPDATE SET
                 payload = excluded.payload,
                 fetched_at = excluded.fetched_at,
                 expires_at = excluded.expires_at",
            rusqlite::params![key, payload, now, expires_at],
        )
        .context("writing intel cache")?;
        Ok(())
    }

    /// Delete every expired row. Returns the number of rows removed.
    pub fn purge_expired_intel_cache(&self) -> Result<usize> {
        let now = unix_now();
        let conn = self.conn();
        let removed = conn
            .execute("DELETE FROM intel_cache WHERE expires_at <= ?1", [now])
            .context("purging expired intel cache")?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        let db = Database::open(":memory:").expect("in-memory db");
        db.initialize().expect("schema v1");
        db
    }

    #[test]
    fn cache_roundtrip_and_overwrite() {
        let db = test_db();
        assert_eq!(db.cache_get("missing").expect("get"), None);

        db.cache_put("cve", r#"{"id":"CVE-2024-3094"}"#, 60)
            .expect("put");
        assert_eq!(
            db.cache_get("cve").expect("get"),
            Some(r#"{"id":"CVE-2024-3094"}"#.to_owned())
        );

        db.cache_put("cve", r#"{"id":"CVE-2024-3095"}"#, 60)
            .expect("overwrite");
        assert_eq!(
            db.cache_get("cve").expect("get"),
            Some(r#"{"id":"CVE-2024-3095"}"#.to_owned())
        );
    }

    #[test]
    fn ttl_zero_is_immediately_expired() {
        let db = test_db();
        db.cache_put("short", "payload", 0).expect("put");
        assert_eq!(db.cache_get("short").expect("get"), None);

        let conn = db.conn();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intel_cache WHERE cache_key = 'short'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(rows, 0, "expired row should be deleted on read");
    }

    #[test]
    fn negative_ttl_is_expired() {
        let db = test_db();
        db.cache_put("past", "payload", -30).expect("put");
        assert_eq!(db.cache_get("past").expect("get"), None);
    }

    #[test]
    fn purge_removes_only_expired_rows() {
        let db = test_db();
        db.cache_put("expired-a", "x", 0).expect("put a");
        db.cache_put("expired-b", "y", -5).expect("put b");
        db.cache_put("live", "z", 3_600).expect("put live");

        assert_eq!(db.purge_expired_intel_cache().expect("purge"), 2);
        assert_eq!(db.cache_get("live").expect("get"), Some("z".to_owned()));
        assert_eq!(db.purge_expired_intel_cache().expect("second purge"), 0);
    }
}
