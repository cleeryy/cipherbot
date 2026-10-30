//! Community commands: `/profile`, `/leaderboard`, `/report`, `/suggest`,
//! `/config` (community lane).
//!
//! Config writes are guild-admin only and validated in code; the command
//! definitions only make the UI tidy.

use anyhow::bail;
use serenity::all::{
    ButtonStyle, ChannelId, CommandDataOption, CommandInteraction, CommandOptionType,
    ComponentInteraction, Context, CreateActionRow, CreateAllowedMentions, CreateButton,
    CreateCommand, CreateCommandOption, CreateEmbed, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, EditMessage, Permissions,
};

use crate::bot::{BotResult, SharedState};
use crate::db::GuildSettings;
use crate::db::community::Suggestion;
use crate::db::guild::AutomodSettings;
use crate::db::users::{level_for_xp, xp_for_level};
use crate::util::{embeds, opts, permissions, respond, text, time};

pub fn commands() -> Vec<CreateCommand> {
    let profile = CreateCommand::new("profile")
        .description("Show a member profile")
        .add_option(CreateCommandOption::new(
            CommandOptionType::User,
            "user",
            "Member to look up (defaults to you)",
        ));

    let leaderboard = CreateCommand::new("leaderboard")
        .description("Server leaderboards")
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "metric", "Leaderboard to show")
                .add_string_choice("XP", "xp")
                .add_string_choice("Quiz accuracy", "quiz")
                .add_string_choice("CTF points", "ctf"),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "limit",
                "Entries to show (1-25)",
            )
            .min_int_value(1)
            .max_int_value(25),
        );

    let report = CreateCommand::new("report")
        .description("Report a member or message to the staff")
        .add_option(
            CreateCommandOption::new(CommandOptionType::User, "user", "Member to report")
                .required(true),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "reason", "What happened?")
                .required(true)
                .max_length(800),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "message_id",
                "Message ID to attach (optional)",
            )
            .max_length(25),
        );

    let suggest = CreateCommand::new("suggest")
        .description("Submit a server suggestion")
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "text", "Your suggestion")
                .required(true)
                .max_length(800),
        );

    let config = CreateCommand::new("config")
        .description("View or change server configuration")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "view",
            "Show every setting",
        ))
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "modlog",
                "Set the moderation log channel",
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Channel, "channel", "Log channel")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "ctf-channel",
                "Set the CTF announcement channel",
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Channel, "channel", "CTF channel")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "automod",
                "Toggle one automod protection",
            )
            .add_sub_option(automod_feature_option())
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Boolean,
                    "enabled",
                    "Enable or disable",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "verify",
                "Set the role granted by the verification panel",
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Role, "role", "Verified role")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "set",
                "Set one configuration key",
            )
            .add_sub_option(config_key_option())
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "value", "New value")
                    .required(true)
                    .max_length(100),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "links",
                "Configure the link archive pipeline",
            )
            .add_sub_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "auto_thread",
                "Create a thread for every link",
            ))
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "ttl_hours",
                    "Hours before forwarded messages are deleted (1-720)",
                )
                .min_int_value(1)
                .max_int_value(720),
            ),
        );

    vec![profile, leaderboard, report, suggest, config]
}

fn automod_feature_option() -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::String, "feature", "Protection to toggle")
        .required(true)
        .add_string_choice("Invite links", "invites")
        .add_string_choice("Phishing links", "phishing")
        .add_string_choice("Mass mentions", "mass_mentions")
        .add_string_choice("Duplicate spam", "duplicate_spam")
        .add_string_choice("Excessive caps", "caps")
        .add_string_choice("Token leaks", "token_leaks")
}

fn config_key_option() -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::String, "key", "Configuration key")
        .required(true)
        .add_string_choice("Modlog channel", "modlog_channel")
        .add_string_choice("CTF channel", "ctf_channel")
        .add_string_choice("Digest channel", "digest_channel")
        .add_string_choice("Quarantine role", "quarantine_role")
        .add_string_choice("Verify role", "verify_role")
        .add_string_choice("Link TTL (hours)", "link_ttl_hours")
        .add_string_choice("Auto-thread links", "auto_thread")
        .add_string_choice(
            "Raid minimum account age (hours)",
            "raid_min_account_age_hours",
        )
        .add_string_choice("Raid require avatar", "raid_require_avatar")
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    match cmd.data.name.as_str() {
        "profile" => profile(state, ctx, cmd).await,
        "leaderboard" => leaderboard(state, ctx, cmd).await,
        "report" => report(state, ctx, cmd).await,
        "suggest" => suggest(state, ctx, cmd).await,
        "config" => config(state, ctx, cmd).await,
        other => respond::error(ctx, cmd, format!("Unknown command `/{other}`.")).await,
    }
}

