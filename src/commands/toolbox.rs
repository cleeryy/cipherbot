//! `/tool` — encoding, hashing, CVSS and other tradecraft (toolbox lane).
//!
//! Every handler replies once. Long or sensitive output is ephemeral; the
//! password check acknowledges with an ephemeral placeholder before its network
//! call and is limited to one lookup every three seconds per user.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::bot::{BotResult, SharedState};
use crate::util::{cvss, embeds, encodings, entropy, hash, hashid, opts, respond, text, time};
use anyhow::{Context as _, Result};
use serenity::all::{
    Colour, CommandInteraction, CommandOptionType, Context, CreateCommand, CreateCommandOption,
    CreateEmbed,
};

/// Maximum characters kept from an encode/decode result inside the fence.
const MAX_OUTPUT: usize = 1_900;
const PASSWORD_COOLDOWN: Duration = Duration::from_secs(3);
const HIBP_RANGE_URL: &str = "https://api.pwnedpasswords.com/range/";

pub fn commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("tool")
            .description("Encoding, hashing, CVSS and tradecraft tools")
            .add_option(encode_like(
                "encode",
                "Encode text as base64, base64url, hex, URL, ROT13, binary or Morse",
            ))
            .add_option(encode_like(
                "decode",
                "Decode base64, base64url, hex, URL, ROT13, binary or Morse",
            ))
            .add_option(single(
                "hashid",
                "Identify a hash, digest or token format",
                "value",
                "Hash or token to identify",
                500,
            ))
            .add_option(single(
                "entropy",
                "Estimate the entropy and crack time of a string",
                "value",
                "Text to analyse (never echoed back)",
                500,
            ))
            .add_option(single(
                "cvss",
                "Calculate a CVSS 3.0/3.1 base score and severity",
                "vector",
                "Vector, e.g. CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
                200,
            ))
            .add_option(single(
                "password",
                "Check password strength and known breaches (ephemeral)",
                "value",
                "Password to check (ephemeral, never logged)",
                200,
            ))
            .add_option(single(
                "dork",
                "Generate Google dorks for a domain you are authorised to test",
                "target",
                "Domain or URL, e.g. example.com",
                253,
            ))
            .add_option(single(
                "jwt",
                "Decode a JWT header and payload (signature not verified)",
                "token",
                "JWT to decode (ephemeral)",
                2_000,
            )),
    ]
}

fn encode_like(name: &str, description: &str) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, name, description)
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::String, "scheme", "Encoding scheme")
                .required(true)
                .add_string_choice("base64", "base64")
                .add_string_choice("base64url", "base64url")
                .add_string_choice("hex", "hex")
                .add_string_choice("url", "url")
                .add_string_choice("rot13", "rot13")
                .add_string_choice("binary", "binary")
                .add_string_choice("morse", "morse"),
        )
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::String, "input", "Text to process")
                .required(true)
                .max_length(1_900),
        )
}

fn single(
    name: &str,
    description: &str,
    field: &str,
    field_description: &str,
    max_length: u16,
) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, name, description).add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, field, field_description)
            .required(true)
            .max_length(max_length),
    )
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some((name, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown /tool subcommand.").await;
    };

    match name {
        "encode" => transform(ctx, cmd, options, true).await,
        "decode" => transform(ctx, cmd, options, false).await,
        "hashid" => hashid_cmd(ctx, cmd, options).await,
        "entropy" => entropy_cmd(ctx, cmd, options).await,
        "cvss" => cvss_cmd(ctx, cmd, options).await,
        "password" => password_cmd(state, ctx, cmd, options).await,
        "dork" => dork_cmd(ctx, cmd, options).await,
        "jwt" => jwt_cmd(ctx, cmd, options).await,
        other => respond::error(ctx, cmd, format!("Unknown /tool subcommand `{other}`.")).await,
    }
}

// -------------------------------------------------------------- encode/decode --

async fn transform(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
    encode: bool,
) -> BotResult<()> {
    let scheme = opts::req_str(options, "scheme")?;
    let input = opts::req_str(options, "input")?;

    let result = if encode {
        encodings::encode(scheme, input)
    } else {
        encodings::decode(scheme, input)
    };
    let output = match result {
        Ok(output) => output,
        Err(error) => return respond::error(ctx, cmd, error.to_string()).await,
    };

    let action = if encode { "Encoded" } else { "Decoded" };
    let content = format!("**{action} — {scheme}**\n{}", code_block(&output));
    respond::text(ctx, cmd, content, true).await
}

