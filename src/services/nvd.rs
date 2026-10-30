//! NVD CVE API 2.0 client.
//!
//! Responses are cached in SQLite (`nvd:*`), so a busy community never hits the
//! NVD rate limit. Without an API key NVD allows five requests per 30 seconds:
//! calls are serialised through a global pacer that keeps 6.5 s between them.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use url::form_urlencoded::Serializer as UrlEncoder;

use crate::bot::SharedState;
use crate::util::text::truncate;

const API_BASE: &str = "https://services.nvd.nist.gov/rest/json/cves/2.0";
const CVE_TTL_SECS: i64 = 24 * 60 * 60;
const LATEST_TTL_SECS: i64 = 60 * 60;
const SEARCH_TTL_SECS: i64 = 6 * 60 * 60;
const MAX_LIMIT: usize = 10;
const KEYLESS_GAP: Duration = Duration::from_millis(6_500);
const KEYED_GAP: Duration = Duration::from_millis(700);

/// Last reserved NVD slot. Reserving `now + wait` before sleeping keeps
/// concurrent lookups spaced even when several commands arrive at once.
static PACER: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cve {
    pub id: String,
    pub description: String,
    pub score: Option<f64>,
    pub severity: Option<String>,
    pub vector: Option<String>,
    pub published: String,
    pub last_modified: String,
    pub references: Vec<String>,
    pub weaknesses: Vec<String>,
}

/// Look up a single CVE by identifier (`CVE-YYYY-NNNN`).
pub async fn cve_by_id(state: &SharedState, id: &str) -> Result<Cve> {
    let id = id.trim().to_ascii_uppercase();
    let key = format!("nvd:cve:{id}");
    if let Some(cve) = recall::<Cve>(state, &key) {
        return Ok(cve);
    }

    let body = get(state, &endpoint(&[("cveId", id.as_str())])).await?;
    let cve = parse_cves(&body)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No CVE found for `{id}`."))?;

    remember(state, &key, &cve, CVE_TTL_SECS);
    Ok(cve)
}

/// Most recently published CVEs, optionally filtered by severity.
pub async fn latest(state: &SharedState, severity: Option<&str>, limit: usize) -> Result<Vec<Cve>> {
    let limit = limit.clamp(1, MAX_LIMIT);
    let severity = severity
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty() && value != "any");

    let key = format!(
        "nvd:latest:{}:{limit}",
        severity.as_deref().unwrap_or("any")
    );
    if let Some(cves) = recall::<Vec<Cve>>(state, &key) {
        return Ok(cves);
    }

    // Over-fetch so the client-side severity filter still fills the page.
    let fetch = (limit * 4).clamp(limit, 50).to_string();
    // NVD returns results in publication order; without a window we would get
    // the oldest CVEs in the database. Ask for the last 14 days explicitly.
    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(14);
    let start = start.format("%Y-%m-%dT%H:%M:%S.000+00:00").to_string();
    let end = end.format("%Y-%m-%dT%H:%M:%S.000+00:00").to_string();
    let body = get(
        state,
        &endpoint(&[
            ("pubStartDate", start.as_str()),
            ("pubEndDate", end.as_str()),
            ("resultsPerPage", fetch.as_str()),
        ]),
    )
    .await?;
    let mut cves = parse_cves(&body);

    if let Some(severity) = severity.as_deref() {
        cves.retain(|cve| {
            cve.severity
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(severity))
        });
    }
    cves.sort_by(|left, right| right.published.cmp(&left.published));
    cves.truncate(limit);

    if cves.is_empty() {
        bail!("No recent CVEs matched that severity.");
    }

    remember(state, &key, &cves, LATEST_TTL_SECS);
    Ok(cves)
}

/// Keyword search across CVE descriptions.
pub async fn search(state: &SharedState, query: &str, limit: usize) -> Result<Vec<Cve>> {
    let query = query.trim();
    if query.is_empty() {
        bail!("Enter a keyword to search CVEs.");
    }
    let query: String = query.chars().take(120).collect();
    let limit = limit.clamp(1, MAX_LIMIT);
    let key = format!("nvd:search:{query}:{limit}");
    if let Some(cves) = recall::<Vec<Cve>>(state, &key) {
        return Ok(cves);
    }

    let per_page = (limit * 2).clamp(limit, 50).to_string();
    let body = get(
        state,
        &endpoint(&[
            ("keywordSearch", query.as_str()),
            ("resultsPerPage", per_page.as_str()),
        ]),
    )
    .await?;
    let mut cves = parse_cves(&body);
    cves.truncate(limit);

    if cves.is_empty() {
        bail!("No CVE matched `{query}`.");
    }

    remember(state, &key, &cves, SEARCH_TTL_SECS);
    Ok(cves)
}

