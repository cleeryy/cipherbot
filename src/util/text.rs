//! Small, allocation-friendly text helpers shared by every command.

use url::Url;

#[allow(unused_imports)]
pub use super::hash::sha256_hex;

/// Truncate to at most `max` characters, appending `…` when cut.
///
/// Character-based, so multi-byte input never panics.
pub fn truncate(input: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if input.chars().count() <= max {
        return input.to_owned();
    }
    let mut out: String = input.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Escape characters that Discord would render as markdown.
pub fn escape_md(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '[' | ']') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Lowercased host of an absolute URL, without a leading `www.`.
pub fn domain_of(input: &str) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(host.strip_prefix("www.").unwrap_or(&host).to_owned())
}

/// True when the string is an absolute http(s) URL with a host.
pub fn is_url(input: &str) -> bool {
    match Url::parse(input.trim()) {
        Ok(url) => matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("héllo wörld", 6), "héllo…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abc", 1), "…");
    }

    #[test]
    fn escape_markdown() {
        assert_eq!(escape_md("*bold*"), "\\*bold\\*");
        assert_eq!(escape_md("a_b"), "a\\_b");
        assert_eq!(escape_md("plain text"), "plain text");
        assert_eq!(escape_md("`code`"), "\\`code\\`");
    }

    #[test]
    fn domain_extraction() {
        assert_eq!(
            domain_of("https://www.Example.COM/path?q=1"),
            Some("example.com".to_owned())
        );
        assert_eq!(
            domain_of("http://sub.example.org"),
            Some("sub.example.org".to_owned())
        );
        assert_eq!(domain_of("not a url"), None);
    }

    #[test]
    fn url_detection() {
        assert!(is_url("https://example.com"));
        assert!(is_url("http://example.com/a"));
        assert!(!is_url("example.com"));
        assert!(!is_url("ftp://example.com"));
        assert!(!is_url("javascript:alert(1)"));
        assert!(!is_url(""));
    }

    #[test]
    fn sha256_reexport_matches() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
