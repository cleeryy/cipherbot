//! Hash and token format identification for `/tool hashid`.
//!
//! Format detection is a guess, never a proof: every entry carries a confidence
//! level and a short caveat so the reply can stay honest.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashGuess {
    pub name: &'static str,
    pub confidence: Confidence,
    pub note: &'static str,
}

const MD5_NOTE: &str = "128-bit digest; the same 32 hex characters could be a Windows NTLM hash.";

/// Best-effort identification of a hash, token or encoded blob.
pub fn identify(value: &str) -> Vec<HashGuess> {
    let trimmed = value.trim();
    let mut guesses = Vec::new();
    if trimmed.is_empty() {
        return guesses;
    }

    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("$2a$")
        || lower.starts_with("$2b$")
        || lower.starts_with("$2y$")
        || lower.starts_with("$2x$")
        || lower.starts_with("$2$")
    {
        guesses.push(HashGuess {
            name: "bcrypt",
            confidence: Confidence::High,
            note: "Blowfish-based password hash; the number after the prefix is the cost factor.",
        });
    }
    if lower.starts_with("$argon2") {
        guesses.push(HashGuess {
            name: "Argon2",
            confidence: Confidence::High,
            note: "Memory-hard password hash (argon2i, argon2d or argon2id).",
        });
    }
    if lower.starts_with("$6$") {
        guesses.push(HashGuess {
            name: "SHA-512 crypt",
            confidence: Confidence::High,
            note: "glibc `crypt(3)` SHA-512 password hash (`$6$`).",
        });
    }
    if lower.starts_with("$5$") {
        guesses.push(HashGuess {
            name: "SHA-256 crypt",
            confidence: Confidence::High,
            note: "glibc `crypt(3)` SHA-256 password hash (`$5$`).",
        });
    }
    let mysql5 = trimmed
        .strip_prefix('*')
        .is_some_and(|rest| rest.len() == 40 && is_hex(rest));
    if mysql5 {
        guesses.push(HashGuess {
            name: "MySQL5",
            confidence: Confidence::High,
            note: "`*` followed by the uppercase SHA-1 of the password SHA-1.",
        });
    }
    if is_uuid(&lower) {
        guesses.push(HashGuess {
            name: "UUID",
            confidence: Confidence::High,
            note: "Universally unique identifier, not a hash.",
        });
    }
    if is_jwt(trimmed) {
        guesses.push(HashGuess {
            name: "JWT",
            confidence: Confidence::High,
            note: "Three base64url segments (`header.payload.signature`); signature not verified.",
        });
    }

    let hex_only = is_hex(&lower);
    if hex_only {
        match lower.len() {
            8 => guesses.push(HashGuess {
                name: "CRC32",
                confidence: Confidence::Low,
                note: "8 hex characters: too short for a cryptographic digest; could also be a truncated id.",
            }),
            32 => {
                guesses.push(HashGuess {
                    name: "MD5",
                    confidence: Confidence::Medium,
                    note: MD5_NOTE,
                });
                guesses.push(HashGuess {
                    name: "NTLM",
                    confidence: Confidence::Medium,
                    note: "Windows NTLM hash; indistinguishable from MD5 by shape alone.",
                });
            }
            40 => guesses.push(HashGuess {
                name: "SHA-1",
                confidence: Confidence::High,
                note: "160-bit digest; deprecated for signatures, still common for Git and HIBP.",
            }),
            64 => guesses.push(HashGuess {
                name: "SHA-256",
                confidence: Confidence::High,
                note: "256-bit digest (also the shape of SHA3-256, BLAKE2s and others).",
            }),
            128 => guesses.push(HashGuess {
                name: "SHA-512",
                confidence: Confidence::High,
                note: "512-bit digest (also the shape of SHA3-512, BLAKE2b and others).",
            }),
            _ => {}
        }
    }

    if guesses.is_empty() && looks_like_base64(trimmed) {
        guesses.push(HashGuess {
            name: "Base64",
            confidence: Confidence::Low,
            note: "Printable base64 blob; could be encoded text, a token or a binary digest.",
        });
    }

    guesses
}

fn is_hex(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && is_hex(group))
}

fn is_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts[0].len() >= 4
        && parts[1].len() >= 4
        && parts.iter().all(|part| {
            part.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '='))
        })
}

fn looks_like_base64(value: &str) -> bool {
    let trimmed = value.trim_end_matches('=');
    if trimmed.len() < 8 {
        return false;
    }
    trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(value: &str) -> Vec<&'static str> {
        identify(value)
            .into_iter()
            .map(|guess| guess.name)
            .collect()
    }

    #[test]
    fn md5_and_ntlm_are_ambiguous() {
        let guesses = identify("d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(guesses.len(), 2);
        assert!(guesses.iter().any(|guess| guess.name == "MD5"));
        assert!(guesses.iter().any(|guess| guess.name == "NTLM"));
        assert!(
            guesses
                .iter()
                .all(|guess| guess.confidence == Confidence::Medium)
        );
    }

    #[test]
    fn sha_family_lengths() {
        assert_eq!(
            names("da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            vec!["SHA-1"]
        );
        assert_eq!(
            names("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            vec!["SHA-256"]
        );
        assert_eq!(names(&"a".repeat(128)), vec!["SHA-512"],);
        assert_eq!(names("1a2b3c4d"), vec!["CRC32"]);
        assert_eq!(identify("1a2b3c4d")[0].confidence, Confidence::Low);
    }

    #[test]
    fn password_hash_prefixes() {
        assert_eq!(names("$2b$12$KIXQ4Yl7z1Y8oPq0Ff3nUe"), vec!["bcrypt"]);
        assert_eq!(names("$2y$10$abcdefghijklmnopqrstuv"), vec!["bcrypt"]);
        assert_eq!(
            names("$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$hash"),
            vec!["Argon2"]
        );
        assert_eq!(names("$6$rounds=5000$salt$hash"), vec!["SHA-512 crypt"]);
        assert_eq!(names("$5$salt$hash"), vec!["SHA-256 crypt"]);
    }

    #[test]
    fn mysql5_uuid_and_jwt() {
        assert_eq!(
            names("*2470C0C06DEE42FD1618BB99005ADCA2EC9D1E19"),
            vec!["MySQL5"]
        );
        assert_eq!(
            names("*2470c0c06dee42fd1618bb99005adca2ec9d1e19"),
            vec!["MySQL5"]
        );
        assert_eq!(names("123e4567-e89b-12d3-a456-426614174000"), vec!["UUID"]);
        assert_eq!(
            names("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJl"),
            vec!["JWT"]
        );
    }

    #[test]
    fn base64_fallback_is_low_confidence() {
        let guesses = identify("aGVsbG8gd29ybGQ=");
        assert_eq!(guesses.len(), 1);
        assert_eq!(guesses[0].name, "Base64");
        assert_eq!(guesses[0].confidence, Confidence::Low);
        assert!(identify("hello world!").is_empty());
    }

    #[test]
    fn trims_and_ignores_noise() {
        assert!(identify("   ").is_empty());
        assert!(
            identify("  d41d8cd98f00b204e9800998ecf8427e  ")
                .iter()
                .any(|guess| guess.name == "MD5")
        );
        assert!(identify("").is_empty());
        assert!(identify("not a hash at all").is_empty());
    }
}