async fn profile(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Profiles are only available in servers.").await;
    };
    let target = opts::opt_user(&cmd.data.options, "user").unwrap_or(cmd.user.id);

    state.db.ensure_user(guild_id.get(), target.get())?;
    let profile = state.db.get_profile(guild_id.get(), target.get())?;
    let warnings = state
        .db
        .count_active_warns(guild_id.get(), target.get())
        .unwrap_or(0);

    let level = level_for_xp(profile.xp);
    let floor = xp_for_level(level);
    let next = xp_for_level(level + 1);
    let span = (next - floor).max(1);
    let progress = (profile.xp - floor).clamp(0, span);
    let filled = ((progress * 10) / span).clamp(0, 10) as usize;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
    let accuracy = if profile.quiz_total > 0 {
        profile.quiz_correct * 100 / profile.quiz_total
    } else {
        0
    };
    let first_seen = if profile.first_seen > 0 {
        time::human_date(profile.first_seen)
    } else {
        "unknown".to_owned()
    };

    let description = format!(
        "<@{target}>\nLevel **{level}** · `{bar}` {progress}/{span} XP to level {next_level}",
        next_level = level + 1,
    );
    let embed = embeds::info("Member profile", description)
        .field("XP", profile.xp.to_string(), true)
        .field("Messages", profile.messages.to_string(), true)
        .field(
            "Quiz accuracy",
            format!(
                "{accuracy}% ({}/{})",
                profile.quiz_correct, profile.quiz_total
            ),
            true,
        )
        .field("Quiz streak", profile.streak.to_string(), true)
        .field("CTF points", profile.ctf_points.to_string(), true)
        .field("First seen", first_seen, true);

    // Warning counts are moderation data: self or staff only.
    let can_see_warnings = target == cmd.user.id
        || cmd
            .member
            .as_ref()
            .and_then(|member| member.permissions)
            .is_some_and(|permissions| {
                permissions.contains(Permissions::ADMINISTRATOR)
                    || permissions.contains(Permissions::MODERATE_MEMBERS)
            });
    let embed = if can_see_warnings {
        embed.field("Warnings", warnings.to_string(), true)
    } else {
        embed
    };

    respond::public(ctx, cmd, embed).await
}

async fn leaderboard(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Leaderboards are only available in servers.").await;
    };
    let metric = opts::opt_str(&cmd.data.options, "metric").unwrap_or("xp");
    let limit = opts::opt_i64(&cmd.data.options, "limit")
        .unwrap_or(10)
        .clamp(1, 25) as usize;

    let (title, lines) = match metric {
        "quiz" => {
            let rows = state.db.leaderboard_quiz(guild_id.get(), limit)?;
            let lines = rows
                .iter()
                .enumerate()
                .map(|(index, (user_id, correct, total))| {
                    let accuracy = if *total > 0 { correct * 100 / total } else { 0 };
                    rank_line(index, *user_id, &format!("{correct}/{total} ({accuracy}%)"))
                })
                .collect::<Vec<_>>();
            ("Quiz leaderboard", lines)
        }
        "ctf" => {
            let rows = state.db.ctf_scoreboard(guild_id.get(), limit)?;
            let lines = rows
                .iter()
                .enumerate()
                .map(|(index, (user_id, points))| {
                    rank_line(index, *user_id, &format!("{points} pts"))
                })
                .collect::<Vec<_>>();
            ("CTF leaderboard", lines)
        }
        _ => {
            let rows = state.db.leaderboard_xp(guild_id.get(), limit)?;
            let lines = rows
                .iter()
                .enumerate()
                .map(|(index, (user_id, xp))| rank_line(index, *user_id, &format!("{xp} XP")))
                .collect::<Vec<_>>();
            ("XP leaderboard", lines)
        }
    };

    if lines.is_empty() {
        return respond::public(
            ctx,
            cmd,
            embeds::info(
                title,
                "No entries yet — start chatting and playing the quiz!",
            ),
        )
        .await;
    }

    respond::public(ctx, cmd, embeds::info(title, lines.join("\n"))).await
}

