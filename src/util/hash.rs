//! Hashing helpers used to store flag submissions without keeping plaintext.

use sha1::Sha1;
use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 digest.
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    hex::encode(digest)
}

/// Lowercase hex SHA-1 digest (toolbox `hashid` / lookup helpers).
pub fn sha1_hex(input: &str) -> String {
    let digest = Sha1::digest(input.as_bytes());
    hex::encode(digest)
}

/// Compare two hex digests without an early exit on content mismatch.
///
/// The length check is unavoidable and leaks only the digest length, which is
/// public anyway.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let left = a.as_bytes();
    let right = b.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in left.iter().zip(right) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq("deadbeef", "deadbeef"));
        assert!(!constant_time_eq("deadbeef", "deadbeee"));
        assert!(!constant_time_eq("deadbeef", "deadbee"));
        assert!(!constant_time_eq("", "x"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn digest_is_lowercase_hex() {
        let digest = sha256_hex("CipherBot");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(digest, digest.to_lowercase());
    }
}
