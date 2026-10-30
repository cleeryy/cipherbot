//! `/ctf` — challenge management (CTF lane).
//!
//! Flags are stored as `sha256(guild_id + ":" + normalized_flag)` and never
//! echoed, logged or embedded in plaintext.

use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::{Result as AnyResult, bail};
use parking_lot::Mutex;
use serenity::all::{
    CommandDataOption, CommandInteraction, CommandOptionType, ComponentInteraction, Context,
    CreateAllowedMentions, CreateCommand, CreateCommandOption, CreateEmbed, CreateMessage,
    Permissions,
};

use crate::bot::{BotResult, SharedState};
use crate::db::ctf::{Challenge, SolveOutcome};
use crate::util::{embeds, hash, opts, permissions, respond, text, time};

/// Category choices: `(label, stored value)`.
const CATEGORIES: &[(&str, &str)] = &[
    ("Web", "web"),
    ("Pwn", "pwn"),
    ("Crypto", "crypto"),
    ("Reverse engineering", "reverse"),
    ("Forensics", "forensics"),
    ("OSINT", "osint"),
    ("Network", "network"),
    ("Cloud", "cloud"),
    ("Misc", "misc"),
];

const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 400;
const MAX_FLAG_CHARS: usize = 200;
const MAX_LISTED: usize = 20;
const SOLVE_COOLDOWN_SECS: i64 = 3;

