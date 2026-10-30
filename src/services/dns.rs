//! DNS-over-HTTPS client (Cloudflare JSON API).
//!
//! A single fixed host is queried — the user only picks the name and the record
//! type — and the name is validated before it is interpolated into the query
//! string, so no user-controlled host or path ever reaches the HTTP client.
//! Responses are cached for one hour, which matches Cloudflare's own minimum
//! TTL behaviour closely enough for a chat lookup.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::bot::SharedState;
use crate::services::rdap;
use crate::util::text::truncate;

const DOH_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";
const DNS_TTL_SECS: i64 = 60 * 60;

/// Record types offered by `/intel dns type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordType {
    A,
    Aaaa,
    Mx,
    Txt,
    Ns,
    Cname,
    Soa,
}

impl RecordType {
    /// Case-insensitive parse of a Discord string choice.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "A" => Some(Self::A),
            "AAAA" => Some(Self::Aaaa),
            "MX" => Some(Self::Mx),
            "TXT" => Some(Self::Txt),
            "NS" => Some(Self::Ns),
            "CNAME" => Some(Self::Cname),
            "SOA" => Some(Self::Soa),
            _ => None,
        }
    }

    /// Wire name used in the DoH query and in embed titles.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Mx => "MX",
            Self::Txt => "TXT",
            Self::Ns => "NS",
            Self::Cname => "CNAME",
            Self::Soa => "SOA",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DnsAnswer {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub rtype: u16,
    #[serde(rename = "TTL", default)]
    pub ttl: u32,
    #[serde(default)]
    pub data: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DnsLookup {
    /// DNS RCODE: 0 = NOERROR, 2 = SERVFAIL, 3 = NXDOMAIN.
    pub status: i32,
    pub answers: Vec<DnsAnswer>,
    pub authority: Vec<DnsAnswer>,
}

#[derive(Debug, Default, Deserialize)]
struct RawResponse {
    #[serde(rename = "Status", default)]
    status: i32,
    #[serde(rename = "Answer", default)]
    answers: Vec<DnsAnswer>,
    #[serde(rename = "Authority", default)]
    authority: Vec<DnsAnswer>,
}

/// Validate and canonicalise a domain name.
///
/// Hostname syntax is shared with RDAP: lowercase ASCII labels, no scheme, no
/// path, no wildcard, at most 253 characters. Returns `None` for IP addresses
/// and anything else that is not a plain hostname.
pub fn normalize_name(raw: &str) -> Option<String> {
    rdap::normalize_domain(raw)
}

/// Cache key for a lookup: `doh:<name>:<type>`.
fn cache_key(name: &str, rtype: RecordType) -> String {
    format!("doh:{name}:{}", rtype.as_str())
}

/// Human label for a DNS record type number.
pub fn type_name(code: u16) -> String {
    match code {
        1 => "A".to_owned(),
        2 => "NS".to_owned(),
        5 => "CNAME".to_owned(),
        6 => "SOA".to_owned(),
        12 => "PTR".to_owned(),
        15 => "MX".to_owned(),
        16 => "TXT".to_owned(),
        28 => "AAAA".to_owned(),
        33 => "SRV".to_owned(),
        35 => "NAPTR".to_owned(),
        43 => "DS".to_owned(),
        46 => "RRSIG".to_owned(),
        48 => "DNSKEY".to_owned(),
        52 => "TLSA".to_owned(),
        64 => "SVCB".to_owned(),
        65 => "HTTPS".to_owned(),
        257 => "CAA".to_owned(),
        other => format!("TYPE{other}"),
    }
}

/// Human label for a DNS RCODE.
pub fn status_label(status: i32) -> String {
    match status {
        0 => "NOERROR".to_owned(),
        1 => "FORMERR".to_owned(),
        2 => "SERVFAIL".to_owned(),
        3 => "NXDOMAIN".to_owned(),
        4 => "NOTIMP".to_owned(),
        5 => "REFUSED".to_owned(),
        9 => "NOTAUTH".to_owned(),
        10 => "NOTZONE".to_owned(),
        other => format!("RCODE{other}"),
    }
}

/// Resolve `name` for one record type through Cloudflare DoH.
pub async fn lookup(state: &SharedState, raw_name: &str, rtype: RecordType) -> Result<DnsLookup> {
    let name = normalize_name(raw_name).ok_or_else(|| {
        anyhow::anyhow!(
            "`{}` is not a valid domain name.",
            truncate(raw_name.trim(), 80)
        )
    })?;

    let key = cache_key(&name, rtype);
    if let Some(cached) = recall(state, &key) {
        return Ok(cached);
    }

    let url = format!("{DOH_ENDPOINT}?name={name}&type={}", rtype.as_str());
    let response = state
        .http
        .get(url)
        .header(reqwest::header::ACCEPT, "application/dns-json")
        .send()
        .await
        .map_err(|error| {
            tracing::debug!("DNS DoH request failed: {}", error.without_url());
            anyhow::anyhow!("DNS resolver unavailable — try again shortly.")
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        tracing::debug!("DNS DoH rate limit (body: {})", truncate(&body, 200));
        bail!("DNS resolver is rate limiting requests — try again in a minute.");
    }
    if !status.is_success() {
        tracing::debug!("DNS DoH HTTP {status} (body: {})", truncate(&body, 200));
        bail!("DNS resolver unavailable (HTTP {status}).");
    }

    let lookup = parse_lookup(&body)?;
    remember(state, &key, &lookup);
    Ok(lookup)
}

/// Parse a Cloudflare DoH JSON body. Missing sections default to empty.
fn parse_lookup(body: &str) -> Result<DnsLookup> {
    let raw: RawResponse = serde_json::from_str(body).map_err(|error| {
        tracing::debug!("DNS DoH JSON parse failed: {error}");
        anyhow::anyhow!("DNS returned an unreadable response.")
    })?;

    Ok(DnsLookup {
        status: raw.status,
        answers: raw.answers,
        authority: raw.authority,
    })
}

fn recall(state: &SharedState, key: &str) -> Option<DnsLookup> {
    let payload = state.db.cache_get(key).ok().flatten()?;
    serde_json::from_str(&payload).ok()
}

fn remember(state: &SharedState, key: &str, value: &DnsLookup) {
    match serde_json::to_string(value) {
        Ok(payload) => {
            if let Err(error) = state.db.cache_put(key, &payload, DNS_TTL_SECS) {
                tracing::warn!("DNS cache write failed: {error}");
            }
        }
        Err(error) => tracing::warn!("DNS cache serialisation failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cloudflare_fixture() {
        let body = r#"{
            "Status": 0,
            "TC": false,
            "RD": true,
            "RA": true,
            "AD": false,
            "CD": false,
            "Question": [{"name": "example.com.", "type": 1}],
            "Answer": [
                {"name": "example.com.", "type": 1, "TTL": 300, "data": "93.184.216.34"},
                {"name": "example.com.", "type": 1, "TTL": 300, "data": "93.184.216.35"}
            ],
            "Authority": []
        }"#;

        let lookup = parse_lookup(body).expect("valid fixture");
        assert_eq!(lookup.status, 0);
        assert_eq!(lookup.answers.len(), 2);
        assert_eq!(lookup.answers[0].name, "example.com.");
        assert_eq!(lookup.answers[0].rtype, 1);
        assert_eq!(lookup.answers[0].ttl, 300);
        assert_eq!(lookup.answers[0].data, "93.184.216.34");
        assert!(lookup.authority.is_empty());
    }

    #[test]
    fn parses_mixed_answer_types() {
        let body = r#"{
            "Status": 0,
            "Answer": [
                {"name": "example.com.", "type": 15, "TTL": 3600, "data": "10 mail.example.com."},
                {"name": "example.com.", "type": 16, "TTL": 60, "data": "\"v=spf1 -all\""}
            ]
        }"#;
        let lookup = parse_lookup(body).expect("valid fixture");
        assert_eq!(type_name(lookup.answers[0].rtype), "MX");
        assert_eq!(type_name(lookup.answers[1].rtype), "TXT");
        assert_eq!(lookup.answers[0].data, "10 mail.example.com.");
        assert_eq!(lookup.answers[1].data, "\"v=spf1 -all\"");
    }

    #[test]
    fn parses_nxdomain_with_soa_authority() {
        let body = r#"{
            "Status": 3,
            "Authority": [
                {"name": "com.", "type": 6, "TTL": 900, "data": "a.gtld-servers.net. nstld.verisign-grs.com. 1 1800 900 604800 86400"}
            ]
        }"#;
        let lookup = parse_lookup(body).expect("valid fixture");
        assert_eq!(status_label(lookup.status), "NXDOMAIN");
        assert!(lookup.answers.is_empty());
        assert_eq!(lookup.authority.len(), 1);
        assert_eq!(type_name(lookup.authority[0].rtype), "SOA");
    }

    #[test]
    fn empty_response_has_no_answers() {
        let lookup = parse_lookup(r#"{"Status": 0}"#).expect("valid fixture");
        assert_eq!(lookup.status, 0);
        assert!(lookup.answers.is_empty());
        assert!(lookup.authority.is_empty());
    }

    #[test]
    fn rejects_unreadable_responses() {
        assert!(parse_lookup("not json").is_err());
        assert!(parse_lookup("").is_err());
        assert!(parse_lookup(r#"{"Status": "oops"}"#).is_err());
    }

    #[test]
    fn maps_record_type_numbers() {
        assert_eq!(type_name(1), "A");
        assert_eq!(type_name(28), "AAAA");
        assert_eq!(type_name(15), "MX");
        assert_eq!(type_name(16), "TXT");
        assert_eq!(type_name(2), "NS");
        assert_eq!(type_name(5), "CNAME");
        assert_eq!(type_name(6), "SOA");
        assert_eq!(type_name(65), "HTTPS");
        assert_eq!(type_name(9999), "TYPE9999");
    }

    #[test]
    fn parses_and_prints_record_types() {
        for (raw, expected) in [
            ("a", RecordType::A),
            ("AAAA", RecordType::Aaaa),
            ("Mx", RecordType::Mx),
            ("txt", RecordType::Txt),
            ("NS", RecordType::Ns),
            ("cname", RecordType::Cname),
            ("SOA", RecordType::Soa),
        ] {
            assert_eq!(RecordType::parse(raw), Some(expected));
            assert_eq!(RecordType::parse(expected.as_str()), Some(expected));
        }
        assert_eq!(RecordType::parse("PTR"), None);
        assert_eq!(RecordType::parse(""), None);
    }

    #[test]
    fn validates_domains_and_rejects_ips() {
        assert_eq!(
            normalize_name("Example.COM."),
            Some("example.com".to_owned())
        );
        assert_eq!(
            normalize_name(" sub.example.org "),
            Some("sub.example.org".to_owned())
        );
        assert_eq!(normalize_name("https://example.com"), None);
        assert_eq!(normalize_name("example.com/path"), None);
        assert_eq!(normalize_name("1.1.1.1"), None);
        assert_eq!(normalize_name("2001:db8::1"), None);
        assert_eq!(normalize_name("bad_domain.example"), None);
        assert_eq!(normalize_name("*._tcp.example.com"), None);
        assert_eq!(normalize_name(""), None);
    }

    #[test]
    fn builds_stable_cache_keys() {
        assert_eq!(cache_key("example.com", RecordType::A), "doh:example.com:A");
        assert_eq!(
            cache_key("sub.example.com", RecordType::Aaaa),
            "doh:sub.example.com:AAAA"
        );
        assert_eq!(
            cache_key("example.com", RecordType::Soa),
            "doh:example.com:SOA"
        );
    }
}
