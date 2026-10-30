//! Runtime state shared by every handler.
//!
//! Built once in `main`, wrapped in an `Arc`, and cloned into the serenity
//! handler, the health server, the cleanup loop and the scheduler.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use parking_lot::RwLock;
use serenity::all::UserId;

use crate::config::Config;
use crate::db::Database;
use crate::ops::metrics::Metrics;

/// Standard result type for bot operations.
pub type BotResult<T> = anyhow::Result<T>;

/// Shared handle passed to every async task.
pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
    /// HTTP client for external APIs only (fixed hosts, 5s timeout).
    pub http: reqwest::Client,
    pub metrics: Metrics,
    pub started_at: Instant,
    pub phishing: PhishingStore,
    pub bot_user_id: OnceLock<UserId>,
}

/// Domain blocklist fed by the phishing feed (populated by the moderation lane).
#[derive(Default)]
pub struct PhishingStore {
    domains: RwLock<HashSet<String>>,
}

impl PhishingStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the whole feed. Returns the number of stored domains.
    pub fn replace(&self, domains: impl IntoIterator<Item = String>) -> usize {
        let normalized: HashSet<String> = domains
            .into_iter()
            .filter_map(|domain| normalize_domain(&domain))
            .collect();
        let count = normalized.len();
        *self.domains.write() = normalized;
        count
    }

    pub fn contains_domain(&self, domain: &str) -> bool {
        normalize_domain(domain)
            .map(|candidate| self.domains.read().contains(&candidate))
            .unwrap_or(false)
    }

    pub fn len(&self) -> usize {
        self.domains.read().len()
    }
}

fn normalize_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let host = if trimmed.contains("://") {
        crate::util::text::domain_of(trimmed)?
    } else {
        let lowered = trimmed.trim_end_matches('.').to_ascii_lowercase();
        lowered
            .strip_prefix("www.")
            .unwrap_or(lowered.as_str())
            .to_owned()
    };
    if host.is_empty() { None } else { Some(host) }
}

pub fn build_state(config: Config, db: Arc<Database>) -> BotResult<SharedState> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent(format!(
            "CipherBot/{} (+https://github.com/cleeryy/cipherbot)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;

    Ok(Arc::new(AppState {
        config,
        db,
        http,
        metrics: Metrics::default(),
        started_at: Instant::now(),
        phishing: PhishingStore::new(),
        bot_user_id: OnceLock::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phishing_store_normalizes() {
        let store = PhishingStore::new();
        let count = store.replace(vec![
            "Evil.Example".to_owned(),
            "www.bad.example".to_owned(),
            "  ".to_owned(),
            "https://URL.example/path".to_owned(),
        ]);
        assert_eq!(count, 3);
        assert!(store.contains_domain("evil.example"));
        assert!(store.contains_domain("WWW.EVIL.EXAMPLE"));
        assert!(store.contains_domain("https://evil.example/x"));
        assert!(store.contains_domain("bad.example"));
        assert!(store.contains_domain("url.example"));
        assert!(!store.contains_domain("good.example"));
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn phishing_store_replace_overwrites() {
        let store = PhishingStore::new();
        store.replace(vec!["a.example".to_owned()]);
        store.replace(vec!["b.example".to_owned()]);
        assert!(!store.contains_domain("a.example"));
        assert!(store.contains_domain("b.example"));
    }
}
