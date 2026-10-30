//! Anti-raid join/leave hooks.
//!
//! Only called when `features.member_intent` is enabled and the privileged
//! `GUILD_MEMBERS` intent is active (see `handler.rs`).
//!
//! Safety default: this module **never kicks or bans** on a join spike. It
//! quarantines suspicious accounts when a quarantine role is configured and posts
//! a single modlog alert per 5-minute window when more than [`JOIN_LIMIT`] members
//! join within [`JOIN_WINDOW_SECS`]. Humans stay in the loop.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serenity::all::{Context, CreateEmbed, GuildId, Member, RoleId, Timestamp, User};

use crate::bot::{BotResult, SharedState};
use crate::db::guild::RaidSettings;
use crate::util::time::unix_now;

/// Joins inside this window count toward the raid threshold.
pub const JOIN_WINDOW_SECS: i64 = 60;
/// More than this many joins in the window raises an alert.
pub const JOIN_LIMIT: usize = 10;
/// Minimum delay between two raid alerts for the same guild.
pub const ALERT_COOLDOWN_SECS: i64 = 300;

/// Pure verdict for one join.
///
/// An account is suspicious when it is younger than `min_account_age_hours`, or
/// when avatars are required and the account has none.
pub fn is_suspicious(account_age_hours: u64, has_avatar: bool, settings: &RaidSettings) -> bool {
    if !settings.enabled {
        return false;
    }
    if account_age_hours < settings.min_account_age_hours {
        return true;
    }
    settings.require_avatar && !has_avatar
}