// ------------------------------------------------------------------- hashid --

async fn hashid_cmd(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let value = opts::req_str(options, "value")?;
    let guesses = hashid::identify(value);
    if guesses.is_empty() {
        return respond::text(
            ctx,
            cmd,
            "No known hash, digest or token format matched.",
            false,
        )
        .await;
    }

    let description = text::truncate(&hashid_lines(&guesses), 3_900);
    respond::public(ctx, cmd, embeds::info("Hash identification", description)).await
}

fn hashid_lines(guesses: &[hashid::HashGuess]) -> String {
    guesses
        .iter()
        .map(|guess| {
            format!(
                "**{}** — {} confidence\n{}",
                guess.name,
                guess.confidence.label(),
                guess.note
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ------------------------------------------------------------------ entropy --

async fn entropy_cmd(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let value = opts::req_str(options, "value")?;
    let report = entropy::analyze(value);
    respond::public(ctx, cmd, entropy_embed(&report)).await
}

fn entropy_embed(report: &entropy::EntropyReport) -> CreateEmbed {
    let description = format!(
        "**Strength:** {} — {:.1} bits total ({:.2} bits/char)\n\
         **Estimated crack time:** {} at 1e10 guesses/s\n\
         -# The analysed value is never stored or echoed.",
        report.label,
        report.total_bits,
        report.shannon_bits_per_char,
        human_crack(report.crack_time_seconds),
    );
    embeds::footer(
        CreateEmbed::new()
            .title("Entropy analysis")
            .description(description)
            .colour(strength_colour(report.label))
            .field("Length", report.length.to_string(), true)
            .field("Unique chars", report.unique_chars.to_string(), true)
            .field("Charset size", report.charset_size.to_string(), true),
    )
}

fn strength_colour(label: &str) -> Colour {
    match label {
        "very strong" | "strong" => embeds::BRAND,
        "reasonable" => embeds::BLUE,
        "weak" => embeds::AMBER,
        _ => embeds::RED,
    }
}

/// Crack time at 10^10 guesses/s, floored and humanised.
fn human_crack(seconds: f64) -> String {
    const CENTURIES_SECONDS: f64 = 3.15e9; // ~100 years
    if !seconds.is_finite() || seconds >= CENTURIES_SECONDS {
        "> centuries".to_owned()
    } else if seconds < 1.0 {
        "instantly (<1s)".to_owned()
    } else {
        time::human_duration(seconds as i64)
    }
}

// -------------------------------------------------------------------- cvss --

async fn cvss_cmd(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let raw = opts::req_str(options, "vector")?;
    let vector = match cvss::parse_vector(raw) {
        Ok(vector) => vector,
        Err(error) => return respond::error(ctx, cmd, error.to_string()).await,
    };
    let score = cvss::base_score(&vector);
    respond::public(ctx, cmd, cvss_embed(&vector, score)).await
}

fn cvss_embed(vector: &cvss::CvssVector, score: f64) -> CreateEmbed {
    let severity = cvss::severity(score);
    let mut embed = CreateEmbed::new()
        .title(format!("CVSS {severity} — {score:.1}"))
        .description(format!("```\n{vector}\n```"))
        .colour(cvss_colour(score));

    for (name, value) in cvss_fields(vector) {
        embed = embed.field(name, value, true);
    }
    embeds::footer(embed)
}

fn cvss_fields(vector: &cvss::CvssVector) -> Vec<(&'static str, &'static str)> {
    vec![
        ("AV — Attack Vector", vector.av.label()),
        ("AC — Attack Complexity", vector.ac.label()),
        ("PR — Privileges Required", vector.pr.label()),
        ("UI — User Interaction", vector.ui.label()),
        ("S — Scope", vector.scope.label()),
        ("C — Confidentiality", vector.confidentiality.label()),
        ("I — Integrity", vector.integrity.label()),
        ("A — Availability", vector.availability.label()),
    ]
}

fn cvss_colour(score: f64) -> Colour {
    if score >= 9.0 {
        embeds::RED
    } else if score >= 7.0 {
        Colour::new(0x00F7_6B00)
    } else if score >= 4.0 {
        embeds::AMBER
    } else {
        embeds::BLUE
    }
}

// ---------------------------------------------------------------- password --

async fn password_cmd(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    if !cooldown_ok(cmd.user.id.get(), Instant::now()) {
        return respond::error(
            ctx,
            cmd,
            "Password checks are limited to one every 3 seconds.",
        )
        .await;
    }

    let value = opts::req_str(options, "value")?;
    respond::working(ctx, cmd, "Analysing strength and checking known breaches…").await?;

    let report = entropy::analyze(value);
    let breach = match hibp_lookup(state, value).await {
        Ok(count) => Some(count),
        Err(error) => {
            tracing::warn!(%error, "HIBP range lookup failed");
            None
        }
    };

    respond::edit(ctx, cmd, password_embed(&report, breach)).await
}

fn password_embed(report: &entropy::EntropyReport, breach: Option<u32>) -> CreateEmbed {
    embeds::footer(
        CreateEmbed::new()
            .title(format!("Password check — {}", report.label))
            .description(password_description(breach))
            .colour(strength_colour(report.label))
            .field("Length", report.length.to_string(), true)
            .field("Charset size", report.charset_size.to_string(), true)
            .field(
                "Entropy",
                format!(
                    "{:.1} bits ({:.2}/char)",
                    report.total_bits, report.shannon_bits_per_char
                ),
                true,
            )
            .field("Crack time", human_crack(report.crack_time_seconds), true),
    )
}

fn password_description(breach: Option<u32>) -> String {
    format!("{}\n\n{}", breach_line(breach), password_guidance())
}

fn breach_line(breach: Option<u32>) -> String {
    match breach {
        Some(0) => "✅ Not found in the Pwned Passwords corpus.".to_owned(),
        Some(count) => {
            format!("⚠️ Found in **{count}** known breaches — change it everywhere it is reused.")
        }
        None => "⚠️ Breach check unavailable (network error). Strength estimate only.".to_owned(),
    }
}

fn password_guidance() -> &'static str {
    "• Use at least 16 characters and a unique password per site.\n\
     • Prefer a password manager and multi-factor authentication.\n\
     -# Only the first 5 characters of the SHA-1 are sent to Pwned Passwords (k-anonymity)."
}

/// `(first 5 hex chars, remaining 35 hex chars)` of the uppercase SHA-1.
fn hibp_prefix(password: &str) -> (String, String) {
    let hash = hash::sha1_hex(password).to_ascii_uppercase();
    let prefix = hash.get(..5).unwrap_or_default().to_owned();
    let suffix = hash.get(5..).unwrap_or_default().to_owned();
    (prefix, suffix)
}

fn hibp_url(prefix: &str) -> String {
    format!("{HIBP_RANGE_URL}{prefix}")
}

/// Parse `SUFFIX:COUNT` lines and return the count for `suffix` (0 if absent).
fn parse_hibp_body(body: &str, suffix: &str) -> u32 {
    for line in body.lines() {
        let Some((candidate, count)) = line.trim().split_once(':') else {
            continue;
        };
        if candidate.trim().eq_ignore_ascii_case(suffix) {
            return count.trim().parse::<u32>().unwrap_or(0);
        }
    }
    0
}

async fn hibp_lookup(state: &SharedState, password: &str) -> Result<u32> {
    let (prefix, suffix) = hibp_prefix(password);
    let response = state
        .http
        .get(hibp_url(&prefix))
        .header("Add-Padding", "true")
        .send()
        .await
        .context("Pwned Passwords request failed")?;

    if !response.status().is_success() {
        anyhow::bail!("Pwned Passwords returned HTTP {}", response.status());
    }
    let body = response
        .text()
        .await
        .context("Pwned Passwords response could not be read")?;
    Ok(parse_hibp_body(&body, &suffix))
}

static PASSWORD_USES: OnceLock<parking_lot::Mutex<HashMap<u64, Instant>>> = OnceLock::new();

/// Sliding 3 s per-user cooldown. Inserts on success so double-clicks are blocked.
fn cooldown_ok(user_id: u64, now: Instant) -> bool {
    let uses = PASSWORD_USES.get_or_init(|| parking_lot::Mutex::new(HashMap::new()));
    let mut guard = uses.lock();

    if guard.len() > 4_096 {
        guard.retain(|_, last| now.duration_since(*last) < PASSWORD_COOLDOWN);
    }

    match guard.get(&user_id) {
        Some(last) if now.duration_since(*last) < PASSWORD_COOLDOWN => false,
        _ => {
            guard.insert(user_id, now);
            true
        }
    }
}

// -------------------------------------------------------------------- dork --

async fn dork_cmd(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let target = opts::req_str(options, "target")?;
    let domain = match derive_domain(target) {
        Ok(domain) => domain,
        Err(error) => return respond::error(ctx, cmd, error.to_string()).await,
    };

    let queries = dork_queries(&domain)
        .iter()
        .map(|(label, query)| format!("[{label}] {query}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "**Google dorks — {domain}**\n{}\n-# Only test assets you own or are authorised to test.",
        code_block(&queries)
    );
    respond::text(ctx, cmd, content, false).await
}

fn dork_queries(domain: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Indexed pages", format!("site:{domain}")),
        ("PDF documents", format!("site:{domain} ext:pdf")),
        (
            "Directory listings",
            format!("site:{domain} intitle:index.of"),
        ),
        ("Admin panels", format!("site:{domain} inurl:admin")),
        ("SQL dumps", format!("site:{domain} ext:sql")),
        (
            "Password mentions",
            format!("site:{domain} intext:\"password\""),
        ),
        (
            "Backup listings",
            format!("site:{domain} intitle:\"index of\" \"backup\""),
        ),
        ("Env files", format!("site:{domain} ext:env")),
        ("Login pages", format!("site:{domain} inurl:login")),
        ("API keys", format!("site:{domain} \"api_key\"")),
    ]
}

fn derive_domain(target: &str) -> Result<String> {
    let raw = target.trim();
    if raw.is_empty() {
        anyhow::bail!("Give a domain or URL, e.g. example.com.");
    }

    let candidate = if text::is_url(raw) {
        text::domain_of(raw)
            .ok_or_else(|| anyhow::anyhow!("Could not extract a domain from `{raw}`."))?
    } else {
        let without_path = raw.split('/').next().unwrap_or(raw);
        let without_port = without_path.split(':').next().unwrap_or(without_path);
        let trimmed = without_port.trim_end_matches('.').to_ascii_lowercase();
        trimmed
            .strip_prefix("www.")
            .unwrap_or(trimmed.as_str())
            .to_owned()
    };

    if !is_domain_like(&candidate) {
        anyhow::bail!("`{candidate}` is not a valid domain name.");
    }
    Ok(candidate)
}

fn is_domain_like(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 253 || !domain.contains('.') {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    }) {
        return false;
    }
    match labels.last() {
        Some(tld) => tld.len() >= 2 && !tld.chars().all(|ch| ch.is_ascii_digit()),
        None => false,
    }
}

// --------------------------------------------------------------------- jwt --

async fn jwt_cmd(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let token = opts::req_str(options, "token")?;
    let (header, payload, claims) = match encodings::decode_jwt(token) {
        Ok(parts) => parts,
        Err(error) => return respond::error(ctx, cmd, error.to_string()).await,
    };

    let algorithm = serde_json::from_str::<serde_json::Value>(&header)
        .ok()
        .and_then(|value| {
            value
                .get("alg")
                .and_then(|alg| alg.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());

    let description = format!(
        "⚠️ **Signature NOT verified** — never trust this output for authentication.\n\n\
         **Header**\n```json\n{}\n```\n\
         **Payload**\n```json\n{}\n```\n{}",
        fence_safe(&header),
        fence_safe(&payload),
        claims_lines(&claims),
    );
    let description = text::truncate(&description, 4_000);

    let embed = embeds::footer(
        CreateEmbed::new()
            .title("JWT decoded")
            .description(description)
            .colour(embeds::AMBER)
            .field("alg", text::truncate(&algorithm, 100), true)
            .field("Segments", "3", true),
    );
    respond::ephemeral(ctx, cmd, embed).await
}

fn claims_lines(claims: &[(String, String)]) -> String {
    if claims.is_empty() {
        return String::new();
    }
    let lines = claims
        .iter()
        .map(|(key, value)| {
            let rendered = match key.as_str() {
                "exp" | "iat" | "nbf" => match value.parse::<i64>() {
                    Ok(timestamp) => format!("{} ({timestamp})", time::human_date(timestamp)),
                    Err(_) => text::truncate(value, 200),
                },
                _ => text::truncate(value, 200),
            };
            format!(
                "• **{}**: {}",
                text::escape_md(&text::truncate(key, 64)),
                text::escape_md(&rendered)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("**Claims**\n{lines}")
}

// ---------------------------------------------------------------- rendering --

/// Fenced code block, truncating the body to [`MAX_OUTPUT`] characters.
fn code_block(output: &str) -> String {
    let display = if output.is_empty() { "(empty)" } else { output };
    let safe = fence_safe(display);
    let body = text::truncate(&safe, MAX_OUTPUT);
    let note = if safe.chars().count() > MAX_OUTPUT {
        "\n-# Output truncated to 1,900 characters."
    } else {
        ""
    };
    format!("```\n{body}\n```{note}")
}

/// Neutralise triple backticks so untrusted output cannot break out of a fence.
fn fence_safe(input: &str) -> String {
    input.replace("```", "`\u{200b}``")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::hashid::Confidence;

    #[test]
    fn command_surface_is_complete() {
        let commands = commands();
        assert_eq!(commands.len(), 1);

        let value = serde_json::to_value(&commands[0]).expect("serialize");
        assert_eq!(value["name"], "tool");
        let options = value["options"].as_array().expect("options");

        let names: Vec<&str> = options
            .iter()
            .map(|option| option["name"].as_str().unwrap_or_default())
            .collect();
        for expected in [
            "encode", "decode", "hashid", "entropy", "cvss", "password", "dork", "jwt",
        ] {
            assert!(names.contains(&expected), "missing /tool {expected}");
        }
        assert_eq!(names.len(), 8);

        for option in options {
            let description = option["description"].as_str().unwrap_or_default();
            assert!(!description.is_empty() && description.chars().count() <= 100);
            for sub in option["options"].as_array().cloned().unwrap_or_default() {
                let sub_description = sub["description"].as_str().unwrap_or_default();
                assert!(!sub_description.is_empty() && sub_description.chars().count() <= 100);
            }
        }

        let encode = options
            .iter()
            .find(|option| option["name"] == "encode")
            .expect("encode");
        let scheme = encode["options"]
            .as_array()
            .expect("encode options")
            .iter()
            .find(|option| option["name"] == "scheme")
            .expect("scheme");
        assert_eq!(
            scheme["choices"].as_array().map(Vec::len),
            Some(encodings::SCHEMES.len())
        );
    }

    #[test]
    fn code_block_wraps_truncates_and_escapes() {
        let block = code_block("aGVsbG8=");
        assert!(block.starts_with("```\n"));
        assert!(block.ends_with("\n```"));
        assert!(block.contains("aGVsbG8="));

        let long = "a".repeat(2_500);
        let block = code_block(&long);
        assert!(block.contains("truncated"));
        assert!(block.chars().count() < 2_000);

        let hostile = "```\n@everyone";
        let block = code_block(hostile);
        assert!(!block[3..block.len() - 3].contains("```"));

        assert!(code_block("").contains("(empty)"));
    }

    #[test]
    fn human_crack_is_readable() {
        assert_eq!(human_crack(0.2), "instantly (<1s)");
        assert_eq!(human_crack(90.0), "1m 30s");
        assert_eq!(human_crack(1.0e12), "> centuries");
        assert_eq!(human_crack(f64::MAX), "> centuries");
        assert_eq!(human_crack(f64::INFINITY), "> centuries");
    }

    #[test]
    fn hashid_lines_show_confidence_and_note() {
        let guesses = vec![hashid::HashGuess {
            name: "MD5",
            confidence: Confidence::Medium,
            note: "ambiguous",
        }];
        let lines = hashid_lines(&guesses);
        assert!(lines.contains("MD5"));
        assert!(lines.contains("medium"));
        assert!(lines.contains("ambiguous"));
    }

    #[test]
    fn domains_are_derived_from_domains_and_urls() {
        assert_eq!(derive_domain("example.com").expect("domain"), "example.com");
        assert_eq!(
            derive_domain("https://www.Example.COM/path?q=1").expect("url"),
            "example.com"
        );
        assert_eq!(
            derive_domain("sub.example.co.uk/path").expect("path"),
            "sub.example.co.uk"
        );
        assert_eq!(
            derive_domain("example.com:8443").expect("port"),
            "example.com"
        );
        assert!(derive_domain("").is_err());
        assert!(derive_domain("localhost").is_err());
        assert!(derive_domain("not a domain").is_err());
        assert!(derive_domain("http://").is_err());
    }

    #[test]
    fn dorks_cover_the_expected_queries() {
        let queries = dork_queries("example.com");
        assert_eq!(queries.len(), 10);
        assert_eq!(queries[0].1, "site:example.com");
        assert!(
            queries
                .iter()
                .all(|(_, query)| query.contains("example.com"))
        );
        assert!(queries.iter().any(|(_, query)| query.contains("ext:pdf")));
        assert!(
            queries
                .iter()
                .any(|(_, query)| query.contains("inurl:login"))
        );
        assert!(queries.iter().any(|(_, query)| query.contains("ext:env")));
        assert!(queries.iter().any(|(_, query)| query.contains("api_key")));
    }

    #[test]
    fn hibp_prefix_is_five_chars_and_url_leaks_nothing_else() {
        let (prefix, suffix) = hibp_prefix("password");
        assert_eq!(prefix.len(), 5);
        assert_eq!(suffix.len(), 35);
        assert_eq!(prefix, "5BAA6");
        assert_eq!(
            format!("{prefix}{suffix}"),
            "5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8"
        );

        let url = hibp_url(&prefix);
        assert_eq!(url, "https://api.pwnedpasswords.com/range/5BAA6");
        assert!(!url.contains(&suffix));

        let (prefix, suffix) = hibp_prefix("correct horse battery staple");
        let url = hibp_url(&prefix);
        assert_eq!(prefix.len(), 5);
        assert!(!url.contains("correct"));
        assert!(!url.contains(&suffix));
    }

    #[test]
    fn hibp_body_parsing() {
        let body = "0018A45C4D1DEF81644B54AB7F969B88D65:1\r\n\
                    1E4C9B93F3F0682250B6CF8331B7EE68FD8:3861493\r\n\
                    00D4F6E8FA6EECAD2A3AA415EEC418D38EC:0";
        assert_eq!(
            parse_hibp_body(body, "1E4C9B93F3F0682250B6CF8331B7EE68FD8"),
            3_861_493
        );
        assert_eq!(
            parse_hibp_body(body, "1e4c9b93f3f0682250b6cf8331b7ee68fd8"),
            3_861_493
        );
        assert_eq!(
            parse_hibp_body(body, "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            0
        );
        assert_eq!(parse_hibp_body("garbage", "ABC"), 0);
        assert_eq!(parse_hibp_body("", "ABC"), 0);
    }

    #[test]
    fn password_output_never_contains_the_secret() {
        let secret = "correct horse battery staple";
        let report = entropy::analyze(secret);
        let description = password_description(Some(3));
        assert!(description.contains("Found in **3**"));
        assert!(!description.contains(secret));
        assert!(!description.contains("correct"));
        assert_eq!(report.length, secret.chars().count());

        let unavailable = password_description(None);
        assert!(unavailable.contains("unavailable"));
        assert!(!unavailable.contains(secret));
    }

    #[test]
    fn password_cooldown_blocks_rapid_checks() {
        let user = 987_654_321_000_042;
        let start = Instant::now();
        assert!(cooldown_ok(user, start));
        assert!(!cooldown_ok(user, start + Duration::from_secs(1)));
        assert!(!cooldown_ok(user, start + Duration::from_secs(2)));
        assert!(cooldown_ok(user, start + Duration::from_secs(4)));
        assert!(cooldown_ok(user + 1, start + Duration::from_secs(1)));
    }

    #[test]
    fn claims_render_dates_and_escape_markdown() {
        let claims = vec![
            ("exp".to_owned(), "1516239022".to_owned()),
            ("name".to_owned(), "*John*".to_owned()),
            ("nested".to_owned(), "{\"a\":1}".to_owned()),
            ("bad_exp".to_owned(), "nope".to_owned()),
        ];
        let lines = claims_lines(&claims);
        assert!(lines.contains("2018-01-18"));
        assert!(lines.contains("UTC"));
        assert!(lines.contains("\\*John\\*"));
        assert!(lines.contains("nope"));
        assert!(claims_lines(&[]).is_empty());
    }

    #[test]
    fn entropy_embed_mentions_strength_and_charset() {
        let report = entropy::analyze("Xk9#mQ2$vL7@pR4!nZ8&");
        let embed = entropy_embed(&report);
        let _ = embed;
        assert_eq!(strength_colour(report.label).0, embeds::BRAND.0);
        assert_eq!(strength_colour("very weak").0, embeds::RED.0);
    }

    #[test]
    fn cvss_embed_fields_cover_every_base_metric() {
        let vector =
            cvss::parse_vector("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H").expect("vector");
        let fields = cvss_fields(&vector);
        assert_eq!(fields.len(), 8);
        assert_eq!(fields[0].1, "Network (N)");
        assert_eq!(fields[4].1, "Unchanged (U)");
        assert_eq!(fields[5].1, "High (H)");
        assert_eq!(cvss_colour(10.0).0, embeds::RED.0);
        assert_eq!(cvss_colour(5.0).0, embeds::AMBER.0);
    }
}
