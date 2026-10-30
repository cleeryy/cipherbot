//! Text encodings used by `/tool encode` and `/tool decode`.
//!
//! Everything here is pure: no I/O, no global state, no logging. Decoders are
//! forgiving about incidental whitespace and return readable errors suitable
//! for an ephemeral bot reply.

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use serde_json::Value;

/// `(pretty-printed header, pretty-printed payload, flattened top-level claims)`.
pub type JwtParts = (String, String, Vec<(String, String)>);

/// Schemes exposed by the `/tool` encode/decode choices.
pub const SCHEMES: &[&str] = &[
    "base64",
    "base64url",
    "hex",
    "url",
    "rot13",
    "binary",
    "morse",
];

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Morse table: `A-Z`, `0-9` and the common punctuation used in practice.
const MORSE: &[(char, &str)] = &[
    ('A', ".-"),
    ('B', "-..."),
    ('C', "-.-."),
    ('D', "-.."),
    ('E', "."),
    ('F', "..-."),
    ('G', "--."),
    ('H', "...."),
    ('I', ".."),
    ('J', ".---"),
    ('K', "-.-"),
    ('L', ".-.."),
    ('M', "--"),
    ('N', "-."),
    ('O', "---"),
    ('P', ".--."),
    ('Q', "--.-"),
    ('R', ".-."),
    ('S', "..."),
    ('T', "-"),
    ('U', "..-"),
    ('V', "...-"),
    ('W', ".--"),
    ('X', "-..-"),
    ('Y', "-.--"),
    ('Z', "--.."),
    ('0', "-----"),
    ('1', ".----"),
    ('2', "..---"),
    ('3', "...--"),
    ('4', "....-"),
    ('5', "....."),
    ('6', "-...."),
    ('7', "--..."),
    ('8', "---.."),
    ('9', "----."),
    ('.', ".-.-.-"),
    (',', "--..--"),
    ('?', "..--.."),
    ('!', "-.-.--"),
    ('/', "-..-."),
    ('(', "-.--."),
    (')', "-.--.-"),
    ('&', ".-..."),
    (':', "---..."),
    (';', "-.-.-."),
    ('=', "-...-"),
    ('+', ".-.-."),
    ('-', "-....-"),
    ('_', "..--.-"),
    ('"', ".-..-."),
    ('$', "...-..-"),
    ('@', ".--.-."),
];

fn morse_code(letter: char) -> Option<&'static str> {
    MORSE
        .iter()
        .find(|(candidate, _)| *candidate == letter)
        .map(|(_, code)| *code)
}

fn morse_letter(code: &str) -> Option<char> {
    MORSE
        .iter()
        .find(|(_, candidate)| *candidate == code)
        .map(|(letter, _)| *letter)
}

/// Encode with the scheme named on the command.
pub fn encode(scheme: &str, input: &str) -> Result<String> {
    match scheme.trim().to_ascii_lowercase().as_str() {
        "base64" => Ok(base64_encode(input)),
        "base64url" => Ok(base64url_encode(input)),
        "hex" => Ok(hex_encode(input)),
        "url" => Ok(url_encode(input)),
        "rot13" => Ok(rot13(input)),
        "binary" => Ok(binary_encode(input)),
        "morse" => morse_encode(input),
        other => bail!(
            "Unknown encoding `{other}`. Choose one of: {}.",
            SCHEMES.join(", ")
        ),
    }
}

/// Decode with the scheme named on the command.
pub fn decode(scheme: &str, input: &str) -> Result<String> {
    match scheme.trim().to_ascii_lowercase().as_str() {
        "base64" => base64_decode(input),
        "base64url" => base64url_decode(input),
        "hex" => hex_decode(input),
        "url" => url_decode(input),
        "rot13" => Ok(rot13(input)),
        "binary" => binary_decode(input),
        "morse" => morse_decode(input),
        other => bail!(
            "Unknown encoding `{other}`. Choose one of: {}.",
            SCHEMES.join(", ")
        ),
    }
}

// ---------------------------------------------------------------- base64 ----

pub fn base64_encode(input: &str) -> String {
    STANDARD.encode(input.as_bytes())
}

