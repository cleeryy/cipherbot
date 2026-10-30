//! Automod pipeline hook.
//!
//! Runs before the link pipeline. Pure detection lives in [`evaluate`] so it can
//! be unit-tested without Discord; [`on_message`] only performs the side effects
//! (delete, evidence, modlog, case).
//!
//! Duplicate spam needs memory, so it is *not* part of `evaluate`: the tracker in
//! [`note_duplicate`] is consulted from `on_message` and appended as a violation.
//!
//! Note: without the privileged `MESSAGE_CONTENT` intent `Message::content` is
//! empty, so content-based checks silently do nothing. Mention and invite checks
//! still work on the fields Discord always sends.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use regex::Regex;
use serenity::all::{Context, CreateEmbed, GuildId, Message, Permissions, Timestamp, UserId};

use crate::bot::{BotResult, SharedState};
use crate::db::GuildSettings;
use crate::db::guild::AutomodSettings;
use crate::handler::HookResult;
use crate::security::phishing;
use crate::util::text::{domain_of, escape_md, truncate};
use crate::util::time::unix_now;

/// Mentions at or above this count are mass mentions.
pub const MASS_MENTION_THRESHOLD: usize = 8;
/// Two identical messages must arrive within this window to count as spam.
pub const DUPLICATE_WINDOW_SECS: i64 = 30;
/// Repetitions within the window before the message is treated as spam.
pub const DUPLICATE_LIMIT: u8 = 3;
/// Upper bound on domains kept from the feeds.
pub const MAX_FEED_ENTRIES: usize = 100_000;

const FEEDS: [(&str, &str); 2] = [
    ("openphish", "https://openphish.com/feed.txt"),
    ("urlhaus", "https://urlhaus.abuse.ch/downloads/text/"),
];

/// One detected automod rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    Invite,
    Phishing,
    MassMention,
    Duplicate,
    Caps,
    TokenLeak,
}

impl Violation {
    /// Short human label used in evidence reasons and modlog embeds.
    pub fn label(self) -> &'static str {
        match self {
            Self::Invite => "invite link",
            Self::Phishing => "phishing link",
            Self::MassMention => "mass mention",
            Self::Duplicate => "duplicate spam",
            Self::Caps => "excessive caps",
            Self::TokenLeak => "credential leak",
        }
    }

    /// High-severity rules also open a case.
    pub fn creates_case(self) -> bool {
        matches!(self, Self::Phishing | Self::TokenLeak | Self::MassMention)
    }
}

static URL_REGEX: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?i)\bhttps?://[^\s<>"'`]+"#).ok());

static INVITE_REGEX: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:discord\.gg/|discord(?:app)?\.com/invite/)[a-z0-9-]+").ok()
});

/// Discord bot tokens, AWS keys, GitHub tokens, PEM private keys and Slack tokens.
static SECRET_REGEX: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r"[MN][A-Za-z0-9]{23,25}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27,40}",
        r"|AKIA[0-9A-Z]{16}",
        r"|gh[pousr]_[A-Za-z0-9]{36,}",
        r"|-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----",
        r"|xox[baprs]-[A-Za-z0-9-]+",
    ))
    .ok()
});

/// Pure rule evaluation. Never returns [`Violation::Duplicate`] (stateful).
pub fn evaluate(
    content: &str,
    mention_count: usize,
    everyone_mentioned: bool,
    settings: &AutomodSettings,
    bad_domain: impl Fn(&str) -> bool,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    if settings.invites && has_invite(content) {
        violations.push(Violation::Invite);
    }
    if settings.phishing && has_phishing_url(content, &bad_domain) {
        violations.push(Violation::Phishing);
    }
    if settings.mass_mentions && (mention_count >= MASS_MENTION_THRESHOLD || everyone_mentioned) {
        violations.push(Violation::MassMention);
    }
    if settings.caps && is_caps(content) {
        violations.push(Violation::Caps);
    }
    if settings.token_leaks && has_secret(content) {
        violations.push(Violation::TokenLeak);
    }

    violations
}

