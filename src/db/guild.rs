//! Per-guild settings, stored as a JSON blob in `guild_config`.
//!
//! Guild ids are passed as raw `u64` so this layer stays independent of serenity;
//! callers use `guild_id.get()`.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Database;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuildSettings {
    pub modlog_channel: Option<u64>,
    pub quarantine_role: Option<u64>,
    pub verify_role: Option<u64>,
    pub ticket_role: Option<u64>,
    pub ctf_channel: Option<u64>,
    pub digest_channel: Option<u64>,
    pub locale: String,
    pub automod: AutomodSettings,
    pub raid: RaidSettings,
    pub links: LinkSettings,
}

impl Default for GuildSettings {
    fn default() -> Self {
        Self {
            modlog_channel: None,
            quarantine_role: None,
            verify_role: None,
            ticket_role: None,
            ctf_channel: None,
            digest_channel: None,
            locale: "en".to_owned(),
            automod: AutomodSettings::default(),
            raid: RaidSettings::default(),
            links: LinkSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomodSettings {
    pub invites: bool,
    pub phishing: bool,
    pub mass_mentions: bool,
    pub duplicate_spam: bool,
    pub caps: bool,
    pub token_leaks: bool,
}

impl Default for AutomodSettings {
    fn default() -> Self {
        Self {
            invites: true,
            phishing: true,
            mass_mentions: true,
            duplicate_spam: true,
            caps: true,
            token_leaks: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RaidSettings {
    pub enabled: bool,
    pub min_account_age_hours: u64,
    pub require_avatar: bool,
}

impl Default for RaidSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_account_age_hours: 24,
            require_avatar: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkSettings {
    pub auto_thread: bool,
    pub ttl_hours: u64,
    /// When true, these values override the per-category YAML settings.
    /// Set by `/config links` or `/config set auto_thread|link_ttl_hours`.
    pub links_custom: bool,
}

impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            auto_thread: true,
            ttl_hours: 24,
            links_custom: false,
        }
    }
}

impl Database {
    pub fn get_settings(&self, guild_id: u64) -> Result<GuildSettings> {
        let conn = self.conn();
        let raw: Option<String> = conn
            .query_row(
                "SELECT settings_json FROM guild_config WHERE guild_id = ?1",
                params![guild_id as i64],
                |row| row.get(0),
            )
            .optional()?;

        let Some(raw) = raw else {
            return Ok(GuildSettings::default());
        };

        match serde_json::from_str(&raw) {
            Ok(settings) => Ok(settings),
            Err(error) => {
                tracing::warn!(
                    "Corrupt settings JSON for guild {guild_id} ({error}); using defaults"
                );
                Ok(GuildSettings::default())
            }
        }
    }

    pub fn save_settings(&self, guild_id: u64, settings: &GuildSettings) -> Result<()> {
        let json = serde_json::to_string(settings)?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO guild_config (guild_id, settings_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(guild_id) DO UPDATE SET
                 settings_json = excluded.settings_json,
                 updated_at = excluded.updated_at",
            params![guild_id as i64, json, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Every guild that ever saved settings.
    pub fn all_guild_ids(&self) -> Result<Vec<u64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT guild_id FROM guild_config ORDER BY guild_id")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row? as u64);
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("guild.db");
        let db = Database::open(path.to_str().expect("utf8 path")).expect("open");
        db.initialize().expect("init");
        (dir, db)
    }

    #[test]
    fn settings_roundtrip() {
        let (_dir, db) = temp_db();
        let guild_id = 42;

        let defaults = db.get_settings(guild_id).expect("defaults");
        assert_eq!(defaults, GuildSettings::default());

        let settings = GuildSettings {
            modlog_channel: Some(7),
            locale: "fr".to_owned(),
            automod: AutomodSettings {
                caps: false,
                ..AutomodSettings::default()
            },
            raid: RaidSettings {
                enabled: true,
                ..RaidSettings::default()
            },
            links: LinkSettings {
                ttl_hours: 6,
                ..LinkSettings::default()
            },
            ..GuildSettings::default()
        };

        db.save_settings(guild_id, &settings).expect("save");
        let loaded = db.get_settings(guild_id).expect("load");
        assert_eq!(loaded, settings);

        db.save_settings(guild_id, &GuildSettings::default())
            .expect("overwrite");
        assert_eq!(
            db.get_settings(guild_id).expect("reload"),
            GuildSettings::default()
        );

        assert_eq!(db.all_guild_ids().expect("ids"), vec![guild_id]);
    }
}
