use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub discord_token: String,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub categories: Vec<CategoryConfig>,
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
    "cipherbot.db".to_string()
}

#[derive(Clone, Debug, Deserialize)]
pub struct CategoryConfig {
    pub id: u64,
    #[serde(default = "default_true")]
    pub auto_thread_links: bool,
    #[serde(default = "default_ttl")]
    pub message_ttl_hours: u64,
}

const fn default_true() -> bool {
    true
}

const fn default_ttl() -> u64 {
    24
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let config_path = std::env::var("CONFIG_PATH")
            .unwrap_or_else(|_| "config.yaml".to_string());

        tracing::info!("Looking for config file at '{}'", config_path);

        let mut config: Config = if Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path)?;
            tracing::info!("Config file found and loaded from '{}'", config_path);
            serde_yaml::from_str(&content)?
        } else {
            tracing::info!("No config file at '{}' — starting with defaults, will check env vars", config_path);
            Config {
                discord_token: String::new(),
                database: DatabaseConfig::default(),
                categories: Vec::new(),
            }
        };

        if let Ok(token) = std::env::var("DISCORD_TOKEN") {
            tracing::info!("Using DISCORD_TOKEN from environment variable");
            config.discord_token = token;
        } else {
            tracing::debug!("DISCORD_TOKEN env var not set, relying on config file");
        }

        if let Ok(path) = std::env::var("DATABASE_PATH") {
            tracing::info!("Using DATABASE_PATH from environment: {}", path);
            config.database.path = path;
        } else {
            tracing::debug!("DATABASE_PATH env var not set, using default: {}", config.database.path);
        }

        if let Ok(cats) = std::env::var("MONITORED_CATEGORIES") {
            tracing::info!("MONITORED_CATEGORIES env var found: {:?}", cats);
            let ids: Vec<u64> = cats
                .split(',')
                .filter_map(|s| {
                    let trimmed = s.trim();
                    let parsed = trimmed.parse::<u64>();
                    if parsed.is_err() {
                        tracing::warn!("  -> failed to parse category ID: '{:?}'", trimmed);
                    }
                    parsed.ok()
                })
                .collect();

            if ids.is_empty() {
                tracing::warn!("MONITORED_CATEGORIES set but no valid IDs found in: {:?}", cats);
            } else {
                tracing::info!("Parsed {} category ID(s) from MONITORED_CATEGORIES: {:?}", ids.len(), ids);
                config.categories = ids
                    .into_iter()
                    .map(|id| CategoryConfig {
                        id,
                        auto_thread_links: true,
                        message_ttl_hours: 24,
                    })
                    .collect();
            }
        } else {
            tracing::debug!("MONITORED_CATEGORIES env var not set, using config file categories");
        }

        if config.discord_token.is_empty() {
            anyhow::bail!(
                "DISCORD_TOKEN is not set. Provide it via DISCORD_TOKEN env var \
                 or in the config.yaml file."
            );
        }
        if config.categories.is_empty() {
            anyhow::bail!(
                "No categories configured. Set MONITORED_CATEGORIES env var \
                 (e.g. MONITORED_CATEGORIES=\"123456789\") or add them in config.yaml."
            );
        }

        tracing::info!(
            "Final config: {} category(ies) configured, db path: '{}'",
            config.categories.len(),
            config.database.path,
        );

        Ok(config)
    }
}