/// Per-user solve cooldown: user id -> earliest next attempt (unix seconds).
static SOLVE_COOLDOWN: LazyLock<Mutex<HashMap<u64, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn commands() -> Vec<CreateCommand> {
    let mut category =
        CreateCommandOption::new(CommandOptionType::String, "category", "Challenge category")
            .required(true);
    for (label, value) in CATEGORIES {
        category = category.add_string_choice(*label, *value);
    }

    let create = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "create",
        "Create a new challenge and optionally announce it",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "name", "Challenge name")
            .required(true)
            .max_length(MAX_NAME_CHARS as u16),
    )
    .add_sub_option(category)
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Integer,
            "points",
            "Points awarded for solving (10-1000)",
        )
        .required(true)
        .min_int_value(10)
        .max_int_value(1_000),
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "flag",
            "Secret flag — stored hashed, never shown",
        )
        .required(true)
        .max_length(MAX_FLAG_CHARS as u16),
    )
    .add_sub_option(CreateCommandOption::new(
        CommandOptionType::Channel,
        "channel",
        "Channel to announce the challenge in",
    ))
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "description",
            "Short description of the challenge",
        )
        .max_length(MAX_DESCRIPTION_CHARS as u16),
    );

    let list = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "list",
        "List active challenges",
    );

    let info = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "info",
        "Show details for one challenge",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "name", "Challenge name")
            .required(true)
            .max_length(MAX_NAME_CHARS as u16),
    );

    let solve = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "solve",
        "Submit a flag for a challenge",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "name", "Challenge name")
            .required(true)
            .max_length(MAX_NAME_CHARS as u16),
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "flag", "The flag you found")
            .required(true)
            .max_length(MAX_FLAG_CHARS as u16),
    );

    let scoreboard = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "scoreboard",
        "Show the CTF leaderboard",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Integer,
            "limit",
            "How many places to show (1-25, default 10)",
        )
        .min_int_value(1)
        .max_int_value(25),
    );

    let delete = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "delete",
        "Delete a challenge and all its solves",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "name", "Challenge name")
            .required(true)
            .max_length(MAX_NAME_CHARS as u16),
    );

    vec![
        CreateCommand::new("ctf")
            .description("Create, solve and score CTF challenges")
            .add_option(create)
            .add_option(list)
            .add_option(info)
            .add_option(solve)
            .add_option(scoreboard)
            .add_option(delete),
    ]
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some((subcommand, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/ctf` subcommand.").await;
    };
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "`/ctf` only works in a server.").await;
    };
    let guild_id = guild_id.get();

    match subcommand {
        "create" => create(state, ctx, cmd, guild_id, options).await,
        "list" => list(state, ctx, cmd, guild_id).await,
        "info" => info(state, ctx, cmd, guild_id, options).await,
        "solve" => solve(state, ctx, cmd, guild_id, options).await,
        "scoreboard" => scoreboard(state, ctx, cmd, guild_id, options).await,
        "delete" => delete(state, ctx, cmd, guild_id, options).await,
        other => respond::error(ctx, cmd, format!("Unknown subcommand `{other}`.")).await,
    }
}

async fn create(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    if let Err(error) = require_manager(cmd) {
        return respond::error(ctx, cmd, error).await;
    }

    let raw_name = opts::req_str(options, "name")?;
    let name = match clean_name(raw_name) {
        Ok(name) => name,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    let category = opts::req_str(options, "category")?;
    if let Err(error) = validate_category(category) {
        return respond::error(ctx, cmd, error).await;
    }

    let Some(points) = opts::opt_i64(options, "points") else {
        return respond::error(ctx, cmd, "Missing required option `points`.").await;
    };
    if let Err(error) = validate_points(points) {
        return respond::error(ctx, cmd, error).await;
    }

    let raw_flag = opts::req_str(options, "flag")?;
    let flag = match normalize_flag(raw_flag) {
        Ok(flag) => flag,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };
    let flag_hash = hash_flag(guild_id, flag);

    let description = opts::opt_str(options, "description")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(description) = description
        && description.chars().count() > MAX_DESCRIPTION_CHARS
    {
        return respond::error(
            ctx,
            cmd,
            format!("The description must be {MAX_DESCRIPTION_CHARS} characters or fewer."),
        )
        .await;
    }

    let channel = opts::opt_channel(options, "channel");

    let challenge_id = match state.db.create_challenge(
        guild_id,
        &name,
        category,
        points,
        &flag_hash,
        channel.map(|id| id.get()),
        description,
        cmd.user.id.get(),
    ) {
        Ok(id) => id,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    let mut announced = channel.is_none();
    if let Some(channel_id) = channel {
        let embed = announce_embed(&name, category, points, description);
        match channel_id
            .send_message(
                &ctx.http,
                CreateMessage::new()
                    .embed(embed)
                    .allowed_mentions(CreateAllowedMentions::new()),
            )
            .await
        {
            Ok(_) => announced = true,
            Err(error) => {
                tracing::warn!(
                    "Failed to announce CTF challenge {challenge_id} in {channel_id}: {error}"
                );
            }
        }
    }

    let mut summary = format!(
        "**{}** · {} · **{} points**\nChallenge ID: `{}`",
        text::escape_md(&name),
        category,
        points,
        challenge_id
    );
    if !announced {
        summary.push_str(
            "\n\n⚠️ The challenge was created but I could not post the announcement — check my permissions in that channel.",
        );
    }

    respond::ephemeral(ctx, cmd, embeds::ok("Challenge created", summary)).await
}

async fn list(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
) -> BotResult<()> {
    let challenges = match state.db.list_challenges(guild_id) {
        Ok(challenges) => challenges,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    if challenges.is_empty() {
        return respond::public(
            ctx,
            cmd,
            embeds::info(
                "CTF challenges",
                "No challenges yet. Staff can create one with `/ctf create`.",
            ),
        )
        .await;
    }

    let mut description = format!("**{}** challenge(s) in this server.", challenges.len());
    if challenges.len() > MAX_LISTED {
        description.push_str(&format!(
            "\nShowing the {MAX_LISTED} most recent — use `/ctf info` for any challenge."
        ));
    }

    let mut embed = embeds::info("CTF challenges", description);
    for challenge in challenges.iter().take(MAX_LISTED) {
        embed = embed.field(
            format!(
                "{} — {}",
                text::escape_md(&challenge.name),
                challenge.category
            ),
            format!(
                "{} points · {} solve(s)",
                challenge.points, challenge.solved_count
            ),
            false,
        );
    }

    respond::public(ctx, cmd, embed).await
}

async fn info(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let raw_name = opts::req_str(options, "name")?;
    let name = match clean_name(raw_name) {
        Ok(name) => name,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    match state.db.get_challenge_by_name(guild_id, &name) {
        Ok(Some(challenge)) => respond::public(ctx, cmd, challenge_embed(&challenge)).await,
        Ok(None) => respond::error(ctx, cmd, format!("No challenge named `{name}`.")).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn solve(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let raw_name = opts::req_str(options, "name")?;
    let name = match clean_name(raw_name) {
        Ok(name) => name,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    let raw_flag = opts::req_str(options, "flag")?;
    let submitted = match normalize_flag(raw_flag) {
        Ok(flag) => flag,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    if let Err(error) = check_cooldown(cmd.user.id.get()) {
        return respond::error(ctx, cmd, error).await;
    }

    let challenge = match state.db.get_challenge_by_name(guild_id, &name) {
        Ok(Some(challenge)) => challenge,
        Ok(None) => {
            return respond::error(ctx, cmd, format!("No challenge named `{name}`.")).await;
        }
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    let candidate = hash_flag(guild_id, submitted);
    if !hash::constant_time_eq(&candidate, &challenge.flag_hash) {
        // Never log or echo the submitted flag.
        tracing::debug!(
            "Incorrect CTF flag for challenge {} by {}",
            challenge.id,
            cmd.user.id
        );
        return respond::ephemeral(
            ctx,
            cmd,
            embeds::warn("Incorrect flag", "That flag is not correct."),
        )
        .await;
    }

    match state
        .db
        .record_solve(challenge.id, guild_id, cmd.user.id.get(), challenge.points)
    {
        Ok(Some(SolveOutcome::FirstBlood)) => {
            award_ctf_xp(state, guild_id, cmd.user.id.get(), challenge.points);
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "🩸 First blood!",
                    format!(
                        "You were the first to solve **{}** and earned **{} points**.",
                        text::escape_md(&challenge.name),
                        challenge.points
                    ),
                ),
            )
            .await
        }
        Ok(Some(SolveOutcome::Solved)) => {
            award_ctf_xp(state, guild_id, cmd.user.id.get(), challenge.points);
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "Correct flag",
                    format!(
                        "You solved **{}** and earned **{} points**.",
                        text::escape_md(&challenge.name),
                        challenge.points
                    ),
                ),
            )
            .await
        }
        Ok(Some(SolveOutcome::AlreadySolved)) => {
            respond::ephemeral(
                ctx,
                cmd,
                embeds::info("Already solved", "You have already solved this challenge."),
            )
            .await
        }
        Ok(None) => respond::error(ctx, cmd, "That challenge no longer exists.").await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn scoreboard(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let limit = opts::opt_i64(options, "limit").unwrap_or(10).clamp(1, 25) as usize;

    let rows = match state.db.ctf_scoreboard(guild_id, limit) {
        Ok(rows) => rows,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    if rows.is_empty() {
        return respond::public(
            ctx,
            cmd,
            embeds::info(
                "CTF scoreboard",
                "No solves yet — be the first to get on the board.",
            ),
        )
        .await;
    }

    let mut description = String::new();
    for (index, (user_id, points)) in rows.iter().enumerate() {
        let medal = match index {
            0 => "🥇",
            1 => "🥈",
            2 => "🥉",
            _ => "▫️",
        };
        description.push_str(&format!("{medal} <@{user_id}> — **{points}** points\n"));
    }

    respond::public(ctx, cmd, embeds::info("CTF scoreboard", description)).await
}

async fn delete(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    if let Err(error) = require_manager(cmd) {
        return respond::error(ctx, cmd, error).await;
    }

    let raw_name = opts::req_str(options, "name")?;
    let name = match clean_name(raw_name) {
        Ok(name) => name,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    match state.db.delete_challenge(guild_id, &name) {
        Ok(true) => {
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "Challenge deleted",
                    format!(
                        "**{}** and all of its solves were deleted.",
                        text::escape_md(&name)
                    ),
                ),
            )
            .await
        }
        Ok(false) => respond::error(ctx, cmd, format!("No challenge named `{name}`.")).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

/// Reserved for the `ctf:hint:<challenge_id>` component. No CTF components yet.
pub async fn handle_component(
    _state: &SharedState,
    _ctx: &Context,
    _ci: &ComponentInteraction,
) -> BotResult<bool> {
    Ok(false)
}

/// Authoritative permission gate: MANAGE_GUILD or MANAGE_MESSAGES (admin bypass).
fn require_manager(cmd: &CommandInteraction) -> BotResult<()> {
    let granted = cmd
        .member
        .as_ref()
        .and_then(|member| member.permissions)
        .unwrap_or_else(Permissions::empty);

    if granted.contains(Permissions::ADMINISTRATOR)
        || granted.contains(Permissions::MANAGE_GUILD)
        || granted.contains(Permissions::MANAGE_MESSAGES)
    {
        return Ok(());
    }

    permissions::require(cmd, Permissions::MANAGE_GUILD)
}

/// Trim and validate a challenge name before it reaches the database.
fn clean_name(raw: &str) -> AnyResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("The challenge name cannot be empty.");
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        bail!("The challenge name must be {MAX_NAME_CHARS} characters or fewer.");
    }
    if trimmed.chars().any(char::is_control) {
        bail!("The challenge name contains invalid characters.");
    }
    Ok(trimmed.to_owned())
}

fn validate_category(category: &str) -> AnyResult<()> {
    if CATEGORIES.iter().any(|(_, value)| *value == category) {
        return Ok(());
    }
    bail!("Unknown category `{category}`.")
}

fn validate_points(points: i64) -> AnyResult<()> {
    if !(10..=1_000).contains(&points) {
        bail!("Points must be between 10 and 1000.");
    }
    Ok(())
}

/// Trim and validate a submitted flag. Callers hash the returned value.
fn normalize_flag(raw: &str) -> AnyResult<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("The flag cannot be empty.");
    }
    if trimmed.chars().count() > MAX_FLAG_CHARS {
        bail!("The flag must be {MAX_FLAG_CHARS} characters or fewer.");
    }
    Ok(trimmed)
}

/// Guild-scoped flag digest: the same flag in two guilds must not match.
fn hash_flag(guild_id: u64, normalized_flag: &str) -> String {
    hash::sha256_hex(&format!("{guild_id}:{normalized_flag}"))
}

/// 3-second per-user cooldown for `/ctf solve`.
fn check_cooldown(user_id: u64) -> AnyResult<()> {
    let now = time::unix_now();
    let mut cooldowns = SOLVE_COOLDOWN.lock();

    if let Some(ready_at) = cooldowns.get(&user_id)
        && now < *ready_at
    {
        let remaining = (*ready_at - now).max(1);
        bail!("Slow down — try again in {remaining} second(s).");
    }

    cooldowns.insert(user_id, now + SOLVE_COOLDOWN_SECS);
    if cooldowns.len() > 10_000 {
        cooldowns.retain(|_, ready_at| *ready_at > now);
    }
    Ok(())
}

/// Public challenge card. Never includes `flag_hash`.
fn challenge_embed(challenge: &Challenge) -> CreateEmbed {
    let first_blood = match challenge.first_blood_id {
        Some(user_id) => {
            let at = challenge.first_blood_at.unwrap_or(challenge.created_at);
            format!("<@{user_id}> · {}", time::human_date(at))
        }
        None => "Nobody yet".to_owned(),
    };

    let description = challenge
        .description
        .as_deref()
        .map(text::escape_md)
        .unwrap_or_else(|| "No description provided.".to_owned());

    let mut embed = embeds::info(
        format!("🚩 {}", text::escape_md(&challenge.name)),
        description,
    )
    .field("Category", &challenge.category, true)
    .field("Points", challenge.points.to_string(), true)
    .field("Solves", challenge.solved_count.to_string(), true)
    .field("First blood", first_blood, false)
    .field("Created by", format!("<@{}>", challenge.created_by), true)
    .field("Created", time::human_date(challenge.created_at), true);

    if let Some(channel_id) = challenge.channel_id {
        embed = embed.field("Channel", format!("<#{channel_id}>"), true);
    }
    embed
}

/// Announcement posted in the challenge channel. Never includes `flag_hash`.
fn announce_embed(
    name: &str,
    category: &str,
    points: i64,
    description: Option<&str>,
) -> CreateEmbed {
    let description = description
        .map(text::escape_md)
        .unwrap_or_else(|| "No description provided.".to_owned());

    embeds::ok(
        format!("🚩 New challenge: {}", text::escape_md(name)),
        format!(
            "**Category:** {category}\n**Points:** {points}\n\n{description}\n\nUse `/ctf solve` to submit your flag."
        ),
    )
}

/// Solving also feeds the community XP economy (half the challenge points).
fn award_ctf_xp(state: &SharedState, guild_id: u64, user_id: u64, points: i64) {
    let xp = (points / 2).max(1);
    if let Err(error) = state.db.add_xp(guild_id, user_id, xp) {
        tracing::debug!("Could not award CTF XP to {user_id}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_challenge(flag_hash: String) -> Challenge {
        Challenge {
            id: 7,
            name: "Heap Overflow".to_owned(),
            category: "pwn".to_owned(),
            points: 500,
            flag_hash,
            channel_id: Some(42),
            description: Some("A classic heap puzzle.".to_owned()),
            created_by: 1_234,
            created_at: 1_790_000_000,
            first_blood_id: Some(55),
            first_blood_at: Some(1_790_000_100),
            solved_count: 3,
        }
    }

    #[test]
    fn flag_hashing_is_deterministic_and_guild_scoped() {
        let first = hash_flag(1, "flag{abc}");
        assert_eq!(first, hash_flag(1, "flag{abc}"));
        assert_ne!(first, hash_flag(2, "flag{abc}"));
        assert_eq!(first.len(), 64);
        assert_eq!(
            normalize_flag("  flag{abc}\n").expect("normalized"),
            "flag{abc}"
        );
        assert!(normalize_flag("   ").is_err());
        assert!(normalize_flag(&"a".repeat(MAX_FLAG_CHARS + 1)).is_err());
    }

    #[test]
    fn challenge_names_are_sanitized() {
        assert_eq!(
            clean_name("  Heap  Overflow ").expect("valid"),
            "Heap  Overflow"
        );
        assert!(clean_name("").is_err());
        assert!(clean_name("   ").is_err());
        assert!(clean_name("line\nbreak").is_err());
        assert!(clean_name(&"n".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert!(clean_name(&"n".repeat(MAX_NAME_CHARS)).is_ok());
    }

    #[test]
    fn points_and_categories_are_validated() {
        assert!(validate_points(9).is_err());
        assert!(validate_points(10).is_ok());
        assert!(validate_points(1_000).is_ok());
        assert!(validate_points(1_001).is_err());

        assert!(validate_category("web").is_ok());
        assert!(validate_category("misc").is_ok());
        assert!(validate_category("blockchain").is_err());
    }

    #[test]
    fn embeds_never_contain_the_plaintext_flag_or_hash() {
        let flag = "flag{don't_leak_me}";
        let flag_hash = hash_flag(99, flag);
        let challenge = sample_challenge(flag_hash.clone());

        let info = serde_json::to_string(&challenge_embed(&challenge)).expect("embed serializes");
        assert!(info.contains("Heap Overflow"), "{info}");
        assert!(!info.contains(flag), "plaintext flag leaked into embed");
        assert!(!info.contains(&flag_hash), "flag hash leaked into embed");

        let announcement = serde_json::to_string(&announce_embed(
            &challenge.name,
            &challenge.category,
            challenge.points,
            challenge.description.as_deref(),
        ))
        .expect("embed serializes");
        assert!(!announcement.contains(flag));
        assert!(!announcement.contains(&flag_hash));
    }
}
