//! Passive recon clients: certificate transparency, Shodan InternetDB, the
//! Wayback Machine CDX API and urlscan.io.
//!
//! Safety rule: every request goes to a fixed, well-known passive API host.
//! User input is only ever interpolated into the query string of those hosts,
//! so no packet is sent to the scanned target and there is no generic URL
//! fetcher in this module. IP queries are restricted to public addresses so the
//! command cannot be used for internal recon.
//!
//! Responses are cached in `intel_cache` (crt.sh 6h, InternetDB 1h, Wayback 6h,
//! urlscan 1h). Cache failures are tolerated: they only cost a fresh request.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bot::SharedState;
use crate::util::text::truncate;

const USER_AGENT: &str = "CipherBot/1.0 (+https://github.com/cleeryy/cipherbot)";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

const CRTSH_URL: &str = "https://crt.sh/";
const INTERNETDB_URL: &str = "https://internetdb.shodan.io";
const WAYBACK_URL: &str = "https://web.archive.org/cdx/search/cdx";
const URLSCAN_URL: &str = "https://urlscan.io/api/v1/search/";

const SUBDOMAINS_TTL_SECS: i64 = 6 * 60 * 60;
const HOST_TTL_SECS: i64 = 60 * 60;
const WAYBACK_TTL_SECS: i64 = 6 * 60 * 60;
const URLSCAN_TTL_SECS: i64 = 60 * 60;

/// crt.sh is routinely slow and sometimes rate-limits, so it gets its own
/// long-timeout client instead of the shared 5s `state.http`.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!("recon HTTP client fell back to reqwest defaults: {error}");
            reqwest::Client::new()
        })
});