/// Standard base64, ignoring whitespace and accepting URL-safe characters.
pub fn base64_decode(input: &str) -> Result<String> {
    let cleaned: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Ok(String::new());
    }
    let unpadded = cleaned.trim_end_matches('=');
    let bytes = STANDARD
        .decode(cleaned.as_bytes())
        .or_else(|_| STANDARD_NO_PAD.decode(unpadded.as_bytes()))
        .or_else(|_| URL_SAFE_NO_PAD.decode(unpadded.as_bytes()))
        .context("Invalid base64 input.")?;
    String::from_utf8(bytes).context("Decoded base64 is not valid UTF-8.")
}

pub fn base64url_encode(input: &str) -> String {
    URL_SAFE_NO_PAD.encode(input.as_bytes())
}

/// URL-safe base64 without padding; padded input is accepted.
pub fn base64url_decode(input: &str) -> Result<String> {
    let cleaned: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Ok(String::new());
    }
    let unpadded = cleaned.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD
        .decode(unpadded.as_bytes())
        .or_else(|_| STANDARD_NO_PAD.decode(unpadded.as_bytes()))
        .context("Invalid base64url input.")?;
    String::from_utf8(bytes).context("Decoded base64url is not valid UTF-8.")
}

// ------------------------------------------------------------------- hex ----

pub fn hex_encode(input: &str) -> String {
    hex::encode(input.as_bytes())
}

/// Hex with optional `0x` prefixes, spaces, colons, dashes and underscores.
pub fn hex_decode(input: &str) -> Result<String> {
    let mut cleaned = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_whitespace() || matches!(ch, ':' | '-' | '_') {
            continue;
        }
        cleaned.push(ch);
    }
    let cleaned = cleaned.replace("0x", "").replace("0X", "");
    if cleaned.is_empty() {
        return Ok(String::new());
    }
    if !cleaned.len().is_multiple_of(2) {
        bail!(
            "Hex input must have an even number of digits (got {}).",
            cleaned.len()
        );
    }
    if !cleaned.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("Hex input contains characters outside 0-9 and a-f.");
    }
    let bytes = hex::decode(&cleaned).context("Invalid hex input.")?;
    String::from_utf8(bytes).context("Decoded hex is not valid UTF-8.")
}

// ------------------------------------------------------------------- url ----

/// Percent-encode every byte outside the RFC 3986 unreserved set.
pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
            out.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
        }
    }
    out
}

/// Percent-decode, treating `+` as a space (form encoding).
pub fn url_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|byte| char::from(*byte).to_digit(16));
                let low = bytes
                    .get(index + 2)
                    .and_then(|byte| char::from(*byte).to_digit(16));
                match (high, low) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        index += 3;
                    }
                    _ => bail!("Invalid percent escape at byte {index}: expected `%XX`."),
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).context("Percent-decoded bytes are not valid UTF-8.")
}

// ----------------------------------------------------------------- rot13 ----

pub fn rot13(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'a'..='z' => (((ch as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((ch as u8 - b'A' + 13) % 26) + b'A') as char,
            other => other,
        })
        .collect()
}

// ---------------------------------------------------------------- binary ----