fn rank_line(index: usize, user_id: u64, value: &str) -> String {
    match index {
        0 => format!("🥇 <@{user_id}> — {value}"),
        1 => format!("🥈 <@{user_id}> — {value}"),
        2 => format!("🥉 <@{user_id}> — {value}"),
        _ => format!("**#{}** <@{user_id}> — {value}", index + 1),
    }
}

async fn report(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Reports are only available in servers.").await;
    };
    let Some(target) = opts::opt_user(&cmd.data.options, "user") else {
        return respond::error(ctx, cmd, "Pick the member you want to report.").await;
    };
    let reason = opts::req_str(&cmd.data.options, "reason")?;
    let message_id = match opts::opt_str(&cmd.data.options, "message_id") {
        Some(raw) => match parse_snowflake(raw) {
            Some(id) => Some(id),
            None => {
                return respond::error(
                    ctx,
                    cmd,
                    "`message_id` must be a numeric Discord message ID.",
                )
                .await;
            }
        },
        None => None,
    };

    let report_id = state.db.add_report(
        guild_id.get(),
        cmd.user.id.get(),
        target.get(),
        message_id,
        &text::truncate(reason, 800),
    )?;

    let settings = state.db.get_settings(guild_id.get())?;
    if let Some(channel_id) = settings.modlog_channel {
        let mut embed = embeds::warn(
            format!("New report #{report_id}"),
            format!(
                "**Target:** <@{}>\n**Reporter:** <@{}>\n**Reason:** {}",
                target,
                cmd.user.id,
                text::escape_md(&text::truncate(reason, 800))
            ),
        );
        if let Some(message_id) = message_id {
            embed = embed.field(
                "Message",
                format!(
                    "[Jump to message](https://discord.com/channels/{}/{}/{})",
                    guild_id.get(),
                    cmd.channel_id.get(),
                    message_id
                ),
                false,
            );
        }
        if let Err(error) = ChannelId::new(channel_id)
            .send_message(
                &ctx.http,
                CreateMessage::new()
                    .embed(embed)
                    .allowed_mentions(CreateAllowedMentions::new()),
            )
            .await
        {
            tracing::warn!("Failed to send report {report_id} to modlog: {error}");
        }
    }

    respond::ephemeral(
        ctx,
        cmd,
        embeds::ok(
            "Report sent to staff",
            "Thanks — the staff team has been notified.",
        ),
    )
    .await
}

async fn suggest(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Suggestions are only available in servers.").await;
    };
    let raw = opts::req_str(&cmd.data.options, "text")?;
    let content = text::truncate(raw, 800);

    let id = state
        .db
        .add_suggestion(guild_id.get(), cmd.user.id.get(), &content)?;
    let embed = suggestion_embed_parts(id, cmd.user.id.get(), &content, 0, 0, "open");

    let message = cmd
        .channel_id
        .send_message(
            &ctx.http,
            CreateMessage::new()
                .embed(embed)
                .components(vec![vote_row(id)])
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await?;
    state.db.set_suggestion_message(id, message.id.get())?;

    respond::ephemeral(
        ctx,
        cmd,
        embeds::ok(
            "Suggestion posted",
            format!(
                "Suggestion #{id} is now open for votes in <#{}>.",
                cmd.channel_id.get()
            ),
        ),
    )
    .await
}