// One label: 1..=63 chars, alphanumeric at both ends, hyphens inside.
static LABEL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$").ok());

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubdomainReport {
    pub domain: String,
    /// Distinct subdomains, lowercase, sorted, apex excluded.
    pub names: Vec<String>,
    /// The apex itself appeared in the CT logs.
    pub apex_present: bool,
    /// `names.len()` plus the apex when present.
    pub total: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostReport {
    pub ip: String,
    pub ports: Vec<u16>,
    pub hostnames: Vec<String>,
    pub cpes: Vec<String>,
    pub vulns: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WaybackRow {
    pub original: String,
    pub timestamp: String,
    pub status: String,
    pub mimetype: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WaybackReport {
    pub domain: String,
    pub limit: usize,
    pub rows: Vec<WaybackRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UrlscanHit {
    pub url: String,
    pub time: String,
    pub malicious: u64,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UrlscanReport {
    pub domain: String,
    pub limit: usize,
    pub hits: Vec<UrlscanHit>,
}

/// Lowercase and validate a bare hostname. URLs, IPs, wildcards, underscores
/// and single-label names are all rejected.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let candidate = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if candidate.is_empty() || candidate.len() > 253 {
        return None;
    }
    if candidate.chars().any(|ch| {
        ch.is_whitespace() || matches!(ch, '/' | '\\' | '*' | '?' | '%' | ':' | '@' | '#')
    }) {
        return None;
    }
    if IpAddr::from_str(&candidate).is_ok() {
        return None;
    }

    let labels = candidate.split('.').collect::<Vec<_>>();
    if labels.len() < 2 {
        return None;
    }
    let pattern = LABEL_RE.as_ref()?;
    if labels.iter().any(|label| !pattern.is_match(label)) {
        return None;
    }
    Some(candidate)
}

/// True only for addresses that are globally routable. Loopback, private,
/// link-local, CGNAT, multicast, documentation and reserved ranges are refused
/// so `/recon ip` cannot be pointed at the homelab or a LAN.
pub fn is_public_ip(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_public_v4(*ip),
        IpAddr::V6(ip) => is_public_v6(*ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    let value = u32::from(ip);

    // 0.0.0.0/8 "this network", plus loopback, RFC1918, link-local, multicast,
    // broadcast and the three documentation prefixes.
    if octets[0] == 0
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
    {
        return false;
    }
    // 100.64.0.0/10 carrier-grade NAT.
    if (value & 0xFFC0_0000) == 0x6440_0000 {
        return false;
    }
    // 192.0.0.0/24 IETF protocol assignments.
    if (value & 0xFFFF_FF00) == 0xC000_0000 {
        return false;
    }
    // 198.18.0.0/15 benchmarking.
    if (value & 0xFFFF_FE00) == 0xC612_0000 {
        return false;
    }
    // 192.88.99.0/24 6to4 relay anycast.
    if (value & 0xFFFF_FF00) == 0xC058_6300 {
        return false;
    }
    // 240.0.0.0/4 reserved (255.255.255.255 already handled above).
    if (value & 0xF000_0000) == 0xF000_0000 {
        return false;
    }
    true
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let segments = ip.segments();

    // fc00::/7 unique local.
    if (segments[0] & 0xFE00) == 0xFC00 {
        return false;
    }
    // fe80::/10 link-local.
    if (segments[0] & 0xFFC0) == 0xFE80 {
        return false;
    }
    // 2001:db8::/32 documentation (also covered by std, kept explicit).
    if segments[0] == 0x2001 && segments[1] == 0x0DB8 {
        return false;
    }
    // 100::/64 discard-only.
    if segments[0] == 0x0100 && segments[1..4].iter().all(|segment| *segment == 0) {
        return false;
    }
    // 64:ff9b::/96 NAT64: the embedded IPv4 may be private.
    if segments[0] == 0x0064
        && segments[1] == 0xFF9B
        && segments[2..6].iter().all(|segment| *segment == 0)
    {
        return false;
    }
    // IPv4-mapped addresses (::ffff:0:0/96) inherit the IPv4 rules.
    if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xFFFF {
        let octets = ip.octets();
        return is_public_v4(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ));
    }
    // Deprecated IPv4-compatible ::/96.
    if segments[..6].iter().all(|segment| *segment == 0) {
        return false;
    }
    true
}

/// Parse a crt.sh JSON array into a sorted, deduplicated subdomain report.
///
/// `name_value` may hold several newline-separated names and wildcard entries;
/// both `name_value` and `common_name` are read, and only names at or under the
/// queried domain are kept. Missing or malformed fields are ignored.
pub fn parse_crtsh(domain: &str, body: &Value) -> SubdomainReport {
    let apex = domain.to_ascii_lowercase();
    let suffix = format!(".{apex}");
    let mut names = BTreeSet::new();
    let mut apex_present = false;

    for entry in body.as_array().into_iter().flatten() {
        for field in ["name_value", "common_name"] {
            let Some(raw) = entry.get(field).and_then(Value::as_str) else {
                continue;
            };
            for line in raw.split(['\n', '\r']) {
                let name = line
                    .trim()
                    .trim_start_matches("*.")
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_'))
                {
                    continue;
                }
                if name == apex {
                    apex_present = true;
                } else if name.len() > suffix.len() && name.ends_with(&suffix) {
                    names.insert(name);
                }
            }
        }
    }

    let total = names.len() + usize::from(apex_present);
    SubdomainReport {
        domain: apex,
        names: names.into_iter().collect(),
        apex_present,
        total,
    }
}

/// Map an InternetDB HTTP status to a user-facing message. `None` means the
/// body is a record and should be parsed.
pub fn internetdb_status_error(status: reqwest::StatusCode, ip: &str) -> Option<String> {
    match status {
        reqwest::StatusCode::OK => None,
        reqwest::StatusCode::NOT_FOUND => {
            Some(format!("Shodan InternetDB has no data for `{ip}`."))
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            Some("Shodan InternetDB is rate limiting requests — try again in a minute.".to_owned())
        }
        other => Some(format!("Shodan InternetDB lookup failed (HTTP {other}).")),
    }
}

/// Parse an InternetDB record, tolerating missing fields.
pub fn parse_internetdb(ip: &str, body: &Value) -> HostReport {
    HostReport {
        ip: ip.to_owned(),
        ports: body
            .get("ports")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_u64)
                    .filter_map(|port| u16::try_from(port).ok())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default(),
        hostnames: string_set(body.get("hostnames")),
        cpes: string_set(body.get("cpes")),
        vulns: string_set(body.get("vulns")),
        tags: string_set(body.get("tags")),
    }
}

/// Parse CDX rows (`fl=original,timestamp,statuscode,mimetype`). The first row
/// is the header; the parser maps columns by name and falls back to the fixed
/// order this client requests.
pub fn parse_wayback(domain: &str, limit: usize, body: &Value) -> WaybackReport {
    let limit = limit.max(1);
    let mut rows = Vec::new();
    let Some(entries) = body.as_array() else {
        return WaybackReport {
            domain: domain.to_owned(),
            limit,
            rows,
        };
    };

    let mut entries = entries.iter();
    let header = entries
        .next()
        .and_then(Value::as_array)
        .map(|columns| {
            columns
                .iter()
                .map(|column| column.as_str().unwrap_or_default().to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let column = |name: &str, fallback: usize| {
        header
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or(fallback)
    };
    let original_at = column("original", 0);
    let timestamp_at = column("timestamp", 1);
    let status_at = column("statuscode", 2);
    let mimetype_at = column("mimetype", 3);

    for entry in entries {
        if rows.len() >= limit {
            break;
        }
        let Some(fields) = entry.as_array() else {
            continue;
        };
        let cell = |index: usize| {
            fields
                .get(index)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        let original = cell(original_at);
        if original.is_empty() {
            continue;
        }
        rows.push(WaybackRow {
            original,
            timestamp: cell(timestamp_at),
            status: cell(status_at),
            mimetype: cell(mimetype_at),
        });
    }

    WaybackReport {
        domain: domain.to_owned(),
        limit,
        rows,
    }
}

/// Parse urlscan.io `results[]`, tolerating missing fields and non-string ids.
pub fn parse_urlscan(domain: &str, limit: usize, body: &Value) -> UrlscanReport {
    let limit = limit.max(1);
    let hits = body
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .take(limit)
                .filter_map(|result| {
                    let url = result
                        .get("page")
                        .and_then(|page| page.get("url"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())?;
                    let time = result
                        .get("task")
                        .and_then(|task| task.get("time"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let malicious = result
                        .get("stats")
                        .and_then(|stats| stats.get("malicious"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let id = result
                        .get("_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .chars()
                        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
                        .collect::<String>();
                    Some(UrlscanHit {
                        url: url.trim().to_owned(),
                        time: time.to_owned(),
                        malicious,
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    UrlscanReport {
        domain: domain.to_owned(),
        limit,
        hits,
    }
}

/// Subdomains from certificate transparency logs (crt.sh), cached for 6h.
pub async fn subdomains(state: &SharedState, domain: &str) -> Result<SubdomainReport> {
    let domain = normalize_domain(domain).ok_or_else(|| {
        anyhow::anyhow!(
            "`{}` is not a valid domain name.",
            truncate(domain.trim(), 80)
        )
    })?;

    let key = format!("recon:subdomains:{domain}");
    if let Some(report) = recall::<SubdomainReport>(state, &key) {
        return Ok(report);
    }

    let url = format!("{CRTSH_URL}?q=%25.{domain}&output=json");
    let response = CLIENT.get(&url).send().await.map_err(|error| {
        tracing::debug!("crt.sh request failed: {}", error.without_url());
        anyhow::anyhow!("crt.sh is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
    {
        bail!("crt.sh is busy right now — try again in a minute.");
    }
    if !status.is_success() {
        tracing::debug!("crt.sh returned HTTP {status}");
        bail!("crt.sh lookup failed (HTTP {status}).");
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("crt.sh JSON parse failed: {error}");
        anyhow::anyhow!("crt.sh returned an unreadable response — try again shortly.")
    })?;
    if !value.is_array() {
        bail!("crt.sh returned an unexpected response — try again shortly.");
    }

    let report = parse_crtsh(&domain, &value);
    remember(state, &key, &report, SUBDOMAINS_TTL_SECS);
    Ok(report)
}

/// Shodan InternetDB record for a public IP, cached for 1h.
pub async fn internetdb(state: &SharedState, address: IpAddr) -> Result<HostReport> {
    if !is_public_ip(&address) {
        bail!("`{address}` is not a public address — InternetDB only covers routable IPs.");
    }
    let ip = address.to_string();

    let key = format!("recon:internetdb:{ip}");
    if let Some(report) = recall::<HostReport>(state, &key) {
        return Ok(report);
    }

    let url = format!("{INTERNETDB_URL}/{ip}");
    let response = CLIENT.get(&url).send().await.map_err(|error| {
        tracing::debug!("InternetDB request failed: {}", error.without_url());
        anyhow::anyhow!("Shodan InternetDB is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if let Some(message) = internetdb_status_error(status, &ip) {
        tracing::debug!("InternetDB returned HTTP {status}");
        bail!(message);
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("InternetDB JSON parse failed: {error}");
        anyhow::anyhow!("InternetDB returned an unreadable record.")
    })?;

    let report = parse_internetdb(&ip, &value);
    remember(state, &key, &report, HOST_TTL_SECS);
    Ok(report)
}

/// Wayback CDX snapshots for `*.domain`, cached for 6h per (domain, limit).
pub async fn wayback(state: &SharedState, domain: &str, limit: usize) -> Result<WaybackReport> {
    let domain = normalize_domain(domain).ok_or_else(|| {
        anyhow::anyhow!(
            "`{}` is not a valid domain name.",
            truncate(domain.trim(), 80)
        )
    })?;
    let limit = limit.clamp(1, 50);

    let key = format!("recon:wayback:{domain}:{limit}");
    if let Some(report) = recall::<WaybackReport>(state, &key) {
        return Ok(report);
    }

    let url = format!(
        "{WAYBACK_URL}?url=*.{domain}&output=json&fl=original,timestamp,statuscode,mimetype\
         &collapse=urlkey&limit={limit}"
    );
    let response = CLIENT.get(&url).send().await.map_err(|error| {
        tracing::debug!("Wayback request failed: {}", error.without_url());
        anyhow::anyhow!("The Wayback Machine is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
    {
        bail!("The Wayback Machine is busy right now — try again in a minute.");
    }
    if !status.is_success() {
        tracing::debug!("Wayback returned HTTP {status}");
        bail!("Wayback lookup failed (HTTP {status}).");
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("Wayback JSON parse failed: {error}");
        anyhow::anyhow!("The Wayback Machine returned an unreadable response.")
    })?;

    let report = parse_wayback(&domain, limit, &value);
    remember(state, &key, &report, WAYBACK_TTL_SECS);
    Ok(report)
}

/// Public urlscan.io scans for a domain, cached for 1h per (domain, limit).
pub async fn urlscan(state: &SharedState, domain: &str, limit: usize) -> Result<UrlscanReport> {
    let domain = normalize_domain(domain).ok_or_else(|| {
        anyhow::anyhow!(
            "`{}` is not a valid domain name.",
            truncate(domain.trim(), 80)
        )
    })?;
    let limit = limit.clamp(1, 15);

    let key = format!("recon:urlscan:{domain}:{limit}");
    if let Some(report) = recall::<UrlscanReport>(state, &key) {
        return Ok(report);
    }

    let url = format!("{URLSCAN_URL}?q=domain%3A{domain}&size={limit}");
    let response = CLIENT.get(&url).send().await.map_err(|error| {
        tracing::debug!("urlscan request failed: {}", error.without_url());
        anyhow::anyhow!("urlscan.io is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!("urlscan.io is rate limiting requests — try again later.");
    }
    if !status.is_success() {
        tracing::debug!("urlscan returned HTTP {status}");
        bail!("urlscan.io lookup failed (HTTP {status}).");
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("urlscan JSON parse failed: {error}");
        anyhow::anyhow!("urlscan.io returned an unreadable response.")
    })?;

    let report = parse_urlscan(&domain, limit, &value);
    remember(state, &key, &report, URLSCAN_TTL_SECS);
    Ok(report)
}

fn string_set(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

fn recall<T: serde::de::DeserializeOwned>(state: &SharedState, key: &str) -> Option<T> {
    let payload = match state.db.cache_get(key) {
        Ok(payload) => payload?,
        Err(error) => {
            tracing::debug!("recon cache read failed: {error}");
            return None;
        }
    };
    match serde_json::from_str(&payload) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::debug!("recon cache payload unreadable: {error}");
            None
        }
    }
}

fn remember<T: Serialize>(state: &SharedState, key: &str, value: &T, ttl_secs: i64) {
    let payload = match serde_json::to_string(value) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!("recon cache serialisation failed: {error}");
            return;
        }
    };
    if let Err(error) = state.db.cache_put(key, &payload, ttl_secs) {
        tracing::debug!("recon cache write failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_normal_hostnames() {
        assert_eq!(
            normalize_domain(" Example.COM. "),
            Some("example.com".to_owned())
        );
        assert_eq!(
            normalize_domain("a-b.sub.example.co.uk"),
            Some("a-b.sub.example.co.uk".to_owned())
        );
        assert_eq!(
            normalize_domain("xn--p1ai.example"),
            Some("xn--p1ai.example".to_owned())
        );
        assert_eq!(normalize_domain("1.2.3.4"), None);
        assert_eq!(normalize_domain("2001:db8::1"), None);
        assert_eq!(normalize_domain("example"), None);
        assert_eq!(normalize_domain(""), None);
        assert_eq!(normalize_domain("   "), None);
        assert_eq!(normalize_domain("https://example.com"), None);
        assert_eq!(normalize_domain("example.com/path"), None);
        assert_eq!(normalize_domain("*.example.com"), None);
        assert_eq!(normalize_domain("bad_domain.example"), None);
        assert_eq!(normalize_domain("-bad.example"), None);
        assert_eq!(normalize_domain("bad-.example"), None);
        assert_eq!(normalize_domain("a..example"), None);
        let long_label = "a".repeat(64);
        assert_eq!(normalize_domain(&format!("{long_label}.example")), None);
        let long_name = format!("{}.example", "a".repeat(250));
        assert_eq!(normalize_domain(&long_name), None);
    }

    #[test]
    fn public_ip_filter_rejects_non_routable_ranges() {
        for public in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "::ffff:1.2.3.4",
        ] {
            let address = IpAddr::from_str(public).expect("fixture");
            assert!(is_public_ip(&address), "{public} should be public");
        }

        for private in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "169.254.10.1",
            "172.16.5.4",
            "172.31.255.255",
            "192.0.0.9",
            "192.0.2.5",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:10.0.0.1",
            "::192.168.1.1",
            "64:ff9b::a00:1",
            "100::1",
            "2001:db8::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
        ] {
            let address = IpAddr::from_str(private).expect("fixture");
            assert!(!is_public_ip(&address), "{private} should be refused");
        }
    }

    #[test]
    fn parses_crtsh_fixture() {
        let body = json!([
            {
                "name_value": "www.example.com\n*.example.com",
                "common_name": "example.com"
            },
            {
                "name_value": "WWW.EXAMPLE.COM\napi.example.com",
                "common_name": "api.example.com"
            },
            {
                "name_value": "*.dev.example.com\nnotexample.com\nevil.com",
                "common_name": ""
            },
            {"name_value": null},
            {}
        ]);

        let report = parse_crtsh("example.com", &body);
        assert_eq!(report.domain, "example.com");
        assert_eq!(
            report.names,
            vec![
                "api.example.com".to_owned(),
                "dev.example.com".to_owned(),
                "www.example.com".to_owned(),
            ]
        );
        assert!(report.apex_present);
        assert_eq!(report.total, 4);
    }

    #[test]
    fn crtsh_tolerates_odd_bodies() {
        let report = parse_crtsh("example.com", &json!({}));
        assert!(report.names.is_empty());
        assert!(!report.apex_present);
        assert_eq!(report.total, 0);

        let report = parse_crtsh("example.com", &json!([{"name_value": "www.example.com"}]));
        assert_eq!(report.names, vec!["www.example.com".to_owned()]);
        assert!(!report.apex_present);
        assert_eq!(report.total, 1);
    }

    #[test]
    fn internetdb_statuses_are_mapped() {
        assert_eq!(
            internetdb_status_error(reqwest::StatusCode::OK, "1.2.3.4"),
            None
        );
        let missing =
            internetdb_status_error(reqwest::StatusCode::NOT_FOUND, "1.2.3.4").expect("404");
        assert!(missing.contains("no data"), "{missing}");
        let limited = internetdb_status_error(reqwest::StatusCode::TOO_MANY_REQUESTS, "1.2.3.4")
            .expect("429");
        assert!(limited.to_lowercase().contains("rate limit"), "{limited}");
        let failed =
            internetdb_status_error(reqwest::StatusCode::BAD_GATEWAY, "1.2.3.4").expect("502");
        assert!(failed.contains("502"), "{failed}");
    }

    #[test]
    fn parses_internetdb_fixture() {
        let body = json!({
            "ports": [443, 80, 443, 8080, 70000],
            "hostnames": ["example.com", "www.example.com", "example.com"],
            "cpes": ["cpe:/a:nginx:nginx"],
            "vulns": ["CVE-2024-0002", "CVE-2024-0001"],
            "tags": ["cloud", "https"]
        });
        let report = parse_internetdb("93.184.216.34", &body);
        assert_eq!(report.ip, "93.184.216.34");
        assert_eq!(report.ports, vec![80, 443, 8080]);
        assert_eq!(
            report.hostnames,
            vec!["example.com".to_owned(), "www.example.com".to_owned()]
        );
        assert_eq!(report.cpes, vec!["cpe:/a:nginx:nginx".to_owned()]);
        assert_eq!(
            report.vulns,
            vec!["CVE-2024-0001".to_owned(), "CVE-2024-0002".to_owned()]
        );
        assert_eq!(report.tags, vec!["cloud".to_owned(), "https".to_owned()]);
    }

    #[test]
    fn internetdb_tolerates_missing_fields() {
        let report = parse_internetdb("1.2.3.4", &json!({}));
        assert!(report.ports.is_empty());
        assert!(report.hostnames.is_empty());
        assert!(report.vulns.is_empty());
        assert!(report.tags.is_empty());
    }

    #[test]
    fn parses_wayback_rows_and_skips_header() {
        let body = json!([
            ["original", "timestamp", "statuscode", "mimetype"],
            ["https://example.com/", "20240101000000", "200", "text/html"],
            [
                "https://example.com/login",
                "20230101000000",
                "301",
                "text/html"
            ],
            [
                "https://example.com/third",
                "20220101000000",
                "404",
                "text/plain"
            ]
        ]);
        let report = parse_wayback("example.com", 2, &body);
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].original, "https://example.com/");
        assert_eq!(report.rows[0].timestamp, "20240101000000");
        assert_eq!(report.rows[0].status, "200");
        assert_eq!(report.rows[0].mimetype, "text/html");
        assert_eq!(report.rows[1].original, "https://example.com/login");
    }

    #[test]
    fn wayback_tolerates_missing_fields_and_odd_bodies() {
        let body = json!([
            ["original", "timestamp"],
            ["https://example.com/"],
            {},
            ["", "20240101000000"]
        ]);
        let report = parse_wayback("example.com", 15, &body);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].original, "https://example.com/");
        assert!(report.rows[0].status.is_empty());

        let empty = parse_wayback("example.com", 15, &json!([]));
        assert!(empty.rows.is_empty());
        let odd = parse_wayback("example.com", 15, &json!({"detail": "nope"}));
        assert!(odd.rows.is_empty());
    }

    #[test]
    fn parses_urlscan_fixture() {
        let body = json!({
            "results": [
                {
                    "page": {"url": "https://example.com/login"},
                    "task": {"time": "2026-10-01T12:00:00.000Z"},
                    "stats": {"malicious": 2},
                    "_id": "abc-123"
                },
                {
                    "page": {"url": "https://example.com/"},
                    "task": {"time": "2026-09-30T09:30:00.000Z"},
                    "_id": "def-456"
                },
                {
                    "task": {"time": "2026-09-29T09:30:00.000Z"},
                    "_id": "missing-page"
                }
            ],
            "total": 3
        });
        let report = parse_urlscan("example.com", 10, &body);
        assert_eq!(report.hits.len(), 2);
        assert_eq!(report.hits[0].url, "https://example.com/login");
        assert_eq!(report.hits[0].time, "2026-10-01T12:00:00.000Z");
        assert_eq!(report.hits[0].malicious, 2);
        assert_eq!(report.hits[0].id, "abc-123");
        assert_eq!(report.hits[1].malicious, 0);

        let capped = parse_urlscan("example.com", 1, &body);
        assert_eq!(capped.hits.len(), 1);
    }

    #[test]
    fn urlscan_tolerates_odd_bodies() {
        assert!(parse_urlscan("example.com", 10, &json!({})).hits.is_empty());
        assert!(
            parse_urlscan("example.com", 10, &json!({"results": "nope"}))
                .hits
                .is_empty()
        );
        assert!(
            parse_urlscan("example.com", 10, &json!({"results": []}))
                .hits
                .is_empty()
        );
    }
}
