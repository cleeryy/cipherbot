//! Flag-leak detection. Runs inside the automod pipeline, before links.
//!
//! The raw flag is never logged, stored in evidence or posted to the modlog: the
//! point is to stop the leak, not to archive the secret.

use std::sync::LazyLock;

use regex::Regex;
use serenity::all::{Context, Message, Timestamp};

use crate::bot::SharedState;
use crate::handler::HookResult;
use crate::security::automod;
use crate::util::embeds;
use crate::util::text::{escape_md, is_url, truncate};

/// Flag prefixes recognised by the watcher (case-insensitive).
const PREFIXES: &[&str] = &[
    "flag", "ctf", "htb", "picoctf", "thm", "ductf", "pwnme", "404ctf", "uiuctf", "corctf",
];

static FLAG_REGEX: LazyLock<Option<Regex>> = LazyLock::new(|| {
    let alternation = PREFIXES.join("|");
    Regex::new(&format!(r"(?i)\b(?:{alternation})\{{[^}}\s]{{4,200}}\}}")).ok()
});

/// True when the text contains something that looks like a submitted flag.
pub fn looks_like_flag(content: &str) -> bool {
    FLAG_REGEX
        .as_ref()
        .is_some_and(|regex| regex.is_match(content))
}

/// Replace flag-shaped tokens so evidence and modlogs stay secret-free.
pub fn redact_flags(content: &str) -> String {
    match FLAG_REGEX.as_ref() {
        Some(regex) => regex.replace_all(content, "[flag redacted]").into_owned(),
        None => content.to_owned(),
    }
}

/// True when the message is nothing but links (no prose to inspect).
fn is_only_link(content: &str) -> bool {
    let mut tokens = content.split_whitespace().peekable();
    if tokens.peek().is_none() {
        return false;
    }
    tokens.all(|token| {
        let candidate = token.trim_start_matches('<').trim_end_matches('>');
        is_url(candidate)
    })
}

pub async fn on_message(state: &SharedState, ctx: &Context, msg: &Message) -> HookResult {
    if msg.author.bot || msg.webhook_id.is_some() {
        return HookResult::Continue;
    }
    let Some(guild_id) = msg.guild_id else {
        return HookResult::Continue;
    };
    if is_only_link(&msg.content) || !looks_like_flag(&msg.content) {
        return HookResult::Continue;
    }

    let settings = match state.db.get_settings(guild_id.get()) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!("Flagwatch could not load settings for {guild_id}: {error}");
            return HookResult::Continue;
        }
    };

    // The CTF channel is where flags are meant to be submitted.
    if settings.ctf_channel == Some(msg.channel_id.get()) {
        return HookResult::Continue;
    }
    if automod::is_staff_checked(ctx, guild_id, msg.author.id).await {
        return HookResult::Continue;
    }

    if let Err(error) = msg.delete(&ctx.http).await {
        tracing::warn!(
            "Flagwatch could not delete message {} in {}: {error}",
            msg.id,
            msg.channel_id
        );
    }

    let evidence = redact_flags(&msg.content);
    if let Err(error) = state.db.add_evidence(
        guild_id.get(),
        msg.channel_id.get(),
        msg.id.get(),
        msg.author.id.get(),
        &evidence,
        "Possible flag leak",
    ) {
        tracing::warn!("Flagwatch could not store evidence: {error}");
    }

    state.metrics.inc_automod();

    let embed = embeds::warn(
        "Possible flag leak",
        format!(
            "A message from <@{}> in <#{}> looked like a submitted flag and was removed.\n\
             Submit flags with `/ctf solve` or in the CTF channel.",
            msg.author.id, msg.channel_id
        ),
    )
    .field("Snippet", truncate(&escape_md(&evidence), 300), false)
    .timestamp(Timestamp::now());
    crate::commands::moderation::post_modlog(state, ctx, guild_id, embed).await;

    HookResult::Stop
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_flag_formats() {
        assert!(looks_like_flag("here: flag{th1s_is_a_flag}"));
        assert!(looks_like_flag("picoCTF{abc123}"));
        assert!(looks_like_flag("HTB{r3v3rs3_m3}"));
        assert!(looks_like_flag("CTF{demo_flag}"));
        assert!(looks_like_flag("DUCTF{down_under}"));
        assert!(looks_like_flag("PWNME{exploit_me}"));
        assert!(looks_like_flag("404CTF{not_found}"));
        assert!(looks_like_flag("uiuctf{uiuc}"));
        assert!(looks_like_flag("corctf{c0rctf}"));
        assert!(looks_like_flag("THM{try_hack_me}"));
        assert!(looks_like_flag("FLAG{upper_case_prefix}"));
    }

    #[test]
    fn ignores_non_flags() {
        assert!(!looks_like_flag("flag"));
        assert!(!looks_like_flag("flags{abc}"));
        assert!(!looks_like_flag("flag{}"));
        assert!(!looks_like_flag("flag{a}"));
        assert!(!looks_like_flag("just a normal message"));
        assert!(!looks_like_flag("HashMap<String>"));
        assert!(!looks_like_flag("let x = HashMap::<String, u32>::new();"));
        assert!(!looks_like_flag("fn main() { println!(\"{}\", 1); }"));
    }

    #[test]
    fn redaction_removes_flags() {
        let redacted = redact_flags("nice try flag{super_secret_value} but no");
        assert!(!redacted.contains("super_secret_value"));
        assert!(redacted.contains("[flag redacted]"));
    }

    #[test]
    fn detects_link_only_messages() {
        assert!(is_only_link("https://example.com"));
        assert!(is_only_link("<https://example.com/a?b=c>"));
        assert!(is_only_link("https://a.example https://b.example"));
        assert!(!is_only_link("look at https://example.com"));
        assert!(!is_only_link(""));
    }

    #[test]
    fn prefix_table_is_lowercase() {
        for prefix in PREFIXES {
            assert_eq!(prefix.to_ascii_lowercase(), *prefix);
        }
    }
}