pub fn has_invite(content: &str) -> bool {
    INVITE_REGEX
        .as_ref()
        .is_some_and(|regex| regex.is_match(content))
}

fn has_phishing_url(content: &str, bad_domain: &impl Fn(&str) -> bool) -> bool {
    URL_REGEX.as_ref().is_some_and(|regex| {
        regex.find_iter(content).any(|candidate| {
            domain_of(candidate.as_str()).is_some_and(|domain| bad_domain(&domain))
        })
    })
}

/// Public so flagwatch and tests can reuse the same secret detector.
pub fn has_secret(content: &str) -> bool {
    SECRET_REGEX
        .as_ref()
        .is_some_and(|regex| regex.is_match(content))
}

/// Replace every detected secret with `[redacted]` before logging or storing it.
pub fn redact_secrets(content: &str) -> String {
    match SECRET_REGEX.as_ref() {
        Some(regex) => regex.replace_all(content, "[redacted]").into_owned(),
        None => content.to_owned(),
    }
}

/// `>= 70%` uppercase and at least 12 letters.
pub fn is_caps(content: &str) -> bool {
    let mut letters = 0usize;
    let mut upper = 0usize;
    for ch in content.chars() {
        if ch.is_alphabetic() {
            letters += 1;
            if ch.is_uppercase() {
                upper += 1;
            }
        }
    }
    letters >= 12 && upper * 10 >= letters * 7
}

type DuplicateKey = (u64, u64, String);

static DUPLICATES: LazyLock<Mutex<HashMap<DuplicateKey, (i64, u8)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record a message and report whether it completes a spam burst.
///
/// Keyed by `(guild, author, normalized content)`. Three identical messages within
/// [`DUPLICATE_WINDOW_SECS`] return `true`.
pub fn note_duplicate(guild_id: u64, user_id: u64, content: &str, now: i64) -> bool {
    let normalized = normalize_content(content);
    if normalized.is_empty() {
        return false;
    }

    let mut tracker = match DUPLICATES.lock() {
        Ok(tracker) => tracker,
        Err(poisoned) => poisoned.into_inner(),
    };

    if tracker.len() > 5_000 {
        tracker.retain(|_, (last_seen, _)| now.saturating_sub(*last_seen) <= 120);
    }

    let entry = tracker
        .entry((guild_id, user_id, normalized))
        .or_insert((now, 0));

    if now.saturating_sub(entry.0) > DUPLICATE_WINDOW_SECS {
        *entry = (now, 1);
        return false;
    }

    entry.0 = now;
    entry.1 = entry.1.saturating_add(1);
    entry.1 >= DUPLICATE_LIMIT
}

fn normalize_content(content: &str) -> String {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&collapsed.to_ascii_lowercase(), 200)
}

/// True when the author is staff (administrator or manage-messages).
///
/// Uses the guild cache only: no HTTP call on the message hot path. If the guild
/// or member is not cached the author is treated as a regular member.
pub(crate) fn is_staff(ctx: &Context, guild_id: GuildId, user_id: UserId) -> bool {
    let Some(guild) = guild_id.to_guild_cached(&ctx.cache) else {
        return false;
    };
    let Some(member) = guild.members.get(&user_id) else {
        return false;
    };
    let permissions = guild.member_permissions(member);
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_MESSAGES)
}

/// Staff check with a REST fallback.
///
/// `Member::permissions` is deprecated and the member cache is empty without the
/// privileged `GUILD_MEMBERS` intent; fetching the member once (only when a rule
/// already matched) keeps moderators safe without a per-message HTTP call.
pub(crate) async fn is_staff_checked(ctx: &Context, guild_id: GuildId, user_id: UserId) -> bool {
    if is_staff(ctx, guild_id, user_id) {
        return true;
    }

    let Ok(member) = guild_id.member(&ctx.http, user_id).await else {
        return false;
    };
    let Some(guild) = guild_id.to_guild_cached(&ctx.cache) else {
        return false;
    };
    let permissions = guild.member_permissions(&member);
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_MESSAGES)
}

