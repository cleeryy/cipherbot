//! VirusTotal API v3 client.
//!
//! Requires `VT_API_KEY`. Reports are cached for one hour because the public API
//! allows only four lookups per minute. The key travels in the `x-apikey` header
//! and is never logged.

use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bot::SharedState;
use crate::util::text::truncate;

const API_BASE: &str = "https://www.virustotal.com/api/v3";
const GUI_BASE: &str = "https://www.virustotal.com/gui";
const VT_TTL_SECS: i64 = 60 * 60;
const NOT_CONFIGURED: &str = "VirusTotal is not configured on this instance (set VT_API_KEY).";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VtReport {
    pub kind: String,
    pub target: String,
    pub malicious: u64,
    pub suspicious: u64,
    pub harmless: u64,
    pub undetected: u64,
    pub timeout: u64,
    pub last_analysis_date: Option<i64>,
    pub permalink: String,
    pub names: Vec<String>,
}

/// True when a non-empty VirusTotal API key is configured.
pub fn configured(state: &SharedState) -> bool {
    api_key(state).is_some()
}

pub async fn file(state: &SharedState, hash: &str) -> Result<VtReport> {
    let target = hash.trim().to_ascii_lowercase();
    if target.is_empty() {
        bail!("Provide a file hash to look up.");
    }
    let permalink = format!("{GUI_BASE}/file/{target}");
    fetch(
        state,
        "file",
        &target,
        &format!("files/{target}"),
        permalink,
    )
    .await
}

pub async fn url(state: &SharedState, url: &str) -> Result<VtReport> {
    let target = url.trim();
    if target.is_empty() {
        bail!("Provide a URL to look up.");
    }
    let id = url_id(target);
    let permalink = format!("{GUI_BASE}/url/{id}");
    fetch(state, "url", target, &format!("urls/{id}"), permalink).await
}

pub async fn domain(state: &SharedState, raw: &str) -> Result<VtReport> {
    let target = raw.trim().to_ascii_lowercase();
    if target.is_empty() {
        bail!("Provide a domain to look up.");
    }
    let permalink = format!("{GUI_BASE}/domain/{target}");
    fetch(
        state,
        "domain",
        &target,
        &format!("domains/{target}"),
        permalink,
    )
    .await
}

pub async fn ip(state: &SharedState, raw: &str) -> Result<VtReport> {
    let target = raw.trim().to_owned();
    if target.is_empty() {
        bail!("Provide an IP address to look up.");
    }
    let permalink = format!("{GUI_BASE}/ip-address/{target}");
    fetch(
        state,
        "ip",
        &target,
        &format!("ip_addresses/{target}"),
        permalink,
    )
    .await
}

/// base64url (no padding) identifier used by the `/urls/{id}` endpoint.
pub fn url_id(url: &str) -> String {
    URL_SAFE_NO_PAD.encode(url.as_bytes())
}

async fn fetch(
    state: &SharedState,
    kind: &str,
    target: &str,
    path: &str,
    permalink: String,
) -> Result<VtReport> {
    let key = format!("vt:{kind}:{target}");
    if let Some(report) = recall(state, &key) {
        return Ok(report);
    }

    let Some(api_key) = api_key(state) else {
        bail!(NOT_CONFIGURED);
    };

    let response = state
        .http
        .get(format!("{API_BASE}/{path}"))
        .header("x-apikey", api_key)
        .send()
        .await
        .map_err(|error| {
            tracing::debug!("VirusTotal {kind} request failed: {}", error.without_url());
            anyhow::anyhow!("VirusTotal is unreachable right now.")
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    match status {
        reqwest::StatusCode::NOT_FOUND => {
            bail!("VirusTotal has no record for `{}`.", truncate(target, 80))
        }
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            tracing::debug!(
                "VirusTotal rejected the API key (HTTP {status}, body: {})",
                truncate(&body, 200)
            );
            bail!("VirusTotal rejected the API key — check VT_API_KEY.")
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            tracing::debug!("VirusTotal rate limit (body: {})", truncate(&body, 200));
            bail!("VirusTotal is rate limiting requests — try again in a minute.");
        }
        _ if !status.is_success() => {
            tracing::debug!("VirusTotal HTTP {status} (body: {})", truncate(&body, 200));
            bail!("VirusTotal lookup failed (HTTP {status}).")
        }
        _ => {}
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("VirusTotal JSON parse failed: {error}");
        anyhow::anyhow!("VirusTotal returned an unreadable response.")
    })?;
    let report = parse_report(kind, target, &value, permalink)?;
    remember(state, &key, &report);
    Ok(report)
}