fn endpoint(params: &[(&str, &str)]) -> String {
    let mut query = UrlEncoder::new(String::new());
    for (key, value) in params {
        query.append_pair(key, value);
    }
    format!("{API_BASE}?{}", query.finish())
}

fn api_key(state: &SharedState) -> Option<&str> {
    state
        .config
        .intel
        .nvd_api_key
        .as_ref()
        .map(crate::config::Secret::expose)
        .filter(|key| !key.is_empty())
}

/// Hold the caller until the global NVD slot is free, then reserve the next one.
async fn pace(state: &SharedState) {
    let gap = if api_key(state).is_some() {
        KEYED_GAP
    } else {
        KEYLESS_GAP
    };
    let now = Instant::now();
    let wait = {
        let mut last = PACER.lock().await;
        let wait = match *last {
            Some(previous) => gap.saturating_sub(now.saturating_duration_since(previous)),
            None => Duration::ZERO,
        };
        *last = Some(now + wait);
        wait
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

async fn get(state: &SharedState, url: &str) -> Result<Value> {
    pace(state).await;
    let mut request = state.http.get(url);
    if let Some(key) = api_key(state) {
        request = request.header("apiKey", key);
    }

    let response = request.send().await.map_err(|error| {
        // `without_url` keeps a malformed request from echoing the full URL.
        tracing::debug!("NVD request failed: {}", error.without_url());
        anyhow::anyhow!("NVD is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        tracing::debug!("NVD rate limit (body: {})", truncate(&body, 200));
        bail!("NVD is rate limiting requests — try again in a minute.");
    }
    if !status.is_success() {
        tracing::debug!("NVD HTTP {status} (body: {})", truncate(&body, 200));
        bail!("NVD returned HTTP {status}.");
    }

    serde_json::from_str(&body).context("parsing NVD response")
}

fn parse_cves(body: &Value) -> Vec<Cve> {
    body.get("vulnerabilities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| parse_cve(item.get("cve")?))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_cve(cve: &Value) -> Option<Cve> {
    let id = cve.get("id").and_then(Value::as_str)?.to_owned();

    let description = cve
        .get("descriptions")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|entry| entry.get("lang").and_then(Value::as_str) == Some("en"))
        })
        .and_then(|entry| entry.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("No description provided.")
        .to_owned();

    let (score, severity, vector) = metrics(cve);

    let mut weaknesses: Vec<String> = Vec::new();
    if let Some(items) = cve.get("weaknesses").and_then(Value::as_array) {
        for item in items {
            let Some(descriptions) = item.get("description").and_then(Value::as_array) else {
                continue;
            };
            for description in descriptions {
                let Some(value) = description.get("value").and_then(Value::as_str) else {
                    continue;
                };
                if value == "NVD-CWE-noinfo" || value == "NVD-CWE-Other" {
                    continue;
                }
                if !weaknesses.iter().any(|existing| existing == value) {
                    weaknesses.push(value.to_owned());
                }
            }
        }
    }

    let references = cve
        .get("references")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("url").and_then(Value::as_str))
                .take(3)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Some(Cve {
        id,
        description,
        score,
        severity,
        vector,
        published: cve
            .get("published")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        last_modified: cve
            .get("lastModified")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        references,
        weaknesses,
    })
}

/// `(score, severity, vector)` from CVSS v3.1, then v3.0.
fn metrics(cve: &Value) -> (Option<f64>, Option<String>, Option<String>) {
    let metrics = cve.get("metrics");
    for key in ["cvssMetricV31", "cvssMetricV30"] {
        let Some(data) = metrics
            .and_then(|metrics| metrics.get(key))
            .and_then(Value::as_array)
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.get("cvssData"))
        else {
            continue;
        };
        let score = data.get("baseScore").and_then(Value::as_f64);
        let severity = data
            .get("baseSeverity")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let vector = data
            .get("vectorString")
            .and_then(Value::as_str)
            .map(str::to_owned);
        return (score, severity, vector);
    }
    (None, None, None)
}

fn recall<T: serde::de::DeserializeOwned>(state: &SharedState, key: &str) -> Option<T> {
    let payload = state.db.cache_get(key).ok().flatten()?;
    serde_json::from_str(&payload).ok()
}

fn remember<T: Serialize>(state: &SharedState, key: &str, value: &T, ttl_secs: i64) {
    match serde_json::to_string(value) {
        Ok(payload) => {
            if let Err(error) = state.db.cache_put(key, &payload, ttl_secs) {
                tracing::warn!("NVD cache write failed: {error}");
            }
        }
        Err(error) => tracing::warn!("NVD cache serialisation failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_full_cve() {
        let body = json!({
            "vulnerabilities": [{
                "cve": {
                    "id": "CVE-2024-3094",
                    "published": "2024-03-29T18:15:00.000",
                    "lastModified": "2024-04-03T14:00:00.000",
                    "descriptions": [
                        {"lang": "es", "value": "descripcion"},
                        {"lang": "en", "value": "Malicious code in xz/liblzma."}
                    ],
                    "metrics": {
                        "cvssMetricV31": [{
                            "cvssData": {
                                "baseScore": 10.0,
                                "baseSeverity": "CRITICAL",
                                "vectorString": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"
                            }
                        }]
                    },
                    "weaknesses": [
                        {"description": [
                            {"lang": "en", "value": "CWE-506"},
                            {"lang": "en", "value": "NVD-CWE-noinfo"}
                        ]},
                        {"description": [{"lang": "en", "value": "CWE-506"}]}
                    ],
                    "references": [
                        {"url": "https://a.example"},
                        {"url": "https://b.example"},
                        {"url": "https://c.example"},
                        {"url": "https://d.example"}
                    ]
                }
            }]
        });

        let cves = parse_cves(&body);
        assert_eq!(cves.len(), 1);
        let cve = &cves[0];
        assert_eq!(cve.id, "CVE-2024-3094");
        assert_eq!(cve.description, "Malicious code in xz/liblzma.");
        assert_eq!(cve.score, Some(10.0));
        assert_eq!(cve.severity.as_deref(), Some("CRITICAL"));
        assert_eq!(
            cve.vector.as_deref(),
            Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H")
        );
        assert_eq!(cve.published, "2024-03-29T18:15:00.000");
        assert_eq!(cve.last_modified, "2024-04-03T14:00:00.000");
        assert_eq!(cve.weaknesses, vec!["CWE-506".to_owned()]);
        assert_eq!(cve.references.len(), 3);
        assert_eq!(cve.references[0], "https://a.example");
    }

    #[test]
    fn falls_back_to_cvss_v30() {
        let body = json!({
            "vulnerabilities": [{"cve": {
                "id": "CVE-2021-44228",
                "descriptions": [{"lang": "en", "value": "Log4Shell"}],
                "metrics": {"cvssMetricV30": [{"cvssData": {
                    "baseScore": 9.8,
                    "baseSeverity": "CRITICAL",
                    "vectorString": "CVSS:3.0/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"
                }}]}
            }}]
        });
        let cves = parse_cves(&body);
        assert_eq!(cves[0].score, Some(9.8));
        assert_eq!(cves[0].severity.as_deref(), Some("CRITICAL"));
        assert!(
            cves[0]
                .vector
                .as_deref()
                .is_some_and(|v| v.starts_with("CVSS:3.0"))
        );
    }

    #[test]
    fn tolerates_missing_fields() {
        let body = json!({"vulnerabilities": [{"cve": {"id": "CVE-1999-0001"}}]});
        let cves = parse_cves(&body);
        assert_eq!(cves.len(), 1);
        assert_eq!(cves[0].description, "No description provided.");
        assert!(cves[0].score.is_none());
        assert!(cves[0].severity.is_none());
        assert_eq!(cves[0].published, "unknown");
        assert!(cves[0].weaknesses.is_empty());
        assert!(cves[0].references.is_empty());
    }

    #[test]
    fn ignores_entries_without_id() {
        let body = json!({"vulnerabilities": [{"cve": {}}, {"cve": {"id": "CVE-2020-0001"}}]});
        let cves = parse_cves(&body);
        assert_eq!(cves.len(), 1);
        assert_eq!(cves[0].id, "CVE-2020-0001");
        assert!(parse_cves(&json!({})).is_empty());
    }

    #[test]
    fn endpoint_encodes_search_terms() {
        let url = endpoint(&[("keywordSearch", "log4j & friends")]);
        assert!(url.starts_with(API_BASE));
        assert!(url.contains("keywordSearch=log4j+%26+friends"));
    }
}
