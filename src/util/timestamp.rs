//! Timestamp parsing and rendering for the `/tool timestamp` subcommand.
//!
//! Accepted inputs: `now`, 1–10 digit unix seconds, 13 digit unix milliseconds
//! and RFC 3339. Every rendering is UTC so the output is unambiguous.

use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, TimeZone as _, Utc};

#[derive(Debug, Clone)]
pub struct Stamp {
    pub datetime: DateTime<Utc>,
    /// How the input was interpreted: `now`, `unix seconds`, `unix milliseconds`, `RFC 3339`.
    pub source: &'static str,
}

impl Stamp {
    /// `2026-09-21 14:13:20 UTC`
    pub fn utc(&self) -> String {
        self.datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }

    pub fn unix_seconds(&self) -> i64 {
        self.datetime.timestamp()
    }

    pub fn unix_millis(&self) -> i64 {
        self.datetime.timestamp_millis()
    }

    /// RFC 3339 in UTC, with fractional seconds only when they are non-zero.
    pub fn rfc3339(&self) -> String {
        self.datetime.to_rfc3339_opts(SecondsFormat::AutoSi, true)
    }

    /// Full English weekday, e.g. `Monday`.
    pub fn weekday(&self) -> String {
        self.datetime.format("%A").to_string()
    }
}

pub fn parse(input: &str) -> Result<Stamp> {
    let raw = input.trim();
    if raw.is_empty() {
        bail!("Give `now`, unix seconds, unix milliseconds or an RFC 3339 timestamp.");
    }
    if raw.chars().count() > 64 {
        bail!("Timestamp must be 64 characters or fewer.");
    }
    if raw.eq_ignore_ascii_case("now") {
        return Ok(Stamp {
            datetime: Utc::now(),
            source: "now",
        });
    }
    if raw.chars().all(|ch| ch.is_ascii_digit()) {
        return parse_numeric(raw);
    }
    if let Ok(datetime) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Stamp {
            datetime: datetime.with_timezone(&Utc),
            source: "RFC 3339",
        });
    }

    bail!(
        "Unrecognised timestamp `{raw}`. Use `now`, 10-digit unix seconds, \
         13-digit unix milliseconds or RFC 3339 (e.g. 2026-10-30T12:00:00Z)."
    )
}

fn parse_numeric(raw: &str) -> Result<Stamp> {
    match raw.len() {
        1..=10 => {
            let seconds: i64 = raw
                .parse()
                .map_err(|_| anyhow::anyhow!("`{raw}` is not a valid unix timestamp."))?;
            from_seconds(seconds)
        }
        13 => {
            let millis: i64 = raw
                .parse()
                .map_err(|_| anyhow::anyhow!("`{raw}` is not a valid unix timestamp."))?;
            from_millis(millis)
        }
        11 | 12 => bail!(
            "`{raw}` has {} digits — ambiguous. Use 10 digits (seconds), \
             13 digits (milliseconds) or RFC 3339.",
            raw.len()
        ),
        _ => bail!(
            "`{raw}` has {} digits — unix seconds need 10 digits (or fewer), \
             milliseconds 13.",
            raw.len()
        ),
    }
}

fn from_seconds(seconds: i64) -> Result<Stamp> {
    let datetime = Utc
        .timestamp_opt(seconds, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("`{seconds}` is outside the supported date range."))?;
    Ok(Stamp {
        datetime,
        source: "unix seconds",
    })
}

fn from_millis(millis: i64) -> Result<Stamp> {
    let datetime = Utc
        .timestamp_millis_opt(millis)
        .single()
        .ok_or_else(|| anyhow::anyhow!("`{millis}` is outside the supported date range."))?;
    Ok(Stamp {
        datetime,
        source: "unix milliseconds",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_seconds() {
        let stamp = parse("1790000000").expect("seconds");
        assert_eq!(stamp.utc(), "2026-09-21 14:13:20 UTC");
        assert_eq!(stamp.unix_seconds(), 1_790_000_000);
        assert_eq!(stamp.unix_millis(), 1_790_000_000_000);
        assert_eq!(stamp.rfc3339(), "2026-09-21T14:13:20Z");
        assert_eq!(stamp.weekday(), "Monday");
        assert_eq!(stamp.source, "unix seconds");
    }

    #[test]
    fn parses_unix_milliseconds() {
        let stamp = parse("1790000000123").expect("milliseconds");
        assert_eq!(stamp.utc(), "2026-09-21 14:13:20 UTC");
        assert_eq!(stamp.unix_seconds(), 1_790_000_000);
        assert_eq!(stamp.unix_millis(), 1_790_000_000_123);
        assert_eq!(stamp.rfc3339(), "2026-09-21T14:13:20.123Z");
        assert_eq!(stamp.source, "unix milliseconds");
    }

    #[test]
    fn parses_rfc3339_and_offsets() {
        let utc = parse("2026-10-30T12:00:00Z").expect("rfc3339");
        assert_eq!(utc.utc(), "2026-10-30 12:00:00 UTC");
        assert_eq!(utc.weekday(), "Friday");
        assert_eq!(utc.rfc3339(), "2026-10-30T12:00:00Z");
        assert_eq!(utc.source, "RFC 3339");

        let offset = parse("2026-10-30T14:00:00+02:00").expect("offset");
        assert_eq!(offset.utc(), "2026-10-30 12:00:00 UTC");
        assert_eq!(offset.unix_seconds(), utc.unix_seconds());

        let fractional = parse("2026-10-30T12:00:00.250Z").expect("fractional");
        assert_eq!(fractional.unix_millis(), utc.unix_millis() + 250);
    }

    #[test]
    fn parses_epoch_and_now() {
        let epoch = parse("0").expect("epoch");
        assert_eq!(epoch.utc(), "1970-01-01 00:00:00 UTC");
        assert_eq!(epoch.weekday(), "Thursday");

        let now = parse("now").expect("now");
        assert_eq!(now.source, "now");
        assert!(now.unix_seconds() > 1_700_000_000);
        assert!(parse("NOW").is_ok());
    }

    #[test]
    fn rejects_invalid_input() {
        for bad in [
            "",
            "abc",
            "12345678901",
            "123456789012",
            "12345678901234",
            "2026-13-40T99:00:00Z",
            "2026-10-30",
            "1790000000.5",
        ] {
            assert!(parse(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn round_trips_through_unix_seconds() {
        let stamp = parse("2026-10-30T12:00:00Z").expect("rfc3339");
        let again = parse(&stamp.unix_seconds().to_string()).expect("seconds");
        assert_eq!(again.utc(), stamp.utc());
        let from_millis = parse(&stamp.unix_millis().to_string()).expect("millis");
        assert_eq!(from_millis.utc(), stamp.utc());
    }
}