static JOINS: LazyLock<Mutex<HashMap<u64, Vec<i64>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static LAST_ALERT: LazyLock<Mutex<HashMap<u64, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record a join and report whether a raid alert should be posted.
///
/// True at most once per [`ALERT_COOLDOWN_SECS`] per guild, and only when the
/// guild exceeds [`JOIN_LIMIT`] joins inside [`JOIN_WINDOW_SECS`].
pub fn note_join(guild_id: u64, now: i64) -> bool {
    let count = {
        let mut joins = match JOINS.lock() {
            Ok(joins) => joins,
            Err(poisoned) => poisoned.into_inner(),
        };
        if joins.len() > 10_000 {
            joins.retain(|_, window| {
                window
                    .last()
                    .is_some_and(|last| now.saturating_sub(*last) <= JOIN_WINDOW_SECS)
            });
        }
        let window = joins.entry(guild_id).or_default();
        window.retain(|timestamp| now.saturating_sub(*timestamp) <= JOIN_WINDOW_SECS);
        window.push(now);
        window.len()
    };

    if count <= JOIN_LIMIT {
        return false;
    }

    let mut last_alert = match LAST_ALERT.lock() {
        Ok(last_alert) => last_alert,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(previous) = last_alert.get(&guild_id)
        && now.saturating_sub(*previous) < ALERT_COOLDOWN_SECS
    {
        return false;
    }
    last_alert.insert(guild_id, now);
    true
}

/// Account age in whole hours (0 for brand-new accounts).
pub fn account_age_hours(created_at: i64, now: i64) -> u64 {
    (now.saturating_sub(created_at).max(0) / 3_600) as u64
}

pub async fn on_member_join(state: &SharedState, ctx: &Context, member: &Member) -> BotResult<()> {
    if member.user.bot {
        return Ok(());
    }

    let guild_id = member.guild_id;
    let settings = state.db.get_settings(guild_id.get())?;
    let now = unix_now();

    if settings.raid.enabled {
        let age = account_age_hours(member.user.created_at().unix_timestamp(), now);
        let has_avatar = member.user.avatar.is_some();

        if is_suspicious(age, has_avatar, &settings.raid) {
            quarantine_new_member(state, ctx, guild_id, member, &settings, age).await;
        }
    }

    if note_join(guild_id.get(), now) {
        let embed = CreateEmbed::new()
            .title("Possible raid in progress")
            .description(format!(
                "More than {JOIN_LIMIT} members joined within {JOIN_WINDOW_SECS}s.\n\
                 No automatic kicks were issued — review and enable `/mod raidmode on` \
                 or lock channels if needed."
            ))
            .colour(crate::util::embeds::RED)
            .timestamp(Timestamp::now());
        crate::commands::moderation::post_modlog(state, ctx, guild_id, embed).await;
        tracing::warn!("Anti-raid: join spike detected in guild {guild_id}");
    }

    Ok(())
}

async fn quarantine_new_member(
    state: &SharedState,
    ctx: &Context,
    guild_id: GuildId,
    member: &Member,
    settings: &crate::db::GuildSettings,
    age_hours: u64,
) {
    let reason = format!(
        "Anti-raid: account age {age_hours}h, avatar {}",
        if member.user.avatar.is_some() {
            "yes"
        } else {
            "no"
        }
    );

    match settings.quarantine_role {
        Some(role_id) => {
            let role = RoleId::new(role_id);
            if let Err(error) = member.add_role(&ctx.http, role).await {
                tracing::warn!(
                    "Anti-raid could not add quarantine role to {}: {error}",
                    member.user.id
                );
            }
            if let Some(verify_role) = settings.verify_role
                && let Err(error) = member
                    .remove_role(&ctx.http, RoleId::new(verify_role))
                    .await
            {
                tracing::warn!(
                    "Anti-raid could not remove verify role from {}: {error}",
                    member.user.id
                );
            }

            let embed = CreateEmbed::new()
                .title("New member quarantined")
                .description(format!(
                    "<@{}> was quarantined.\n**Reason:** {reason}",
                    member.user.id
                ))
                .colour(crate::util::embeds::AMBER)
                .timestamp(Timestamp::now());
            crate::commands::moderation::post_modlog(state, ctx, guild_id, embed).await;
        }
        None => {
            let embed = CreateEmbed::new()
                .title("Suspicious join (no quarantine role)")
                .description(format!(
                    "<@{}> looks suspicious but no quarantine role is configured.\n\
                     **Reason:** {reason}\nRun `/config set quarantine_role <role>` to enable it.",
                    member.user.id
                ))
                .colour(crate::util::embeds::AMBER)
                .timestamp(Timestamp::now());
            crate::commands::moderation::post_modlog(state, ctx, guild_id, embed).await;
        }
    }
}

pub async fn on_member_remove(_state: &SharedState, _ctx: &Context, _user: &User) -> BotResult<()> {
    // `Metrics` (ops/metrics.rs) has no member-join/leave counters and that file
    // is outside the moderation lane's write scope, so this hook only logs.
    tracing::debug!("Member left guild (anti-raid hook)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> RaidSettings {
        RaidSettings {
            enabled: true,
            min_account_age_hours: 24,
            require_avatar: false,
        }
    }

    #[test]
    fn young_accounts_are_suspicious() {
        assert!(is_suspicious(0, true, &enabled()));
        assert!(is_suspicious(23, true, &enabled()));
        assert!(!is_suspicious(24, true, &enabled()));
    }

    #[test]
    fn avatar_requirement_is_enforced() {
        let settings = RaidSettings {
            require_avatar: true,
            ..enabled()
        };
        assert!(is_suspicious(48, false, &settings));
        assert!(!is_suspicious(48, true, &settings));
    }

    #[test]
    fn disabled_raid_mode_never_flags() {
        let settings = RaidSettings {
            enabled: false,
            ..enabled()
        };
        assert!(!is_suspicious(0, false, &settings));
    }

    #[test]
    fn account_age_is_floored() {
        assert_eq!(account_age_hours(1_000, 1_000), 0);
        assert_eq!(account_age_hours(0, 3_600), 1);
        assert_eq!(account_age_hours(0, 7_200), 2);
        assert_eq!(
            account_age_hours(5_000, 1_000),
            0,
            "future clock is clamped"
        );
    }

    #[test]
    fn join_burst_alerts_once_per_cooldown() {
        let guild = 800_001;
        let base = 10_000i64;

        for index in 0..JOIN_LIMIT {
            assert!(
                !note_join(guild, base + index as i64),
                "below threshold stays silent"
            );
        }
        assert!(
            note_join(guild, base + JOIN_LIMIT as i64),
            "11th join raises one alert"
        );
        assert!(!note_join(guild, base + JOIN_LIMIT as i64 + 1), "cooldown");

        // A second burst inside the cooldown must stay silent.
        let second = base + 70;
        let mut all_silent = true;
        for index in 0..(JOIN_LIMIT + 2) {
            all_silent &= !note_join(guild, second + index as i64);
        }
        assert!(all_silent, "cooldown suppresses repeat alerts");

        // A burst after the cooldown alerts again.
        let third = base + ALERT_COOLDOWN_SECS + 100;
        let mut fired = false;
        for index in 0..(JOIN_LIMIT + 1) {
            fired |= note_join(guild, third + index as i64);
        }
        assert!(fired, "new alert after the cooldown");
    }
}