pub fn binary_encode(input: &str) -> String {
    input
        .bytes()
        .map(|byte| format!("{byte:08b}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decode 8-bit groups, ignoring whitespace. Rejects partial groups.
pub fn binary_decode(input: &str) -> Result<String> {
    let bits: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
    if bits.is_empty() {
        return Ok(String::new());
    }
    if !bits.chars().all(|ch| matches!(ch, '0' | '1')) {
        bail!("Binary input may only contain 0s and 1s (spaces are ignored).");
    }
    if !bits.len().is_multiple_of(8) {
        bail!(
            "Binary input must be a multiple of 8 bits (got {}).",
            bits.len()
        );
    }
    let bytes = bits
        .as_bytes()
        .chunks(8)
        .map(|chunk| chunk.iter().fold(0u8, |acc, bit| (acc << 1) | (bit - b'0')))
        .collect::<Vec<_>>();
    String::from_utf8(bytes).context("Decoded binary is not valid UTF-8.")
}

// ----------------------------------------------------------------- morse ----

/// Encode to Morse, using ` / ` as the word separator.
pub fn morse_encode(input: &str) -> Result<String> {
    let mut words = Vec::new();
    for word in input.split_whitespace() {
        let mut letters = Vec::new();
        for letter in word.to_ascii_uppercase().chars() {
            let code =
                morse_code(letter).ok_or_else(|| anyhow!("No Morse code for `{letter}`."))?;
            letters.push(code);
        }
        if !letters.is_empty() {
            words.push(letters.join(" "));
        }
    }
    Ok(words.join(" / "))
}

/// Decode Morse, tolerating repeated spaces around letters and word breaks.
pub fn morse_decode(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let mut words = Vec::new();
    for word in trimmed.split('/') {
        let mut letters = String::new();
        for token in word.split_whitespace() {
            let letter =
                morse_letter(token).ok_or_else(|| anyhow!("Unknown Morse code `{token}`."))?;
            letters.push(letter);
        }
        if !letters.is_empty() {
            words.push(letters);
        }
    }
    Ok(words.join(" "))
}

// ------------------------------------------------------------------- jwt ----

/// Decode a JWT without verifying its signature.
pub fn decode_jwt(token: &str) -> Result<JwtParts> {
    let token = token.trim();
    if token.is_empty() {
        bail!("Give a JWT to decode.");
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        bail!(
            "Not a JWT: expected 3 dot-separated segments, got {}.",
            parts.len()
        );
    }
    let header = decode_jwt_segment(parts[0], "header")?;
    let payload = decode_jwt_segment(parts[1], "payload")?;

    let claims = payload
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| {
                    let rendered = match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    (key.clone(), rendered)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok((
        serde_json::to_string_pretty(&header)?,
        serde_json::to_string_pretty(&payload)?,
        claims,
    ))
}

fn decode_jwt_segment(segment: &str, name: &str) -> Result<Value> {
    if segment.is_empty() {
        bail!("JWT {name} segment is empty.");
    }
    let unpadded = segment.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD
        .decode(unpadded.as_bytes())
        .with_context(|| format!("JWT {name} is not valid base64url."))?;
    serde_json::from_slice(&bytes).with_context(|| format!("JWT {name} is not valid JSON."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

    #[test]
    fn base64_known_vector_and_whitespace() {
        assert_eq!(base64_encode("hello"), "aGVsbG8=");
        assert_eq!(base64_decode("aGVsbG8=").expect("decode"), "hello");
        assert_eq!(base64_decode("aGVs\nbG8=").expect("whitespace"), "hello");
        assert_eq!(base64_decode("aGVsbG8").expect("unpadded"), "hello");
        assert_eq!(base64_decode("").expect("empty"), "");
        assert!(base64_decode("!!!!").is_err());
        assert!(base64_decode("aGVsbG8=").expect("ok").chars().count() == 5);
    }

    #[test]
    fn base64_roundtrip_unicode() {
        let sample = "héllo 🦀 monde";
        assert_eq!(
            base64_decode(&base64_encode(sample)).expect("roundtrip"),
            sample
        );
    }

    #[test]
    fn base64url_roundtrip_and_padding() {
        assert_eq!(base64url_encode("hello?"), "aGVsbG8_");
        assert_eq!(base64url_decode("aGVsbG8_").expect("decode"), "hello?");
        assert_eq!(base64url_decode("aGVsbG8=").expect("padded"), "hello");
        assert_eq!(base64url_encode("hello"), "aGVsbG8");
        assert!(base64url_decode("!!!!").is_err());
        assert_eq!(base64url_decode("").expect("empty"), "");
    }

    #[test]
    fn hex_known_vector_and_separators() {
        assert_eq!(hex_encode("hi"), "6869");
        assert_eq!(hex_decode("6869").expect("decode"), "hi");
        assert_eq!(hex_decode("0x68:69").expect("decorated"), "hi");
        assert_eq!(hex_decode(" 68 69 ").expect("spaced"), "hi");
        assert_eq!(hex_decode("68-69").expect("dashed"), "hi");
        assert_eq!(hex_decode("0X6869").expect("upper prefix"), "hi");
        assert!(hex_decode("abc").is_err());
        assert!(hex_decode("zz").is_err());
        assert_eq!(hex_decode("").expect("empty"), "");
    }

    #[test]
    fn url_known_vector_and_form_decode() {
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
        assert_eq!(url_encode("safe-._~"), "safe-._~");
        assert_eq!(url_decode("a+b%26c").expect("decode"), "a b&c");
        assert_eq!(url_decode("%C3%A9").expect("utf8"), "é");
        assert_eq!(url_encode("é"), "%C3%A9");
        assert!(url_decode("%zz").is_err());
        assert!(url_decode("%4").is_err());
        assert_eq!(url_decode("").expect("empty"), "");
    }

    #[test]
    fn rot13_roundtrip() {
        assert_eq!(rot13("Hello, World!"), "Uryyb, Jbeyq!");
        assert_eq!(rot13("Uryyb, Jbeyq!"), "Hello, World!");
        assert_eq!(rot13("123 é"), "123 é");
        assert_eq!(encode("rot13", "abc").expect("encode"), "nop");
        assert_eq!(decode("rot13", "nop").expect("decode"), "abc");
    }

    #[test]
    fn binary_roundtrip_and_errors() {
        assert_eq!(binary_encode("A"), "01000001");
        assert_eq!(binary_encode("AB"), "01000001 01000010");
        assert_eq!(binary_decode("01000001 01000010").expect("decode"), "AB");
        assert_eq!(binary_decode("0100000101000010").expect("no spaces"), "AB");
        assert_eq!(
            binary_decode(&binary_encode("héllo")).expect("unicode"),
            "héllo"
        );
        assert!(binary_decode("0101").is_err());
        assert!(binary_decode("0100000a").is_err());
        assert_eq!(binary_decode("").expect("empty"), "");
    }

    #[test]
    fn morse_known_vectors() {
        assert_eq!(morse_encode("SOS").expect("encode"), "... --- ...");
        assert_eq!(
            morse_encode("sos sos").expect("words"),
            "... --- ... / ... --- ..."
        );
        assert_eq!(morse_encode("?").expect("punct"), "..--..");
        assert_eq!(
            morse_decode("....  .  .-..  .-.. ---").expect("decode"),
            "HELLO"
        );
        assert_eq!(
            morse_decode("... --- ... / ... --- ...").expect("words"),
            "SOS SOS"
        );
        assert_eq!(morse_decode(".-.-.-").expect("punct"), ".");
        assert_eq!(
            morse_decode(&morse_encode("HELLO, WORLD!").expect("encode")).expect("roundtrip"),
            "HELLO, WORLD!"
        );
        assert!(morse_decode("........").is_err());
        assert!(morse_encode("é").is_err());
    }

    #[test]
    fn dispatchers_accept_any_case_and_reject_unknown() {
        assert_eq!(encode("BASE64", "hi").expect("case"), "aGk=");
        assert_eq!(decode("Hex", "6869").expect("case"), "hi");
        assert!(encode("base32", "hi").is_err());
        assert!(decode("base32", "hi").is_err());
    }

    #[test]
    fn jwt_fixture_decodes() {
        let (header, payload, claims) = decode_jwt(JWT).expect("jwt");
        assert!(header.contains("\"alg\": \"HS256\""));
        assert!(payload.contains("John Doe"));
        assert!(
            claims
                .iter()
                .any(|(key, value)| key == "name" && value == "John Doe")
        );
        assert!(
            claims
                .iter()
                .any(|(key, value)| key == "iat" && value == "1516239022")
        );
    }

    #[test]
    fn jwt_rejects_malformed_tokens() {
        assert!(decode_jwt("only.two").is_err());
        assert!(decode_jwt("").is_err());
        assert!(decode_jwt("bm90anNvbg.bm90anNvbg.sig").is_err()); // not JSON
        assert!(decode_jwt("e30.e30.sig").is_ok()); // {} header/payload, empty claims
    }
}
