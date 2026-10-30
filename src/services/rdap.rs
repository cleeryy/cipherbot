//! RDAP domain/IP lookup client.
//!
//! `rdap.org` redirects to the authoritative registry, so a single fixed host is
//! enough. Records are cached for six hours; RDAP data changes slowly and some
//! registries are aggressive with rate limits.

use std::net::IpAddr;
use std::str::FromStr;
use std::sync::LazyLock;

use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bot::SharedState;
use crate::util::text::truncate;

const DOMAIN_BASE: &str = "https://rdap.org/domain";
const IP_BASE: &str = "https://rdap.org/ip";
const RDAP_TTL_SECS: i64 = 6 * 60 * 60;

// Same shape as the spec pattern, minus the look-ahead: the `regex` crate does
// not support look-around, so the 1..=253 total length is checked separately.
static DOMAIN_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$").ok());

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RdapDomain {
    pub domain: String,
    pub registrar: Option<String>,
    pub created: Option<String>,
    pub expires: Option<String>,
    pub updated: Option<String>,
    pub statuses: Vec<String>,
    pub nameservers: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RdapIp {
    pub handle: Option<String>,
    pub name: Option<String>,
    pub country: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub cidr: Vec<String>,
    pub organization: Option<String>,
}

/// Lowercase and validate a domain name. Returns `None` for anything that is
/// not a plain ASCII hostname (URLs, IPs and wildcards are rejected).
pub fn normalize_domain(raw: &str) -> Option<String> {
    let candidate = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let pattern = DOMAIN_RE.as_ref()?;
    if candidate.is_empty() || candidate.len() > 253 || !pattern.is_match(&candidate) {
        return None;
    }
    Some(candidate)
}

/// Look up a domain registration record.
pub async fn domain(state: &SharedState, raw: &str) -> Result<RdapDomain> {
    let domain = normalize_domain(raw).ok_or_else(|| {
        anyhow::anyhow!("`{}` is not a valid domain name.", truncate(raw.trim(), 80))
    })?;

    let key = format!("rdap:domain:{domain}");
    if let Some(record) = recall::<RdapDomain>(state, &key) {
        return Ok(record);
    }

    let body = get(state, &format!("{DOMAIN_BASE}/{domain}"), "domain").await?;
    let record = parse_domain(&domain, &body);
    remember(state, &key, &record);
    Ok(record)
}

/// Look up an IP network record.
pub async fn ip(state: &SharedState, raw: &str) -> Result<RdapIp> {
    let address = IpAddr::from_str(raw.trim()).map_err(|_| {
        anyhow::anyhow!("`{}` is not a valid IP address.", truncate(raw.trim(), 80))
    })?;
    let address = address.to_string();

    let key = format!("rdap:ip:{address}");
    if let Some(record) = recall::<RdapIp>(state, &key) {
        return Ok(record);
    }

    let body = get(state, &format!("{IP_BASE}/{address}"), "IP").await?;
    let record = parse_ip(&body);
    remember(state, &key, &record);
    Ok(record)
}

async fn get(state: &SharedState, url: &str, kind: &str) -> Result<Value> {
    let response = state.http.get(url).send().await.map_err(|error| {
        tracing::debug!("RDAP {kind} request failed: {}", error.without_url());
        anyhow::anyhow!("RDAP is unreachable right now.")
    })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("No RDAP {kind} record found.");
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        tracing::debug!("RDAP rate limit (body: {})", truncate(&body, 200));
        bail!("RDAP is rate limiting requests — try again in a minute.");
    }
    if !status.is_success() {
        tracing::debug!("RDAP HTTP {status} (body: {})", truncate(&body, 200));
        bail!("RDAP lookup failed (HTTP {status}).");
    }

    serde_json::from_str(&body).map_err(|error| {
        tracing::debug!("RDAP {kind} JSON parse failed: {error}");
        anyhow::anyhow!("RDAP returned an unreadable {kind} record.")
    })
}