fn api_key(state: &SharedState) -> Option<&str> {
    state
        .config
        .intel
        .vt_api_key
        .as_ref()
        .map(crate::config::Secret::expose)
        .filter(|key| !key.is_empty())
}

fn parse_report(kind: &str, target: &str, body: &Value, permalink: String) -> Result<VtReport> {
    let data = body
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("VirusTotal returned no data for `{target}`."))?;
    let attributes = data.get("attributes").unwrap_or(&Value::Null);
    let stats = attributes.get("last_analysis_stats");

    let count = |name: &str| {
        stats
            .and_then(|stats| stats.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };

    let names = attributes
        .get("names")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .take(5)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Ok(VtReport {
        kind: kind.to_owned(),
        target: target.to_owned(),
        malicious: count("malicious"),
        suspicious: count("suspicious"),
        harmless: count("harmless"),
        undetected: count("undetected"),
        timeout: count("timeout"),
        last_analysis_date: attributes.get("last_analysis_date").and_then(Value::as_i64),
        permalink,
        names,
    })
}

fn recall(state: &SharedState, key: &str) -> Option<VtReport> {
    let payload = state.db.cache_get(key).ok().flatten()?;
    serde_json::from_str(&payload).ok()
}

fn remember(state: &SharedState, key: &str, value: &VtReport) {
    match serde_json::to_string(value) {
        Ok(payload) => {
            if let Err(error) = state.db.cache_put(key, &payload, VT_TTL_SECS) {
                tracing::warn!("VirusTotal cache write failed: {error}");
            }
        }
        Err(error) => tracing::warn!("VirusTotal cache serialisation failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_ids_are_base64url_without_padding() {
        assert_eq!(url_id("https://example.com"), "aHR0cHM6Ly9leGFtcGxlLmNvbQ");
        assert!(!url_id("http://a.b/c?d=1").contains('='));
    }

    #[test]
    fn parses_file_report() {
        let body = json!({
            "data": {
                "attributes": {
                    "last_analysis_stats": {
                        "malicious": 3,
                        "suspicious": 1,
                        "harmless": 50,
                        "undetected": 5,
                        "timeout": 0
                    },
                    "last_analysis_date": 1_710_000_000_i64,
                    "names": ["evil.exe", "evil (1).exe", "x", "y", "z", "ignored"]
                }
            }
        });
        let report = parse_report(
            "file",
            "abc",
            &body,
            "https://www.virustotal.com/gui/file/abc".to_owned(),
        )
        .expect("report");
        assert_eq!(report.kind, "file");
        assert_eq!(report.target, "abc");
        assert_eq!(report.malicious, 3);
        assert_eq!(report.suspicious, 1);
        assert_eq!(report.harmless, 50);
        assert_eq!(report.undetected, 5);
        assert_eq!(report.timeout, 0);
        assert_eq!(report.last_analysis_date, Some(1_710_000_000));
        assert_eq!(report.names.len(), 5);
        assert!(report.permalink.ends_with("/abc"));
    }

    #[test]
    fn parses_minimal_url_report() {
        let body = json!({"data": {"attributes": {}}});
        let report =
            parse_report("url", "https://example.com", &body, "p".to_owned()).expect("report");
        assert_eq!(report.malicious, 0);
        assert!(report.last_analysis_date.is_none());
        assert!(report.names.is_empty());
    }

    #[test]
    fn missing_data_is_an_error() {
        assert!(parse_report("domain", "x.test", &json!({}), "p".to_owned()).is_err());
    }
}
