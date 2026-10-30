//! IPinfo client.
//!
//! Optional enrichment: the `/intel ip` command merges this with RDAP and works
//! when IPinfo is unavailable. The token is only ever placed in the query string
//! and never logged.

use std::net::IpAddr;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::form_urlencoded::Serializer as UrlEncoder;

use crate::bot::SharedState;
use crate::util::text::truncate;

const API_BASE: &str = "https://ipinfo.io";
const IPINFO_TTL_SECS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IpInfo {
    pub ip: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub org: Option<String>,
    pub hostname: Option<String>,
    pub timezone: Option<String>,
}

/// Look up geolocation and ASN data for an IP address.
pub async fn lookup(state: &SharedState, raw: &str) -> Result<IpInfo> {
    let address = IpAddr::from_str(raw.trim()).map_err(|_| {
        anyhow::anyhow!("`{}` is not a valid IP address.", truncate(raw.trim(), 80))
    })?;
    let address = address.to_string();

    let key = format!("ipinfo:{address}");
    if let Some(info) = recall(state, &key) {
        return Ok(info);
    }

    let mut url = format!("{API_BASE}/{address}/json");
    if let Some(token) = state
        .config
        .intel
        .ipinfo_token
        .as_ref()
        .map(crate::config::Secret::expose)
        .filter(|token| !token.is_empty())
    {
        let mut query = UrlEncoder::new(String::new());
        query.append_pair("token", token);
        url.push('?');
        url.push_str(&query.finish());
    }

    let response = state.http.get(&url).send().await.map_err(|error| {
        tracing::debug!("IPinfo request failed: {}", error.without_url());
        anyhow::anyhow!("IPinfo is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        tracing::debug!("IPinfo rate limit (body: {})", truncate(&body, 200));
        bail!("IPinfo is rate limiting requests — try again in a minute.");
    }
    if !status.is_success() {
        tracing::debug!("IPinfo HTTP {status} (body: {})", truncate(&body, 200));
        bail!("IPinfo returned no data (HTTP {status}).");
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("IPinfo JSON parse failed: {error}");
        anyhow::anyhow!("IPinfo returned an unreadable response.")
    })?;
    let info = parse_ipinfo(&value);
    remember(state, &key, &info);
    Ok(info)
}

fn parse_ipinfo(body: &Value) -> IpInfo {
    IpInfo {
        ip: field(body, "ip"),
        city: field(body, "city"),
        region: field(body, "region"),
        country: field(body, "country"),
        org: field(body, "org"),
        hostname: field(body, "hostname"),
        timezone: field(body, "timezone"),
    }
}

fn field(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn recall(state: &SharedState, key: &str) -> Option<IpInfo> {
    let payload = state.db.cache_get(key).ok().flatten()?;
    serde_json::from_str(&payload).ok()
}

fn remember(state: &SharedState, key: &str, value: &IpInfo) {
    match serde_json::to_string(value) {
        Ok(payload) => {
            if let Err(error) = state.db.cache_put(key, &payload, IPINFO_TTL_SECS) {
                tracing::warn!("IPinfo cache write failed: {error}");
            }
        }
        Err(error) => tracing::warn!("IPinfo cache serialisation failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_full_response() {
        let body = json!({
            "ip": "1.1.1.1",
            "city": "Sydney",
            "region": "New South Wales",
            "country": "AU",
            "loc": "-33.8838,151.2079",
            "org": "AS13335 Cloudflare, Inc.",
            "postal": "1000",
            "timezone": "Australia/Sydney",
            "hostname": "one.one.one.one"
        });
        let info = parse_ipinfo(&body);
        assert_eq!(info.ip.as_deref(), Some("1.1.1.1"));
        assert_eq!(info.city.as_deref(), Some("Sydney"));
        assert_eq!(info.region.as_deref(), Some("New South Wales"));
        assert_eq!(info.country.as_deref(), Some("AU"));
        assert_eq!(info.org.as_deref(), Some("AS13335 Cloudflare, Inc."));
        assert_eq!(info.hostname.as_deref(), Some("one.one.one.one"));
        assert_eq!(info.timezone.as_deref(), Some("Australia/Sydney"));
    }

    #[test]
    fn tolerates_partial_response() {
        let info = parse_ipinfo(&json!({"ip": "8.8.8.8", "country": "US"}));
        assert_eq!(info.ip.as_deref(), Some("8.8.8.8"));
        assert_eq!(info.country.as_deref(), Some("US"));
        assert!(info.city.is_none());
        assert!(info.org.is_none());

        let empty = parse_ipinfo(&json!({"city": "  "}));
        assert!(empty.city.is_none());
    }
}