fn parse_domain(domain: &str, body: &Value) -> RdapDomain {
    RdapDomain {
        domain: body
            .get("ldhName")
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| domain.to_owned()),
        registrar: registrar(body),
        created: event_date(body, "registration"),
        expires: event_date(body, "expiration"),
        updated: event_date(body, "last changed"),
        statuses: string_array(body.get("status")),
        nameservers: body
            .get("nameservers")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("ldhName")
                            .or_else(|| item.get("unicodeName"))
                            .and_then(Value::as_str)
                            .map(|value| value.to_ascii_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn parse_ip(body: &Value) -> RdapIp {
    RdapIp {
        handle: string_field(body, "handle"),
        name: string_field(body, "name"),
        country: string_field(body, "country"),
        start: string_field(body, "startAddress"),
        end: string_field(body, "endAddress"),
        cidr: body
            .get("cidr0_cidrs")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let prefix = entry
                            .get("v4prefix")
                            .or_else(|| entry.get("v6prefix"))
                            .and_then(Value::as_str)?;
                        let length = entry.get("length").and_then(Value::as_u64)?;
                        Some(format!("{prefix}/{length}"))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        organization: organization(body),
    }
}

fn string_field(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn event_date(body: &Value, action: &str) -> Option<String> {
    body.get("events")?.as_array()?.iter().find_map(|event| {
        let event_action = event.get("eventAction").and_then(Value::as_str)?;
        if !event_action.eq_ignore_ascii_case(action) {
            return None;
        }
        event
            .get("eventDate")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

/// Registrar name from the entity entry whose roles include `registrar`.
fn registrar(body: &Value) -> Option<String> {
    body.get("entities")?.as_array()?.iter().find_map(|entity| {
        let is_registrar = entity
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|roles| {
                roles
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|role| role.eq_ignore_ascii_case("registrar"))
            });
        if !is_registrar {
            return None;
        }
        vcard_field(entity, &["fn", "org"])
    })
}

/// First organisation-ish vCard value across all entities.
fn organization(body: &Value) -> Option<String> {
    body.get("entities")?
        .as_array()?
        .iter()
        .find_map(|entity| vcard_field(entity, &["org", "fn"]))
}

/// Read a text field from a jCard vCard array (RFC 6350 field 4).
fn vcard_field(entity: &Value, keys: &[&str]) -> Option<String> {
    let entries = entity.get("vcardArray")?.as_array()?.get(1)?.as_array()?;
    for key in keys {
        for entry in entries {
            let Some(fields) = entry.as_array() else {
                continue;
            };
            let Some(name) = fields.first().and_then(Value::as_str) else {
                continue;
            };
            if !name.eq_ignore_ascii_case(key) {
                continue;
            }
            if let Some(value) = fields.get(3).and_then(Value::as_str)
                && !value.trim().is_empty()
            {
                return Some(value.trim().to_owned());
            }
        }
    }
    None
}

fn recall<T: serde::de::DeserializeOwned>(state: &SharedState, key: &str) -> Option<T> {
    let payload = state.db.cache_get(key).ok().flatten()?;
    serde_json::from_str(&payload).ok()
}

fn remember<T: Serialize>(state: &SharedState, key: &str, value: &T) {
    match serde_json::to_string(value) {
        Ok(payload) => {
            if let Err(error) = state.db.cache_put(key, &payload, RDAP_TTL_SECS) {
                tracing::warn!("RDAP cache write failed: {error}");
            }
        }
        Err(error) => tracing::warn!("RDAP cache serialisation failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_domains() {
        assert_eq!(
            normalize_domain("Example.COM."),
            Some("example.com".to_owned())
        );
        assert_eq!(
            normalize_domain(" sub.example.org "),
            Some("sub.example.org".to_owned())
        );
        assert_eq!(normalize_domain("https://example.com"), None);
        assert_eq!(normalize_domain("bad_domain.example"), None);
        assert_eq!(normalize_domain("localhost"), None);
        assert_eq!(normalize_domain("-bad.example"), None);
        assert_eq!(normalize_domain(""), None);
    }

    #[test]
    fn parses_domain_record() {
        let body = json!({
            "ldhName": "EXAMPLE.COM",
            "status": ["client transfer prohibited", "active"],
            "events": [
                {"eventAction": "registration", "eventDate": "1995-08-14T04:00:00Z"},
                {"eventAction": "expiration", "eventDate": "2027-08-13T04:00:00Z"},
                {"eventAction": "last changed", "eventDate": "2026-08-13T07:01:35Z"}
            ],
            "nameservers": [
                {"ldhName": "A.IANA-SERVERS.NET"},
                {"ldhName": "B.IANA-SERVERS.NET"}
            ],
            "entities": [{
                "roles": ["registrar"],
                "vcardArray": ["vcard", [
                    ["version", {}, "text", "4.0"],
                    ["fn", {}, "text", "RESERVED-Internet Assigned Numbers Authority"],
                    ["org", {}, "text", "IANA"]
                ]]
            }]
        });
        let record = parse_domain("example.com", &body);
        assert_eq!(record.domain, "example.com");
        assert_eq!(
            record.registrar.as_deref(),
            Some("RESERVED-Internet Assigned Numbers Authority")
        );
        assert_eq!(record.created.as_deref(), Some("1995-08-14T04:00:00Z"));
        assert_eq!(record.expires.as_deref(), Some("2027-08-13T04:00:00Z"));
        assert_eq!(record.updated.as_deref(), Some("2026-08-13T07:01:35Z"));
        assert_eq!(record.statuses.len(), 2);
        assert_eq!(record.nameservers[0], "a.iana-servers.net");
    }

    #[test]
    fn domain_record_tolerates_missing_fields() {
        let record = parse_domain("example.com", &json!({}));
        assert_eq!(record.domain, "example.com");
        assert!(record.registrar.is_none());
        assert!(record.created.is_none());
        assert!(record.statuses.is_empty());
        assert!(record.nameservers.is_empty());
    }

    #[test]
    fn parses_ip_record() {
        let body = json!({
            "handle": "NET-1-1-1-0-1",
            "name": "APNIC-LABS",
            "country": "AU",
            "startAddress": "1.1.1.0",
            "endAddress": "1.1.1.255",
            "cidr0_cidrs": [{"v4prefix": "1.1.1.0", "length": 24}],
            "entities": [{
                "roles": ["registrant"],
                "vcardArray": ["vcard", [
                    ["version", {}, "text", "4.0"],
                    ["fn", {}, "text", "APNIC Research"],
                    ["org", {}, "text", "APNIC Pty Ltd"]
                ]]
            }]
        });
        let record = parse_ip(&body);
        assert_eq!(record.handle.as_deref(), Some("NET-1-1-1-0-1"));
        assert_eq!(record.name.as_deref(), Some("APNIC-LABS"));
        assert_eq!(record.country.as_deref(), Some("AU"));
        assert_eq!(record.start.as_deref(), Some("1.1.1.0"));
        assert_eq!(record.end.as_deref(), Some("1.1.1.255"));
        assert_eq!(record.cidr, vec!["1.1.1.0/24".to_owned()]);
        assert_eq!(record.organization.as_deref(), Some("APNIC Pty Ltd"));
    }

    #[test]
    fn ip_record_tolerates_missing_fields() {
        let record = parse_ip(&json!({}));
        assert!(record.handle.is_none());
        assert!(record.cidr.is_empty());
        assert!(record.organization.is_none());
    }
}
