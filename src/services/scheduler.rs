//! Periodic maintenance: phishing feed, guild gauge, intel cache GC and the
//! opt-in daily CVE digest.

use std::time::Duration;

use chrono::{Timelike as _, Utc};
use serenity::all::{ChannelId, Context, CreateEmbed, CreateMessage};
use tokio::time::{MissedTickBehavior, interval};

use crate::bot::SharedState;
use crate::security;
use crate::services::nvd;
use crate::util::embeds;

const PHISHING_INTERVAL: Duration = Duration::from_secs(15 * 60);
const GUILD_INTERVAL: Duration = Duration::from_secs(60 * 60);
const INTEL_GC_INTERVAL: Duration = Duration::from_secs(10 * 60);
const DIGEST_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Run until the process exits. Spawned from `Handler::ready`.
pub async fn run(state: SharedState, ctx: Context) {
    let mut phishing = interval(PHISHING_INTERVAL);
    let mut guilds = interval(GUILD_INTERVAL);
    let mut intel_gc = interval(INTEL_GC_INTERVAL);
    let mut digest = interval(DIGEST_INTERVAL);
    for timer in [&mut phishing, &mut guilds, &mut intel_gc, &mut digest] {
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        timer.tick().await; // skip the immediate first tick
    }

    let mut last_digest_day: Option<String> = None;

    tracing::info!("Scheduler started (phishing 15m, guilds 60m, intel GC 10m, digest check 15m)");

    loop {
        tokio::select! {
            _ = phishing.tick() => {
                match security::phishing::refresh(&state).await {
                    Ok(count) => tracing::info!("Phishing feed refreshed: {count} domain(s)"),
                    Err(error) => tracing::warn!("Phishing feed refresh failed: {error}"),
                }
            }
            _ = guilds.tick() => {
                let count = ctx.cache.guilds().len() as u64;
                state.metrics.set_guilds(count);
                tracing::debug!("Guild gauge updated: {count}");
            }
            _ = intel_gc.tick() => {
                match state.db.purge_expired_intel_cache() {
                    Ok(count) => tracing::debug!("Intel cache GC removed {count} expired row(s)"),
                    Err(error) => tracing::warn!("Intel cache GC failed: {error}"),
                }
            }
            _ = digest.tick() => {
                let now = Utc::now();
                if now.hour() != 9 {
                    continue;
                }
                let day = now.format("%Y-%m-%d").to_string();
                if last_digest_day.as_deref() == Some(day.as_str()) {
                    continue;
                }
                match send_daily_digest(&state, &ctx, &day).await {
                    Ok(sent) => {
                        last_digest_day = Some(day);
                        if sent > 0 {
                            tracing::info!("Daily CVE digest sent to {sent} channel(s)");
                        }
                    }
                    Err(error) => tracing::warn!("Daily CVE digest failed: {error}"),
                }
            }
        }
    }
}

/// Fetch critical CVEs once and fan the embed out to every configured channel.
/// Returns the number of channels reached; a fetch failure is logged, not fatal.
async fn send_daily_digest(state: &SharedState, ctx: &Context, day: &str) -> anyhow::Result<usize> {
    let guild_ids = state.db.all_guild_ids()?;

    let targets: Vec<u64> = guild_ids
        .into_iter()
        .filter_map(|guild_id| {
            state
                .db
                .get_settings(guild_id)
                .ok()
                .and_then(|settings| settings.digest_channel)
        })
        .collect();
    if targets.is_empty() {
        return Ok(0);
    }

    let cves = match nvd::latest(state, Some("critical"), 5).await {
        Ok(cves) => cves,
        Err(error) => {
            tracing::warn!("Daily digest: NVD unavailable: {error}");
            return Ok(0);
        }
    };

    let embed = digest_embed(&cves, day);
    let mut sent = 0usize;
    for channel_id in targets {
        match ChannelId::new(channel_id)
            .send_message(&ctx.http, CreateMessage::new().embed(embed.clone()))
            .await
        {
            Ok(_) => sent += 1,
            Err(error) => {
                tracing::warn!("Daily digest: could not post in channel {channel_id}: {error}")
            }
        }
    }
    Ok(sent)
}

fn digest_embed(cves: &[nvd::Cve], day: &str) -> CreateEmbed {
    let mut embed = embeds::info(
        format!("🛡️ Daily critical CVE digest — {day}"),
        "The five most recent critical-severity CVEs from the NVD. Patch early, patch often.",
    );

    for cve in cves.iter().take(5) {
        let score = cve
            .score
            .map(|score| format!(" · CVSS {score:.1}"))
            .unwrap_or_default();
        embed = embed.field(
            format!("[{}](https://nvd.nist.gov/vuln/detail/{})", cve.id, cve.id),
            format!(
                "**Critical**{score}\n{}",
                crate::util::text::truncate(&cve.description, 300)
            ),
            false,
        );
    }

    embed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_embed_contains_links_and_scores() {
        let cve = nvd::Cve {
            id: "CVE-2026-0001".to_owned(),
            description: "A test vulnerability.".to_owned(),
            score: Some(9.8),
            severity: Some("CRITICAL".to_owned()),
            vector: None,
            published: "2026-01-01T00:00:00.000".to_owned(),
            last_modified: "2026-01-01T00:00:00.000".to_owned(),
            references: Vec::new(),
            weaknesses: Vec::new(),
        };
        let embed = digest_embed(std::slice::from_ref(&cve), "2026-10-30");
        let json = serde_json::to_string(&embed).expect("serializes");
        assert!(json.contains("CVE-2026-0001"));
        assert!(json.contains("9.8"));
        assert!(json.contains("2026-10-30"));
    }
}
