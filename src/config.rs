//! Configuration: YAML file plus environment overrides.
//!
//! The bot must stay usable without a config file: every field has a default and
//! `Config::load` only hard-fails on a missing Discord token. Link monitoring is
//! optional now — an empty `categories` list only produces a warning.

use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// A string that must never be printed by accident.
///
/// `Debug` renders `Secret(***)` and there is no `Display` implementation, so a
/// secret can only leave the process through [`Secret::expose`].
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Explicitly read the secret. Only call this at the point of use.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Root configuration object.
#[derive(Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub discord_token: Secret,
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Legacy link-monitor categories. Optional: an empty list is valid.
    #[serde(default)]
    pub categories: Vec<CategoryConfig>,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub intel: IntelConfig,
    #[serde(default)]
    pub features: FeatureConfig,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("discord_token", &self.discord_token)
            .field("database", &self.database)
            .field("categories", &self.categories)
            .field("health", &self.health)
            .field("intel", &self.intel)
            .field("features", &self.features)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

fn default_db_path() -> String {
    "cipherbot.db".to_owned()
}

/// Legacy link-monitor category settings.
#[derive(Clone, Debug, Deserialize)]
pub struct CategoryConfig {
    pub id: u64,
    #[serde(default = "default_true")]
    pub auto_thread_links: bool,
    #[serde(default = "default_ttl")]
    pub message_ttl_hours: u64,
}

impl Default for CategoryConfig {
    fn default() -> Self {
        Self {
            id: 0,
            auto_thread_links: true,
            message_ttl_hours: default_ttl(),
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_ttl() -> u64 {
    24
}

#[derive(Clone, Debug, Deserialize)]
pub struct HealthConfig {
    #[serde(default = "default_health_addr")]
    pub addr: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            addr: default_health_addr(),
            enabled: true,
        }
    }
}

fn default_health_addr() -> String {
    "0.0.0.0:8080".to_owned()
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct IntelConfig {
    #[serde(default)]
    pub nvd_api_key: Option<Secret>,
    #[serde(default)]
    pub vt_api_key: Option<Secret>,
    #[serde(default)]
    pub ipinfo_token: Option<Secret>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct FeatureConfig {
    /// Anti-raid join events need the privileged GUILD_MEMBERS intent.
    #[serde(default)]
    pub member_intent: bool,
}

impl Config {
    /// Load configuration from `CONFIG_PATH` (default `config.yaml`) then env.
    ///
    /// A missing or empty YAML file is not an error; a missing token is.
    pub fn load() -> Result<Self> {
        let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.yaml".to_owned());

        let mut config = if Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("reading config file '{config_path}'"))?;
            tracing::info!("Config file loaded from '{config_path}'");
            Self::from_yaml(&content)?
        } else {
            tracing::info!("No config file at '{config_path}' — using defaults + env");
            Self::default()
        };

        config.apply_env()?;

        if config.discord_token.is_empty() {
            anyhow::bail!(
                "DISCORD_TOKEN is not set. Provide it via the DISCORD_TOKEN env var \
                 or in config.yaml."
            );
        }
        if config.categories.is_empty() {
            tracing::warn!(
                "No link-monitor categories configured — the link pipeline is inactive. \
                 Set MONITORED_CATEGORIES or add categories to config.yaml."
            );
        }

        tracing::info!(
            "Config ready: {} categorie(s), db='{}', health='{}' (enabled={}), member_intent={}",
            config.categories.len(),
            config.database.path,
            config.health.addr,
            config.health.enabled,
            config.features.member_intent,
        );

        Ok(config)
    }

    /// Parse YAML without touching the environment (used by tests).
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        if yaml.trim().is_empty() {
            return Ok(Self::default());
        }
        let config: Self = serde_yaml::from_str(yaml).context("parsing config YAML")?;
        Ok(config)
    }