pub async fn on_message(state: &SharedState, ctx: &Context, msg: &Message) -> HookResult {
    if msg.author.bot || msg.webhook_id.is_some() {
        return HookResult::Continue;
    }
    let Some(guild_id) = msg.guild_id else {
        return HookResult::Continue;
    };

    let bot_id = state
        .bot_user_id
        .get()
        .copied()
        .unwrap_or_else(|| ctx.cache.current_user().id);
    if msg.author.id == bot_id {
        return HookResult::Continue;
    }
    if is_staff(ctx, guild_id, msg.author.id) {
        return HookResult::Continue;
    }

    let settings = state
        .db
        .get_settings(guild_id.get())
        .unwrap_or_else(|error| {
            tracing::warn!("Automod could not load settings for {guild_id}: {error}");
            GuildSettings::default()
        });

    let mut violations = evaluate(
        &msg.content,
        msg.mentions.len(),
        msg.mention_everyone,
        &settings.automod,
        |domain| phishing::is_bad_domain(state, domain),
    );

    if settings.automod.duplicate_spam
        && note_duplicate(
            guild_id.get(),
            msg.author.id.get(),
            &msg.content,
            unix_now(),
        )
    {
        violations.push(Violation::Duplicate);
    }

    if violations.is_empty() {
        return HookResult::Continue;
    }

    // The cache usually lacks members without the GUILD_MEMBERS intent, so re-check
    // staff status over REST before acting (rare path: a rule already matched).
    if is_staff_checked(ctx, guild_id, msg.author.id).await {
        return HookResult::Continue;
    }

    apply_violations(state, ctx, msg, guild_id, bot_id, &violations).await;
    HookResult::Stop
}

async fn apply_violations(
    state: &SharedState,
    ctx: &Context,
    msg: &Message,
    guild_id: GuildId,
    bot_id: UserId,
    violations: &[Violation],
) {
    let labels = violations
        .iter()
        .map(|violation| violation.label())
        .collect::<Vec<_>>()
        .join(", ");
    let reason = format!("Automod: {labels}");

    if let Err(error) = msg.delete(&ctx.http).await {
        tracing::warn!(
            "Automod could not delete message {} in {}: {error}",
            msg.id,
            msg.channel_id
        );
    }

    // Never persist a live credential: redact before evidence and modlog.
    let stored = redact_secrets(&msg.content);

    if let Err(error) = state.db.add_evidence(
        guild_id.get(),
        msg.channel_id.get(),
        msg.id.get(),
        msg.author.id.get(),
        &stored,
        &reason,
    ) {
        tracing::warn!("Automod could not store evidence for {}: {error}", msg.id);
    }

    state.metrics.inc_automod();

    let snippet = truncate(&escape_md(&stored), 300);
    let embed = CreateEmbed::new()
        .title("Automod action")
        .description(format!("**Rule(s):** {labels}\n**Reason:** {reason}"))
        .colour(crate::util::embeds::AMBER)
        .field(
            "Author",
            format!("<@{}> (`{}`)", msg.author.id, msg.author.id),
            true,
        )
        .field("Channel", format!("<#{}>", msg.channel_id), true)
        .field(
            "Snippet",
            if snippet.is_empty() {
                "*(empty)*".to_owned()
            } else {
                snippet
            },
            false,
        )
        .timestamp(Timestamp::now());
    crate::commands::moderation::post_modlog(state, ctx, guild_id, embed).await;

    if violations.iter().any(|violation| violation.creates_case())
        && let Err(error) = state.db.add_case(
            guild_id.get(),
            "automod",
            msg.author.id.get(),
            bot_id.get(),
            &reason,
            Some(&truncate(&stored, 500)),
            None,
        )
    {
        tracing::warn!("Automod could not record a case: {error}");
    }
}

