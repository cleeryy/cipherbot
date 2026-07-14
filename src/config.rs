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

        let mut config: Config = if Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path)?;
            serde_yaml::from_str(&content)?
        } else {
            anyhow::bail!(
                "Config file not found at '{}'. Create one or set CONFIG_PATH env var.",
                config_path
            );
        };

        if let Ok(token) = std::env::var("DISCORD_TOKEN") {
            config.discord_token = token;
        }
        if let Ok(path) = std::env::var("DATABASE_PATH") {
            config.database.path = path;
        }
        if let Ok(cats) = std::env::var("MONITORED_CATEGORIES") {
            let ids: Vec<u64> = cats
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            if !ids.is_empty() {
                config.categories = ids
                    .into_iter()
                    .map(|id| CategoryConfig {
                        id,
                        auto_thread_links: true,
                        message_ttl_hours: 24,
                    })
                    .collect();
            }
        }

        if config.discord_token.is_empty() {
            anyhow::bail!("DISCORD_TOKEN is not set");
        }

        if config.categories.is_empty() {
            anyhow::bail!(
                "No categories configured. Add at least one category ID in config.yaml \
                 or set MONITORED_CATEGORIES env var (e.g. MONITORED_CATEGORIES=\"123456789\")"
            );
        }

        Ok(config)
    }
}