async fn config(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Configuration is only available in servers.").await;
    };
    permissions::require(cmd, Permissions::MANAGE_GUILD)?;

    let Some((sub, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/config` subcommand.").await;
    };

    match sub {
        "view" => {
            let settings = state.db.get_settings(guild_id.get())?;
            respond::ephemeral(
                ctx,
                cmd,
                embeds::info("Server configuration", settings_summary(&settings)),
            )
            .await
        }
        "modlog" => {
            let channel = required_channel(options, "channel")?;
            let mut settings = state.db.get_settings(guild_id.get())?;
            settings.modlog_channel = Some(channel.get());
            state.db.save_settings(guild_id.get(), &settings)?;
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "Modlog configured",
                    format!("Moderation logs will be posted in <#{}>.", channel.get()),
                ),
            )
            .await
        }
        "ctf-channel" => {
            let channel = required_channel(options, "channel")?;
            let mut settings = state.db.get_settings(guild_id.get())?;
            settings.ctf_channel = Some(channel.get());
            state.db.save_settings(guild_id.get(), &settings)?;
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "CTF channel configured",
                    format!("CTF announcements will go to <#{}>.", channel.get()),
                ),
            )
            .await
        }
        "automod" => {
            let raw = opts::req_str(options, "feature")?;
            let feature = parse_automod_feature(raw)
                .ok_or_else(|| anyhow::anyhow!("Unknown automod feature `{raw}`."))?;
            let enabled = opts::opt_bool(options, "enabled")
                .ok_or_else(|| anyhow::anyhow!("Missing required option `enabled`."))?;
            let mut settings = state.db.get_settings(guild_id.get())?;
            set_automod(&mut settings.automod, feature, enabled);
            state.db.save_settings(guild_id.get(), &settings)?;
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "Automod updated",
                    format!(
                        "**{}** is now **{}**.",
                        feature.label(),
                        if enabled { "enabled" } else { "disabled" }
                    ),
                ),
            )
            .await
        }
        "verify" => {
            let Some(role) = opts::opt_role(options, "role") else {
                return respond::error(ctx, cmd, "Pick the role to grant on verification.").await;
            };
            let mut settings = state.db.get_settings(guild_id.get())?;
            settings.verify_role = Some(role.get());
            state.db.save_settings(guild_id.get(), &settings)?;
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "Verification role set",
                    format!(
                        "Verified members will receive <@&{}>. Run `/mod verify-panel` to post the button.",
                        role.get()
                    ),
                ),
            )
            .await
        }
        "set" => {
            let key_raw = opts::req_str(options, "key")?;
            let value_raw = opts::req_str(options, "value")?;
            let key = parse_config_key(key_raw)
                .ok_or_else(|| anyhow::anyhow!("Unknown configuration key `{key_raw}`."))?;
            let mut settings = state.db.get_settings(guild_id.get())?;
            let confirmation = apply_config_value(&mut settings, key, value_raw)?;
            state.db.save_settings(guild_id.get(), &settings)?;
            respond::ephemeral(ctx, cmd, embeds::ok("Configuration updated", confirmation)).await
        }
        "links" => {
            let auto_thread = opts::opt_bool(options, "auto_thread");
            let ttl_hours = opts::opt_i64(options, "ttl_hours");
            if auto_thread.is_none() && ttl_hours.is_none() {
                return respond::error(
                    ctx,
                    cmd,
                    "Provide at least one of `auto_thread` or `ttl_hours`.",
                )
                .await;
            }

            let mut settings = state.db.get_settings(guild_id.get())?;
            if let Some(enabled) = auto_thread {
                settings.links.auto_thread = enabled;
            }
            if let Some(hours) = ttl_hours {
                if !(1..=720).contains(&hours) {
                    return respond::error(ctx, cmd, "`ttl_hours` must be between 1 and 720.")
                        .await;
                }
                settings.links.ttl_hours = hours as u64;
            }
            settings.links.links_custom = true;
            state.db.save_settings(guild_id.get(), &settings)?;
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok(
                    "Link pipeline updated",
                    format!(
                        "Auto-thread: **{}** · TTL: **{}h**",
                        if settings.links.auto_thread {
                            "on"
                        } else {
                            "off"
                        },
                        settings.links.ttl_hours
                    ),
                ),
            )
            .await
        }
        other => respond::error(ctx, cmd, format!("Unknown `/config` subcommand `{other}`.")).await,
    }
}

pub async fn handle_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<bool> {
    let Some((value, suggestion_id)) = parse_vote_custom_id(&ci.data.custom_id) else {
        component_error(ctx, ci, "This vote button is malformed.").await?;
        return Ok(true);
    };
    let Some(guild_id) = ci.guild_id else {
        component_error(ctx, ci, "Votes are only available in servers.").await?;
        return Ok(true);
    };

    if let Err(error) =
        state
            .db
            .vote_suggestion(guild_id.get(), suggestion_id, ci.user.id.get(), value)
    {
        tracing::debug!("Vote on suggestion {suggestion_id} failed: {error}");
        component_error(ctx, ci, "This suggestion no longer exists.").await?;
        return Ok(true);
    }

    ci.create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await?;

    match state.db.get_suggestion(guild_id.get(), suggestion_id) {
        Ok(Some(suggestion)) => {
            let embed = suggestion_embed(&suggestion);
            if let Err(error) = ci
                .channel_id
                .edit_message(&ctx.http, ci.message.id, EditMessage::new().embed(embed))
                .await
            {
                tracing::warn!("Failed to refresh suggestion #{suggestion_id}: {error}");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!("Failed to reload suggestion #{suggestion_id}: {error}"),
    }

    Ok(true)
}

fn suggestion_embed(suggestion: &Suggestion) -> CreateEmbed {
    suggestion_embed_parts(
        suggestion.id,
        suggestion.author_id,
        &suggestion.content,
        suggestion.upvotes,
        suggestion.downvotes,
        &suggestion.status,
    )
}

fn suggestion_embed_parts(
    id: i64,
    author_id: u64,
    content: &str,
    up: i64,
    down: i64,
    status: &str,
) -> CreateEmbed {
    embeds::info(
        format!("Suggestion #{id}"),
        format!(
            "{}\n\n*Vote with the buttons below.*",
            text::escape_md(content)
        ),
    )
    .field("Author", format!("<@{author_id}>"), true)
    .field("Upvotes", up.to_string(), true)
    .field("Downvotes", down.to_string(), true)
    .field("Status", status.to_owned(), true)
}

fn vote_row(id: i64) -> CreateActionRow {
    CreateActionRow::Buttons(vec![
        CreateButton::new(format!("vote:up:{id}"))
            .style(ButtonStyle::Success)
            .label("👍 Upvote"),
        CreateButton::new(format!("vote:down:{id}"))
            .style(ButtonStyle::Danger)
            .label("👎 Downvote"),
    ])
}

fn parse_vote_custom_id(custom_id: &str) -> Option<(i8, i64)> {
    let rest = custom_id.strip_prefix("vote:")?;
    let (direction, id) = rest.split_once(':')?;
    let value = match direction {
        "up" => 1,
        "down" => -1,
        _ => return None,
    };
    let id = id.parse::<i64>().ok().filter(|id| *id > 0)?;
    Some((value, id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigKey {
    ModlogChannel,
    CtfChannel,
    DigestChannel,
    QuarantineRole,
    VerifyRole,
    LinkTtlHours,
    AutoThread,
    RaidMinAgeHours,
    RaidRequireAvatar,
}

fn parse_config_key(raw: &str) -> Option<ConfigKey> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "modlog_channel" => Some(ConfigKey::ModlogChannel),
        "ctf_channel" => Some(ConfigKey::CtfChannel),
        "digest_channel" => Some(ConfigKey::DigestChannel),
        "quarantine_role" => Some(ConfigKey::QuarantineRole),
        "verify_role" => Some(ConfigKey::VerifyRole),
        "link_ttl_hours" => Some(ConfigKey::LinkTtlHours),
        "auto_thread" => Some(ConfigKey::AutoThread),
        "raid_min_account_age_hours" => Some(ConfigKey::RaidMinAgeHours),
        "raid_require_avatar" => Some(ConfigKey::RaidRequireAvatar),
        _ => None,
    }
}

fn parse_snowflake(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_prefix("<#")
        .or_else(|| trimmed.strip_prefix("<@&"))
        .or_else(|| trimmed.strip_prefix("<@"))
        .unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('>').unwrap_or(trimmed);
    trimmed.parse::<u64>().ok().filter(|id| *id > 0)
}

fn parse_bool_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Validate and apply one `/config set` value; returns a short confirmation.
fn apply_config_value(
    settings: &mut GuildSettings,
    key: ConfigKey,
    raw: &str,
) -> BotResult<String> {
    match key {
        ConfigKey::ModlogChannel => {
            let id =
                parse_snowflake(raw).ok_or_else(|| anyhow::anyhow!("Expected a channel ID."))?;
            settings.modlog_channel = Some(id);
            Ok(format!("modlog_channel = `{id}`"))
        }
        ConfigKey::CtfChannel => {
            let id =
                parse_snowflake(raw).ok_or_else(|| anyhow::anyhow!("Expected a channel ID."))?;
            settings.ctf_channel = Some(id);
            Ok(format!("ctf_channel = `{id}`"))
        }
        ConfigKey::DigestChannel => {
            let id =
                parse_snowflake(raw).ok_or_else(|| anyhow::anyhow!("Expected a channel ID."))?;
            settings.digest_channel = Some(id);
            Ok(format!("digest_channel = `{id}`"))
        }
        ConfigKey::QuarantineRole => {
            let id = parse_snowflake(raw).ok_or_else(|| anyhow::anyhow!("Expected a role ID."))?;
            settings.quarantine_role = Some(id);
            Ok(format!("quarantine_role = `{id}`"))
        }
        ConfigKey::VerifyRole => {
            let id = parse_snowflake(raw).ok_or_else(|| anyhow::anyhow!("Expected a role ID."))?;
            settings.verify_role = Some(id);
            Ok(format!("verify_role = `{id}`"))
        }
        ConfigKey::LinkTtlHours => {
            let hours: u64 = raw
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("`link_ttl_hours` must be an integer."))?;
            if !(1..=720).contains(&hours) {
                bail!("`link_ttl_hours` must be between 1 and 720.");
            }
            settings.links.ttl_hours = hours;
            settings.links.links_custom = true;
            Ok(format!("link_ttl_hours = `{hours}`"))
        }
        ConfigKey::AutoThread => {
            let enabled = parse_bool_value(raw)
                .ok_or_else(|| anyhow::anyhow!("`auto_thread` must be `true` or `false`."))?;
            settings.links.auto_thread = enabled;
            settings.links.links_custom = true;
            Ok(format!("auto_thread = `{enabled}`"))
        }
        ConfigKey::RaidMinAgeHours => {
            let hours: u64 = raw
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("`raid_min_account_age_hours` must be an integer."))?;
            if hours > 720 {
                bail!("`raid_min_account_age_hours` must be at most 720.");
            }
            settings.raid.min_account_age_hours = hours;
            Ok(format!("raid.min_account_age_hours = `{hours}`"))
        }
        ConfigKey::RaidRequireAvatar => {
            let enabled = parse_bool_value(raw).ok_or_else(|| {
                anyhow::anyhow!("`raid_require_avatar` must be `true` or `false`.")
            })?;
            settings.raid.require_avatar = enabled;
            Ok(format!("raid.require_avatar = `{enabled}`"))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutomodFeature {
    Invites,
    Phishing,
    MassMentions,
    DuplicateSpam,
    Caps,
    TokenLeaks,
}

impl AutomodFeature {
    fn label(self) -> &'static str {
        match self {
            AutomodFeature::Invites => "Invite links",
            AutomodFeature::Phishing => "Phishing links",
            AutomodFeature::MassMentions => "Mass mentions",
            AutomodFeature::DuplicateSpam => "Duplicate spam",
            AutomodFeature::Caps => "Excessive caps",
            AutomodFeature::TokenLeaks => "Token leaks",
        }
    }
}

fn parse_automod_feature(raw: &str) -> Option<AutomodFeature> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "invites" => Some(AutomodFeature::Invites),
        "phishing" => Some(AutomodFeature::Phishing),
        "mass_mentions" => Some(AutomodFeature::MassMentions),
        "duplicate_spam" => Some(AutomodFeature::DuplicateSpam),
        "caps" => Some(AutomodFeature::Caps),
        "token_leaks" => Some(AutomodFeature::TokenLeaks),
        _ => None,
    }
}

fn set_automod(automod: &mut AutomodSettings, feature: AutomodFeature, enabled: bool) {
    match feature {
        AutomodFeature::Invites => automod.invites = enabled,
        AutomodFeature::Phishing => automod.phishing = enabled,
        AutomodFeature::MassMentions => automod.mass_mentions = enabled,
        AutomodFeature::DuplicateSpam => automod.duplicate_spam = enabled,
        AutomodFeature::Caps => automod.caps = enabled,
        AutomodFeature::TokenLeaks => automod.token_leaks = enabled,
    }
}

fn required_channel(options: &[CommandDataOption], name: &str) -> BotResult<ChannelId> {
    opts::opt_channel(options, name)
        .ok_or_else(|| anyhow::anyhow!("Missing required option `{name}`."))
}

fn settings_summary(settings: &GuildSettings) -> String {
    fn channel(id: Option<u64>) -> String {
        id.map(|id| format!("<#{id}>"))
            .unwrap_or_else(|| "not set".to_owned())
    }
    fn role(id: Option<u64>) -> String {
        id.map(|id| format!("<@&{id}>"))
            .unwrap_or_else(|| "not set".to_owned())
    }
    fn flag(enabled: bool) -> &'static str {
        if enabled { "on" } else { "off" }
    }

    format!(
        "**Modlog channel:** {}\n\
         **CTF channel:** {}\n\
         **Digest channel:** {}\n\
         **Quarantine role:** {}\n\
         **Verify role:** {}\n\
         **Automod:** invites {}, phishing {}, mass mentions {}, duplicate spam {}, caps {}, token leaks {}\n\
         **Anti-raid:** {}, minimum account age {}h, require avatar {}\n\
         **Links:** auto-thread {}, TTL {}h",
        channel(settings.modlog_channel),
        channel(settings.ctf_channel),
        channel(settings.digest_channel),
        role(settings.quarantine_role),
        role(settings.verify_role),
        flag(settings.automod.invites),
        flag(settings.automod.phishing),
        flag(settings.automod.mass_mentions),
        flag(settings.automod.duplicate_spam),
        flag(settings.automod.caps),
        flag(settings.automod.token_leaks),
        flag(settings.raid.enabled),
        settings.raid.min_account_age_hours,
        flag(settings.raid.require_avatar),
        flag(settings.links.auto_thread),
        settings.links.ttl_hours,
    )
}

async fn component_error(ctx: &Context, ci: &ComponentInteraction, message: &str) -> BotResult<()> {
    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(message)
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_keys_are_validated() {
        assert_eq!(
            parse_config_key("modlog_channel"),
            Some(ConfigKey::ModlogChannel)
        );
        assert_eq!(
            parse_config_key("  AUTO_THREAD "),
            Some(ConfigKey::AutoThread)
        );
        assert_eq!(parse_config_key("nope"), None);

        let mut settings = GuildSettings::default();
        assert!(apply_config_value(&mut settings, ConfigKey::LinkTtlHours, "720").is_ok());
        assert_eq!(settings.links.ttl_hours, 720);
        assert!(apply_config_value(&mut settings, ConfigKey::LinkTtlHours, "0").is_err());
        assert!(apply_config_value(&mut settings, ConfigKey::LinkTtlHours, "721").is_err());
        assert!(apply_config_value(&mut settings, ConfigKey::LinkTtlHours, "abc").is_err());

        assert!(apply_config_value(&mut settings, ConfigKey::AutoThread, "false").is_ok());
        assert!(!settings.links.auto_thread);
        assert!(apply_config_value(&mut settings, ConfigKey::AutoThread, "maybe").is_err());

        assert!(apply_config_value(&mut settings, ConfigKey::ModlogChannel, "<#123>").is_ok());
        assert_eq!(settings.modlog_channel, Some(123));
        assert!(apply_config_value(&mut settings, ConfigKey::VerifyRole, "0").is_err());
        assert!(apply_config_value(&mut settings, ConfigKey::VerifyRole, "abc").is_err());
    }

    #[test]
    fn vote_ids_are_parsed() {
        assert_eq!(parse_vote_custom_id("vote:up:42"), Some((1, 42)));
        assert_eq!(parse_vote_custom_id("vote:down:7"), Some((-1, 7)));
        assert_eq!(parse_vote_custom_id("vote:sideways:7"), None);
        assert_eq!(parse_vote_custom_id("vote:up:0"), None);
        assert_eq!(parse_vote_custom_id("vote:up:abc"), None);
        assert_eq!(parse_vote_custom_id("quiz:up:1"), None);
    }

    #[test]
    fn automod_features_map_to_settings() {
        assert_eq!(
            parse_automod_feature("mass_mentions"),
            Some(AutomodFeature::MassMentions)
        );
        assert_eq!(parse_automod_feature("none"), None);

        let mut automod = AutomodSettings::default();
        set_automod(&mut automod, AutomodFeature::Caps, false);
        assert!(!automod.caps);
        assert!(automod.phishing);
    }

    #[test]
    fn settings_summary_mentions_everything() {
        let summary = settings_summary(&GuildSettings::default());
        assert!(summary.contains("Modlog channel"));
        assert!(summary.contains("TTL 24h"));
        assert!(summary.contains("not set"));
    }

    #[test]
    fn community_commands_are_registered() {
        let names: Vec<String> = commands()
            .iter()
            .map(|command| {
                serde_json::to_value(command).expect("serialize")["name"]
                    .as_str()
                    .expect("name")
                    .to_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec!["profile", "leaderboard", "report", "suggest", "config"]
        );
    }
}
