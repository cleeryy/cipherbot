//! Password / string entropy estimation for `/tool entropy` and `/tool password`.
//!
//! The estimates are deliberately conservative: the charset is inferred from
//! character classes (an attacker is assumed to know the alphabet) rather than
//! from the observed characters only.

use std::collections::{HashMap, HashSet};

/// Result of [`analyze`].
#[derive(Debug, Clone, PartialEq)]
pub struct EntropyReport {
    /// Number of characters (Unicode scalar values).
    pub length: usize,
    /// Number of distinct characters.
    pub unique_chars: usize,
    /// Shannon entropy of the observed character distribution.
    pub shannon_bits_per_char: f64,
    /// `shannon_bits_per_char * length`.
    pub total_bits: f64,
    /// Size of the alphabet implied by the character classes in use.
    pub charset_size: usize,
    /// `log10(charset_size ^ length)`, i.e. guesses assuming brute force.
    pub estimated_guesses_log10: f64,
    /// Seconds to exhaust the keyspace at 10^10 guesses per second.
    pub crack_time_seconds: f64,
    /// Human label derived from `total_bits`.
    pub label: &'static str,
}

/// Guesses per second assumed for the crack-time estimate.
pub const GUESSES_PER_SECOND: f64 = 1e10;

pub fn analyze(value: &str) -> EntropyReport {
    let length = value.chars().count();
    let unique_chars = value.chars().collect::<HashSet<_>>().len();

    let shannon_bits_per_char = if length == 0 {
        0.0
    } else {
        let mut counts: HashMap<char, usize> = HashMap::new();
        for ch in value.chars() {
            *counts.entry(ch).or_insert(0) += 1;
        }
        counts
            .values()
            .map(|count| {
                let probability = *count as f64 / length as f64;
                -probability * probability.log2()
            })
            .sum::<f64>()
    };
    let total_bits = shannon_bits_per_char * length as f64;

    let charset_size = charset_size(value);
    let estimated_guesses_log10 = if length == 0 || charset_size <= 1 {
        0.0
    } else {
        length as f64 * (charset_size as f64).log10()
    };
    let crack_exponent = estimated_guesses_log10 - GUESSES_PER_SECOND.log10();
    let crack_time_seconds = if crack_exponent > 300.0 {
        f64::MAX
    } else {
        10f64.powf(crack_exponent)
    };

    EntropyReport {
        length,
        unique_chars,
        shannon_bits_per_char,
        total_bits,
        charset_size,
        estimated_guesses_log10,
        crack_time_seconds,
        label: label(total_bits),
    }
}

fn label(total_bits: f64) -> &'static str {
    if total_bits < 28.0 {
        "very weak"
    } else if total_bits < 36.0 {
        "weak"
    } else if total_bits < 60.0 {
        "reasonable"
    } else if total_bits < 80.0 {
        "strong"
    } else {
        "very strong"
    }
}

fn charset_size(value: &str) -> usize {
    let mut size = 0;
    if value.chars().any(|ch| ch.is_ascii_lowercase()) {
        size += 26;
    }
    if value.chars().any(|ch| ch.is_ascii_uppercase()) {
        size += 26;
    }
    if value.chars().any(|ch| ch.is_ascii_digit()) {
        size += 10;
    }
    if value.chars().any(|ch| ch.is_ascii_punctuation()) {
        size += 33;
    }
    if value.chars().any(|ch| ch == ' ') {
        size += 1;
    }
    // Non-ASCII input has no class above; fall back to the observed alphabet.
    if size == 0 && !value.is_empty() {
        return value.chars().collect::<HashSet<_>>().len().max(1);
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(left: f64, right: f64) -> bool {
        (left - right).abs() < 0.001
    }

    #[test]
    fn empty_string_is_neutral() {
        let report = analyze("");
        assert_eq!(report.length, 0);
        assert_eq!(report.unique_chars, 0);
        assert_eq!(report.charset_size, 0);
        assert_eq!(report.total_bits, 0.0);
        assert_eq!(report.label, "very weak");
        assert!(report.crack_time_seconds < 1.0);
    }

    #[test]
    fn single_lowercase_character() {
        let report = analyze("a");
        assert_eq!(report.length, 1);
        assert_eq!(report.unique_chars, 1);
        assert_eq!(report.charset_size, 26);
        assert_eq!(report.shannon_bits_per_char, 0.0);
        assert_eq!(report.total_bits, 0.0);
        assert_eq!(report.label, "very weak");
    }

    #[test]
    fn repeated_character_has_zero_entropy() {
        let report = analyze("aaaa");
        assert_eq!(report.length, 4);
        assert_eq!(report.unique_chars, 1);
        assert_eq!(report.shannon_bits_per_char, 0.0);
        assert_eq!(report.total_bits, 0.0);
    }

    #[test]
    fn four_distinct_lowercase_characters() {
        let report = analyze("abcd");
        assert!(approx(report.shannon_bits_per_char, 2.0));
        assert!(approx(report.total_bits, 8.0));
        assert_eq!(report.charset_size, 26);
        assert!(approx(report.estimated_guesses_log10, 4.0 * 26f64.log10()));
    }

    #[test]
    fn password_is_very_weak() {
        let report = analyze("password");
        assert_eq!(report.length, 8);
        assert_eq!(report.unique_chars, 7);
        assert_eq!(report.charset_size, 26);
        assert!(approx(report.total_bits, 22.0));
        assert_eq!(report.label, "very weak");
    }

    #[test]
    fn sixteen_distinct_mixed_characters_are_strong() {
        let report = analyze("Xk9#mQ2$vL7@pR4!");
        assert_eq!(report.length, 16);
        assert_eq!(report.charset_size, 95);
        assert!(approx(report.shannon_bits_per_char, 4.0));
        assert!(approx(report.total_bits, 64.0));
        assert_eq!(report.label, "strong");
    }

    #[test]
    fn twenty_random_characters_are_very_strong() {
        let report = analyze("Xk9#mQ2$vL7@pR4!nZ8&");
        assert_eq!(report.length, 20);
        assert_eq!(report.unique_chars, 20);
        assert!(report.total_bits > 80.0);
        assert_eq!(report.label, "very strong");
        assert!(report.crack_time_seconds >= 1.0);
    }

    #[test]
    fn unicode_falls_back_to_observed_alphabet() {
        let report = analyze("éé");
        assert_eq!(report.length, 2);
        assert_eq!(report.unique_chars, 1);
        assert_eq!(report.charset_size, 1);
        assert_eq!(report.total_bits, 0.0);
        assert_eq!(report.label, "very weak");
    }

    #[test]
    fn symbols_and_spaces_widen_the_charset() {
        assert_eq!(charset_size("abc DEF 123"), 26 + 26 + 10 + 1);
        assert_eq!(charset_size("!@#"), 33);
        assert_eq!(charset_size(""), 0);
    }

    #[test]
    fn huge_keyspace_caps_at_f64_max() {
        let report = analyze(&"a1!B".repeat(100));
        assert_eq!(report.crack_time_seconds, f64::MAX);
    }
}
