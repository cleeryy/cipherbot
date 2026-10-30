//! Time parsing and formatting. All bot timestamps are unix seconds.

use chrono::{TimeZone as _, Utc};

/// Parse a compact duration into seconds.
///
/// Accepted: `10` (seconds), `10s`, `5m`, `2h`, `7d`, `1w`, and combinations
/// like `1h30m`. Returns `None` for empty, malformed or overflowing input.
pub fn parse_duration(input: &str) -> Option<i64> {
    let raw = input.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }

    if raw.chars().all(|c| c.is_ascii_digit()) {
        return raw.parse::<i64>().ok();
    }

    let mut total: i64 = 0;
    let mut number: i64 = 0;
    let mut has_digits = false;
    let mut has_unit = false;

    for ch in raw.chars() {
        if let Some(digit) = ch.to_digit(10) {
            number = number.checked_mul(10)?.checked_add(i64::from(digit))?;
            has_digits = true;
            continue;
        }

        let multiplier = match ch {
            's' => 1,
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            'w' => 604_800,
            _ => return None,
        };
        if !has_digits {
            return None;
        }
        total = total.checked_add(number.checked_mul(multiplier)?)?;
        number = 0;
        has_digits = false;
        has_unit = true;
    }

    if has_digits || !has_unit {
        return None;
    }
    Some(total)
}

/// Human-readable duration such as `1d 2h 3m 4s`.
pub fn human_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds == 0 {
        return "0s".to_owned();
    }

    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;

    let mut parts = Vec::with_capacity(4);
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if secs > 0 {
        parts.push(format!("{secs}s"));
    }
    parts.join(" ")
}

pub fn unix_now() -> i64 {
    Utc::now().timestamp()
}

/// `2026-10-30 14:05 UTC`
pub fn human_date(unix_seconds: i64) -> String {
    match Utc.timestamp_opt(unix_seconds, 0).single() {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => "unknown".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_seconds() {
        assert_eq!(parse_duration("10"), Some(10));
        assert_eq!(parse_duration(" 0 "), Some(0));
    }

    #[test]
    fn parses_units() {
        assert_eq!(parse_duration("10s"), Some(10));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("2h"), Some(7_200));
        assert_eq!(parse_duration("7d"), Some(604_800));
        assert_eq!(parse_duration("1w"), Some(604_800));
    }

    #[test]
    fn parses_combinations() {
        assert_eq!(parse_duration("1h30m"), Some(5_400));
        assert_eq!(parse_duration("1d2h3m4s"), Some(93_784));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("h"), None);
        assert_eq!(parse_duration("10x"), None);
        assert_eq!(parse_duration("10 "), Some(10));
        assert_eq!(parse_duration("1h30"), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("-5m"), None);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(human_duration(0), "0s");
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(90), "1m 30s");
        assert_eq!(human_duration(3_600), "1h");
        assert_eq!(human_duration(93_784), "1d 2h 3m 4s");
        assert_eq!(human_duration(-5), "0s");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(human_date(0), "1970-01-01 00:00 UTC");
        assert_eq!(human_date(1_790_000_000), "2026-09-21 14:13 UTC");
        assert_eq!(human_date(i64::MAX), "unknown");
    }
}