    fn apply_env(&mut self) -> Result<()> {
        if let Ok(token) = std::env::var("DISCORD_TOKEN")
            && !token.trim().is_empty()
        {
            self.discord_token = Secret::new(token.trim());
        }

        if let Ok(path) = std::env::var("DATABASE_PATH")
            && !path.trim().is_empty()
        {
            self.database.path = path;
        }

        if let Ok(cats) = std::env::var("MONITORED_CATEGORIES") {
            let ids = parse_category_ids(&cats);
            if ids.is_empty() {
                self.categories.clear();
                tracing::warn!("MONITORED_CATEGORIES is empty — link pipeline disabled");
            } else {
                self.categories = ids
                    .into_iter()
                    .map(|id| CategoryConfig {
                        id,
                        ..CategoryConfig::default()
                    })
                    .collect();
            }
        }

        if let Ok(addr) = std::env::var("HEALTH_ADDR") {
            if addr.trim().is_empty() {
                self.health.enabled = false;
                tracing::info!("HEALTH_ADDR is empty — health server disabled");
            } else {
                self.health.addr = addr;
                self.health.enabled = true;
            }
        }

        if let Some(secret) = env_secret("NVD_API_KEY") {
            self.intel.nvd_api_key = Some(secret);
        }
        if let Some(secret) = env_secret("VT_API_KEY") {
            self.intel.vt_api_key = Some(secret);
        }
        if let Some(secret) = env_secret("IPINFO_TOKEN") {
            self.intel.ipinfo_token = Some(secret);
        }

        if let Ok(raw) = std::env::var("ENABLE_MEMBER_INTENT") {
            self.features.member_intent = parse_bool(&raw);
        }

        Ok(())
    }
}

fn env_secret(key: &str) -> Option<Secret> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => Some(Secret::new(value.trim())),
        _ => None,
    }
}

fn parse_bool(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_category_ids(raw: &str) -> Vec<u64> {
    raw.split(',')
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            match trimmed.parse::<u64>() {
                Ok(id) if id != 0 => Some(id),
                _ => {
                    tracing::warn!("Ignoring invalid category ID '{trimmed}'");
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_yaml() {
        let yaml = r#"
discord_token: "test-token"
"#;
        let config = Config::from_yaml(yaml).expect("yaml parses");
        assert_eq!(config.discord_token.expose(), "test-token");
        assert_eq!(config.database.path, "cipherbot.db");
        assert_eq!(config.health.addr, "0.0.0.0:8080");
        assert!(config.health.enabled);
        assert!(!config.features.member_intent);
        assert!(config.categories.is_empty());
    }

    #[test]
    fn optional_categories_are_accepted() {
        let yaml = r#"
discord_token: token
categories: []
"#;
        let config = Config::from_yaml(yaml).expect("empty categories are valid");
        assert!(config.categories.is_empty());
    }

    #[test]
    fn parses_full_yaml() {
        let yaml = r#"
discord_token: token
database:
  path: /tmp/x.db
categories:
  - id: 42
    auto_thread_links: false
    message_ttl_hours: 6
health:
  addr: 127.0.0.1:9000
  enabled: false
intel:
  nvd_api_key: nvd
  vt_api_key: vt
features:
  member_intent: true
"#;
        let config = Config::from_yaml(yaml).expect("yaml parses");
        assert_eq!(config.database.path, "/tmp/x.db");
        assert_eq!(config.categories.len(), 1);
        assert!(!config.categories[0].auto_thread_links);
        assert_eq!(config.categories[0].message_ttl_hours, 6);
        assert_eq!(config.health.addr, "127.0.0.1:9000");
        assert!(!config.health.enabled);
        assert!(config.features.member_intent);
        assert_eq!(
            config.intel.vt_api_key.as_ref().map(Secret::expose),
            Some("vt")
        );
    }

    #[test]
    fn empty_yaml_is_default() {
        let config = Config::from_yaml("").expect("empty yaml");
        assert!(config.discord_token.is_empty());
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("super-secret-value");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "Secret(***)");
        assert!(!rendered.contains("super-secret-value"));

        let config = Config {
            discord_token: secret,
            ..Config::default()
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("super-secret-value"));
        assert!(rendered.contains("Secret(***)"));
    }

    #[test]
    fn secret_from_string() {
        let secret: Secret = String::from("abc").into();
        assert_eq!(secret.expose(), "abc");
        assert!(!secret.is_empty());
    }

    #[test]
    fn bool_parsing() {
        for value in ["1", "true", "TRUE", "yes", "on"] {
            assert!(parse_bool(value), "{value}");
        }
        for value in ["0", "false", "no", "off", ""] {
            assert!(!parse_bool(value), "{value}");
        }
    }

    #[test]
    fn category_parsing_skips_garbage() {
        assert_eq!(parse_category_ids("1, 2 ,x,0,3"), vec![1, 2, 3]);
        assert!(parse_category_ids("").is_empty());
    }
}
