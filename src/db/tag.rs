//! Tag persistence: reusable snippets served by `/tag`.
//!
//! Names are normalized before every read/write (`a-z`, `0-9`, `_`, `-`,
//! 1-32 chars) so a tag is reachable regardless of how it was typed.
//! `get_tag` bumps `uses` in the same locked connection that reads the row, so
//! the counter can never be incremented without returning the snippet.

use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use super::Database;

/// Maximum normalized tag name length.
pub const MAX_NAME_CHARS: usize = 32;

/// One saved snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub content: String,
    pub created_by: u64,
    pub uses: i64,
    pub created_at: i64,
}

/// Normalize a user-supplied tag name, or `None` when it is unusable.
///
/// Trims surrounding whitespace and lowercases; any character outside
/// `[a-z0-9_-]` rejects the whole name (no silent transliteration).
pub fn normalize_name(raw: &str) -> Option<String> {
    let name = raw.trim().to_ascii_lowercase();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return None;
    }
    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-');
    valid.then_some(name)
}

const SELECT_TAG: &str = "SELECT name, content, created_by, uses, created_at
     FROM tags
    WHERE guild_id = ?1 AND name = ?2";

fn map_tag(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        name: row.get(0)?,
        content: row.get(1)?,
        created_by: row.get::<_, i64>(2)? as u64,
        uses: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl Database {
    /// Create a tag, or replace the content of an existing one.
    ///
    /// `uses` and `created_at` are preserved on update; `created_by` tracks the
    /// author of the current content.
    pub fn upsert_tag(
        &self,
        guild_id: u64,
        name: &str,
        content: &str,
        created_by: u64,
    ) -> Result<()> {
        let Some(name) = normalize_name(name) else {
            bail!("Tag names are 1-32 characters from a-z, 0-9, `_` or `-`.");
        };
        if content.trim().is_empty() {
            bail!("Tag content cannot be empty.");
        }

        let conn = self.conn();
        conn.execute(
            "INSERT INTO tags (guild_id, name, content, created_by, uses, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)
             ON CONFLICT(guild_id, name) DO UPDATE SET
                 content = excluded.content,
                 created_by = excluded.created_by",
            params![
                guild_id as i64,
                name,
                content,
                created_by as i64,
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// Look a tag up and increment its use counter in one locked step.
    pub fn get_tag(&self, guild_id: u64, name: &str) -> Result<Option<Tag>> {
        let Some(name) = normalize_name(name) else {
            return Ok(None);
        };

        let conn = self.conn();
        let updated = conn.execute(
            "UPDATE tags SET uses = uses + 1 WHERE guild_id = ?1 AND name = ?2",
            params![guild_id as i64, name],
        )?;
        if updated == 0 {
            return Ok(None);
        }

        let tag = conn
            .query_row(SELECT_TAG, params![guild_id as i64, name], map_tag)
            .optional()?;
        Ok(tag)
    }

    /// Delete a tag; returns whether a row was removed.
    pub fn delete_tag(&self, guild_id: u64, name: &str) -> Result<bool> {
        let Some(name) = normalize_name(name) else {
            return Ok(false);
        };

        let conn = self.conn();
        let deleted = conn.execute(
            "DELETE FROM tags WHERE guild_id = ?1 AND name = ?2",
            params![guild_id as i64, name],
        )?;
        Ok(deleted > 0)
    }

    /// Every tag of a guild, ordered by name.
    pub fn list_tags(&self, guild_id: u64) -> Result<Vec<Tag>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT name, content, created_by, uses, created_at
               FROM tags
              WHERE guild_id = ?1
              ORDER BY name",
        )?;
        let rows = stmt.query_map(params![guild_id as i64], map_tag)?;

        let mut tags = Vec::new();
        for row in rows {
            tags.push(row?);
        }
        Ok(tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_db() -> (TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tags.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn normalize_name_accepts_valid_names() {
        assert_eq!(normalize_name("faq"), Some("faq".to_owned()));
        assert_eq!(normalize_name("FAQ"), Some("faq".to_owned()));
        assert_eq!(
            normalize_name("  Build-Order  "),
            Some("build-order".to_owned())
        );
        assert_eq!(normalize_name("wifi_5g"), Some("wifi_5g".to_owned()));
        assert_eq!(normalize_name("a"), Some("a".to_owned()));
        assert_eq!(
            normalize_name(&"a".repeat(MAX_NAME_CHARS)),
            Some("a".repeat(MAX_NAME_CHARS))
        );
    }

    #[test]
    fn normalize_name_rejects_invalid_names() {
        assert_eq!(normalize_name(""), None);
        assert_eq!(normalize_name("   "), None);
        assert_eq!(normalize_name("two words"), None);
        assert_eq!(normalize_name("faq!"), None);
        assert_eq!(normalize_name("café"), None);
        assert_eq!(normalize_name("../etc/passwd"), None);
        assert_eq!(normalize_name(&"a".repeat(MAX_NAME_CHARS + 1)), None);
    }

    #[test]
    fn upsert_and_get_roundtrip() {
        let (_dir, db) = temp_db();
        db.upsert_tag(1, "  FAQ ", "Read the pinned post.", 42)
            .expect("insert");

        let tag = db.get_tag(1, "faq").expect("get").expect("present");
        assert_eq!(tag.name, "faq");
        assert_eq!(tag.content, "Read the pinned post.");
        assert_eq!(tag.created_by, 42);
        assert_eq!(tag.uses, 1, "a read increments the counter");
        assert!(tag.created_at > 0);
    }

    #[test]
    fn upsert_update_preserves_uses_and_created_at() {
        let (_dir, db) = temp_db();
        db.upsert_tag(1, "faq", "old content", 42).expect("insert");
        db.get_tag(1, "faq").expect("get").expect("present");

        {
            let conn = db.conn();
            conn.execute(
                "UPDATE tags SET created_at = 12345 WHERE guild_id = 1 AND name = 'faq'",
                [],
            )
            .expect("pin created_at");
        }

        db.upsert_tag(1, "faq", "new content", 99).expect("update");

        let tags = db.list_tags(1).expect("list");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].content, "new content");
        assert_eq!(tags[0].created_by, 99);
        assert_eq!(tags[0].uses, 1, "uses survive content updates");
        assert_eq!(tags[0].created_at, 12345, "creation time is immutable");
    }

    #[test]
    fn get_increments_uses_each_time() {
        let (_dir, db) = temp_db();
        db.upsert_tag(1, "faq", "answer", 42).expect("insert");

        assert_eq!(db.get_tag(1, "faq").expect("1st").expect("tag").uses, 1);
        assert_eq!(db.get_tag(1, "faq").expect("2nd").expect("tag").uses, 2);
        assert_eq!(db.get_tag(1, "faq").expect("3rd").expect("tag").uses, 3);

        let tags = db.list_tags(1).expect("list");
        assert_eq!(tags[0].uses, 3);
    }

    #[test]
    fn get_unknown_tag_returns_none_without_creating_a_row() {
        let (_dir, db) = temp_db();
        assert!(db.get_tag(1, "missing").expect("get").is_none());
        assert!(db.get_tag(1, "bad name!").expect("invalid").is_none());
        assert!(db.list_tags(1).expect("list").is_empty());
    }

    #[test]
    fn delete_tag_reports_whether_a_row_was_removed() {
        let (_dir, db) = temp_db();
        db.upsert_tag(1, "faq", "answer", 42).expect("insert");

        assert!(db.delete_tag(1, "FAQ").expect("first delete"));
        assert!(!db.delete_tag(1, "faq").expect("second delete"));
        assert!(!db.delete_tag(1, "not a name").expect("invalid name"));
        assert!(db.get_tag(1, "faq").expect("get").is_none());
    }

    #[test]
    fn list_tags_is_ordered_and_guild_scoped() {
        let (_dir, db) = temp_db();
        for name in ["zulu", "alpha", "mike"] {
            db.upsert_tag(7, name, "answer", 42).expect("insert");
        }
        db.upsert_tag(8, "other", "answer", 42).expect("insert");

        let names: Vec<String> = db
            .list_tags(7)
            .expect("list")
            .into_iter()
            .map(|tag| tag.name)
            .collect();
        assert_eq!(names, vec!["alpha", "mike", "zulu"]);

        let other = db.list_tags(8).expect("list");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].name, "other");

        assert!(db.list_tags(9).expect("list").is_empty());
    }

    #[test]
    fn upsert_rejects_invalid_names_and_empty_content() {
        let (_dir, db) = temp_db();

        assert!(db.upsert_tag(1, "bad name", "answer", 42).is_err());
        assert!(db.upsert_tag(1, "faq", "   ", 42).is_err());
        assert!(db.list_tags(1).expect("list").is_empty());
    }
}