/// Download both feeds and rebuild the phishing store. Partial success applies
/// the feed that worked; only a total failure returns an error.
pub async fn refresh_feeds(state: &SharedState) -> BotResult<()> {
    let mut domains: HashSet<String> = HashSet::new();
    let mut succeeded = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (name, url) in FEEDS {
        match fetch_text(state, url).await {
            Ok(body) => {
                let parsed = parse_feed(&body);
                if let Err(error) = state.db.set_feed_state(name, parsed.len()) {
                    tracing::warn!("Could not persist feed state for {name}: {error}");
                }
                domains.extend(parsed);
                succeeded += 1;
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }

    if succeeded == 0 {
        anyhow::bail!("all phishing feeds failed ({})", failures.join("; "));
    }

    let mut capped: Vec<String> = domains.into_iter().collect();
    capped.sort();
    capped.truncate(MAX_FEED_ENTRIES);
    let count = state.phishing.replace(capped);

    if failures.is_empty() {
        tracing::info!("Phishing feeds refreshed: {count} domain(s)");
    } else {
        tracing::warn!(
            "Partial phishing refresh ({count} domain(s)); failures: {}",
            failures.join("; ")
        );
    }
    Ok(())
}

async fn fetch_text(state: &SharedState, url: &str) -> BotResult<String> {
    let response = state
        .http
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("bad status from {url}"))?;
    Ok(response.text().await?)
}

/// Extract domains from a plain-text feed (one URL or host per line, `#` comments).
pub fn parse_feed(text: &str) -> Vec<String> {
    let mut domains: HashSet<String> = HashSet::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or(line);
        let host = if token.contains("://") {
            domain_of(token)
        } else {
            domain_of(&format!("http://{token}"))
        };
        let Some(host) = host else { continue };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
        if host.len() <= 253 && host.contains('.') && !host.contains(char::is_whitespace) {
            domains.insert(host);
        }
    }

    let mut domains: Vec<String> = domains.into_iter().collect();
    domains.sort();
    domains
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_enabled() -> AutomodSettings {
        AutomodSettings::default()
    }

    fn never_bad(_: &str) -> bool {
        false
    }

    #[test]
    fn detects_invite_links() {
        for content in [
            "join discord.gg/abc123",
            "https://discord.gg/xyz",
            "https://discord.com/invite/fun",
            "https://discordapp.com/invite/legacy",
        ] {
            assert!(
                evaluate(content, 0, false, &all_enabled(), never_bad).contains(&Violation::Invite),
                "{content}"
            );
        }
        assert!(
            !evaluate("discord.gg", 0, false, &all_enabled(), never_bad)
                .contains(&Violation::Invite)
        );
        assert!(
            !evaluate(
                "normal discord.com/channels link",
                0,
                false,
                &all_enabled(),
                never_bad
            )
            .contains(&Violation::Invite)
        );
    }

    #[test]
    fn detects_phishing_domains() {
        let bad = |domain: &str| domain == "evil.example" || domain.ends_with(".evil.example");
        assert!(
            evaluate(
                "free nitro at https://evil.example/claim",
                0,
                false,
                &all_enabled(),
                bad
            )
            .contains(&Violation::Phishing)
        );
        assert!(
            evaluate(
                "https://login.evil.example/x",
                0,
                false,
                &all_enabled(),
                bad
            )
            .contains(&Violation::Phishing)
        );
        assert!(
            !evaluate("https://good.example", 0, false, &all_enabled(), bad)
                .contains(&Violation::Phishing)
        );
    }

    #[test]
    fn detects_mass_mentions() {
        let mentions = ["<@1>"; MASS_MENTION_THRESHOLD].join(" ");
        assert!(
            evaluate(
                &mentions,
                MASS_MENTION_THRESHOLD,
                false,
                &all_enabled(),
                never_bad
            )
            .contains(&Violation::MassMention)
        );
        assert!(
            !evaluate(
                &mentions,
                MASS_MENTION_THRESHOLD - 1,
                false,
                &all_enabled(),
                never_bad
            )
            .contains(&Violation::MassMention)
        );
        assert!(
            evaluate("hi @everyone", 0, true, &all_enabled(), never_bad)
                .contains(&Violation::MassMention)
        );
    }

    #[test]
    fn caps_boundary() {
        // 8/12 uppercase = 66% → not caps.
        assert!(!is_caps("ABCDEFGHijkl"));
        // 9/12 uppercase = 75% → caps.
        assert!(is_caps("ABCDEFGHIjkl"));
        // Too few letters even at 100% uppercase.
        assert!(!is_caps("ABCDEFGHIJK"));
        assert!(!is_caps("lowercase text only"));
        assert!(!is_caps(""));
    }

    #[test]
    fn detects_token_patterns() {
        let discord = format!("M{}.{}.{}", "a".repeat(24), "b".repeat(6), "c".repeat(27));
        let samples = [
            format!("token: {discord}"),
            format!("aws AKIA{}", "A".repeat(16)),
            format!("gh ghp_{}", "z".repeat(36)),
            "-----BEGIN PRIVATE KEY-----".to_owned(),
            "-----BEGIN OPENSSH PRIVATE KEY-----".to_owned(),
            "slack xoxb-1234-abcd".to_owned(),
        ];
        for sample in samples {
            assert!(has_secret(&sample), "{sample}");
        }
        assert!(!has_secret("just a normal sentence about tokens"));
        assert!(!has_secret("AKIA not a key"));
    }

    #[test]
    fn redacts_secrets() {
        let discord = format!("M{}.{}.{}", "a".repeat(24), "b".repeat(6), "c".repeat(27));
        let message = format!("my token is {discord} oops");
        let redacted = redact_secrets(&message);
        assert!(redacted.contains("[redacted]"));
        assert!(!redacted.contains(&discord));
    }

    #[test]
    fn disabled_rules_are_skipped() {
        let settings = AutomodSettings {
            invites: false,
            phishing: false,
            mass_mentions: false,
            duplicate_spam: false,
            caps: false,
            token_leaks: false,
        };
        let bad = |_: &str| true;
        assert!(
            evaluate(
                "discord.gg/x @everyone SEE HTTPS://EVIL.EXAMPLE",
                20,
                true,
                &settings,
                bad
            )
            .is_empty()
        );
    }

    #[test]
    fn duplicate_tracker_fires_on_third_message() {
        let guild = 900_001;
        let user = 42;
        let content = "same spam";
        // Unique timestamps window.
        assert!(!note_duplicate(guild, user, content, 1_000));
        assert!(!note_duplicate(guild, user, content, 1_010));
        assert!(note_duplicate(guild, user, content, 1_020));
        // Different wording is a different key.
        assert!(!note_duplicate(guild, user, "other", 1_025));
        // After the window the counter resets.
        assert!(!note_duplicate(guild, user, content, 1_100));
    }

    #[test]
    fn duplicate_tracker_ignores_empty_content() {
        assert!(!note_duplicate(1, 2, "   ", 5));
    }

    #[test]
    fn feed_parser_normalizes_and_dedupes() {
        let text = "\
# comment line
https://WWW.Evil.Example/path?q=1
EVIL.example
http://second.example:8080/x
not a domain line !!!
";
        let domains = parse_feed(text);
        assert!(domains.contains(&"evil.example".to_owned()));
        assert!(domains.contains(&"second.example".to_owned()));
        assert_eq!(
            domains
                .iter()
                .filter(|domain| *domain == "evil.example")
                .count(),
            1
        );
        assert!(!domains.iter().any(|domain| domain.contains('/')));
    }

    #[test]
    fn feed_parser_handles_empty_input() {
        assert!(parse_feed("").is_empty());
        assert!(parse_feed("# only comments\n\n").is_empty());
    }
}
