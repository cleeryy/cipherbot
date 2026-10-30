//! Phishing domain feed.
//!
//! `refresh` delegates to [`crate::security::automod::refresh_feeds`] (single
//! implementation, two entry points); `is_bad_domain` is the hot-path check used
//! by automod. The store matches an exact host **or any subdomain** of a listed
//! domain, without a bare `ends_with` that would let `notevil.example` match
//! `evil.example`.

use crate::bot::{BotResult, SharedState};

/// Download the feeds and rebuild the in-memory store. Returns the entry count.
pub async fn refresh(state: &SharedState) -> BotResult<usize> {
    super::automod::refresh_feeds(state).await?;
    Ok(state.phishing.len())
}

/// True when `domain` (or one of its parents) is on the blocklist.
pub fn is_bad_domain(state: &SharedState, domain: &str) -> bool {
    let normalized = normalize(domain);
    if normalized.is_empty() {
        return false;
    }
    if state.phishing.contains_domain(&normalized) {
        return true;
    }

    let mut rest = normalized.as_str();
    while let Some((_, parent)) = rest.split_once('.') {
        if parent.is_empty() {
            break;
        }
        if state.phishing.contains_domain(parent) {
            return true;
        }
        rest = parent;
    }
    false
}

fn normalize(domain: &str) -> String {
    let lowered = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    lowered
        .strip_prefix("www.")
        .unwrap_or(lowered.as_str())
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_strips_www_and_case() {
        assert_eq!(normalize(" WWW.Evil.Example. "), "evil.example");
        assert_eq!(normalize(""), "");
    }
}
