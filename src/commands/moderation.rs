//! `/mod` — moderation suite.
//!
//! Every action re-checks permissions in code (`default_member_permissions` is
//! cosmetic), validates the target (not self, not the bot, not the owner, role
//! hierarchy where Discord enforces it), writes a case when the action is
//! case-worthy, and posts a modlog embed when `modlog_channel` is configured.
//!
//! Wire format is frozen: `/mod <subcommand>` with the exact option names used by
//! `docs/USAGE.md` and the other lanes' tests.

use std::collections::HashMap;

use anyhow::bail;
use serenity::all::{
    ButtonStyle, ChannelId, ChannelType, CommandDataOption, CommandDataOptionValue,
    CommandInteraction, CommandOptionType, ComponentInteraction, Context, CreateActionRow,
    CreateAllowedMentions, CreateButton, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, EditChannel,
    EditMember, GetMessages, GuildId, Member, Message, MessageId, PermissionOverwrite,
    PermissionOverwriteType, Permissions, Role, RoleId, Timestamp, UserId,
};

use crate::bot::{BotResult, SharedState};
use crate::db::cases::Case;
use crate::security::automod::redact_secrets;
use crate::util::text::{escape_md, truncate};
use crate::util::time::{human_date, human_duration, unix_now};
use crate::util::{embeds, opts, permissions, respond};

const MIN_TIMEOUT_SECS: i64 = 60;
const MAX_TIMEOUT_SECS: i64 = 28 * 86_400;
const MAX_PURGE: i64 = 100;
const PURGE_EVIDENCE_LIMIT: usize = 25;
const PURGE_MAX_AGE_SECS: i64 = 14 * 86_400;
const MAX_REASON: usize = 400;

// ---------------------------------------------------------------------------
// Command definitions
// ---------------------------------------------------------------------------

pub fn commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("mod")
            .description("Moderation: warn, timeout, kick, ban, purge, cases")
            .add_option(warn_command())
            .add_option(warnings_command())
            .add_option(timeout_command())
            .add_option(untimeout_command())
            .add_option(kick_command())
            .add_option(ban_command())
            .add_option(unban_command())
            .add_option(purge_command())
            .add_option(slowmode_command())
            .add_option(lock_command())
            .add_option(unlock_command())
            .add_option(case_command())
            .add_option(raidmode_command())
            .add_option(quarantine_command())
            .add_option(unquarantine_command())
            .add_option(verify_panel_command())
            .add_option(reports_command())
            .add_option(report_close_command())
            .add_option(evidence_command()),
    ]
}

fn reports_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "reports",
        "List member reports filed with /report",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "status", "Filter by status")
            .add_string_choice("Open", "open")
            .add_string_choice("Handled", "handled")
            .add_string_choice("All", "all"),
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Integer,
            "limit",
            "Maximum results (1-25)",
        )
        .min_int_value(1)
        .max_int_value(25),
    )
}

fn report_close_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "report-close",
        "Mark a member report as handled",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::Integer, "id", "Report id")
            .required(true)
            .min_int_value(1),
    )
}

fn evidence_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "evidence",
        "Show recently captured automod evidence",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Integer,
            "limit",
            "Maximum results (1-25)",
        )
        .min_int_value(1)
        .max_int_value(25),
    )
}

fn user_option(description: &str) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::User, "user", description).required(true)
}

fn optional_user_option(description: &str) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::User, "user", description)
}

fn reason_option(description: &str, required: bool) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::String, "reason", description)
        .max_length(MAX_REASON as u16)
        .required(required)
}

fn optional_channel_option(description: &str) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::Channel, "channel", description)
        .channel_types(vec![ChannelType::Text])
}

fn warn_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "warn",
        "Warn a member and record a case",
    )
    .add_sub_option(user_option("Member to warn"))
    .add_sub_option(reason_option("Reason for the warning", true))
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "evidence",
            "Optional evidence: message link or note",
        )
        .max_length(MAX_REASON as u16),
    )
}

fn warnings_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "warnings",
        "List a member's active warnings",
    )
    .add_sub_option(user_option("Member to inspect"))
}

fn timeout_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "timeout",
        "Time a member out (60s to 28d)",
    )
    .add_sub_option(user_option("Member to time out"))
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "duration",
            "Duration, e.g. 10m, 2h, 7d",
        )
        .required(true),
    )
    .add_sub_option(reason_option("Reason for the timeout", true))
}

fn untimeout_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "untimeout",
        "Clear a member's timeout",
    )
    .add_sub_option(user_option("Member to release"))
    .add_sub_option(reason_option("Reason for clearing the timeout", false))
}

fn kick_command() -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, "kick", "Kick a member")
        .add_sub_option(user_option("Member to kick"))
        .add_sub_option(reason_option("Reason for the kick", true))
}

fn ban_command() -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, "ban", "Ban a member")
        .add_sub_option(user_option("Member to ban"))
        .add_sub_option(reason_option("Reason for the ban", true))
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "delete_days",
                "Delete this many days of messages (0-7)",
            )
            .min_int_value(0)
            .max_int_value(7),
        )
}

fn unban_command() -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, "unban", "Unban a user id")
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::String, "user_id", "User id to unban")
                .required(true),
        )
        .add_sub_option(reason_option("Reason for the unban", true))
}

fn purge_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "purge",
        "Bulk delete recent messages (max 100)",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Integer,
            "count",
            "How many messages (1-100)",
        )
        .required(true)
        .min_int_value(1)
        .max_int_value(MAX_PURGE as u64),
    )
    .add_sub_option(optional_user_option(
        "Only delete messages from this member",
    ))
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "contains",
            "Only delete messages containing this text",
        )
        .max_length(200),
    )
}

fn slowmode_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "slowmode",
        "Set channel slowmode (0-21600 seconds)",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Integer,
            "seconds",
            "Seconds between messages (0 disables)",
        )
        .required(true)
        .min_int_value(0)
        .max_int_value(21_600),
    )
    .add_sub_option(optional_channel_option(
        "Channel to edit (default: current)",
    ))
}

fn lock_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "lock",
        "Deny @everyone the ability to send messages",
    )
    .add_sub_option(optional_channel_option(
        "Channel to lock (default: current)",
    ))
    .add_sub_option(reason_option("Reason for the lock", false))
}

fn unlock_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "unlock",
        "Restore @everyone send permissions",
    )
    .add_sub_option(optional_channel_option(
        "Channel to unlock (default: current)",
    ))
    .add_sub_option(reason_option("Reason for the unlock", false))
}

fn case_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommandGroup,
        "case",
        "Inspect moderation cases",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "view",
            "Show one case by number",
        )
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::Integer, "id", "Case number")
                .required(true)
                .min_int_value(1),
        ),
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "list", "List recent cases")
            .add_sub_option(optional_user_option("Filter by member"))
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "limit",
                    "How many cases to show (1-50)",
                )
                .min_int_value(1)
                .max_int_value(50),
            ),
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "close",
            "Close a case so it stops counting as active",
        )
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::Integer, "id", "Case number")
                .required(true)
                .min_int_value(1),
        ),
    )
}

fn raidmode_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "raidmode",
        "Enable or disable anti-raid quarantine",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "state",
            "Turn raid mode on or off",
        )
        .required(true)
        .add_string_choice("on", "on")
        .add_string_choice("off", "off"),
    )
}

fn quarantine_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "quarantine",
        "Move a member to the quarantine role",
    )
    .add_sub_option(user_option("Member to quarantine"))
    .add_sub_option(reason_option("Reason for the quarantine", false))
}

fn unquarantine_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "unquarantine",
        "Release a member from quarantine",
    )
    .add_sub_option(user_option("Member to release"))
    .add_sub_option(reason_option("Reason for the release", false))
}

fn verify_panel_command() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "verify-panel",
        "Post a verification button panel",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::Role,
            "role",
            "Role granted when a member verifies",
        )
        .required(true),
    )
    .add_sub_option(optional_channel_option(
        "Channel to post in (default: current)",
    ))
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    // `opts::subcommand` flattens a subcommand group to its nested subcommand, so
    // the group name (`case`) is read from the raw payload first.
    if let Some(CommandDataOption {
        name,
        value: CommandDataOptionValue::SubCommandGroup(_),
        ..
    }) = cmd.data.options.first()
    {
        return match name.as_str() {
            "case" => case(state, ctx, cmd).await,
            other => {
                respond::error(
                    ctx,
                    cmd,
                    format!("Unknown moderation subcommand `{other}`."),
                )
                .await
            }
        };
    }

    let Some((name, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown moderation subcommand.").await;
    };

    match name {
        "warn" => warn(state, ctx, cmd, options).await,
        "warnings" => warnings(state, ctx, cmd, options).await,
        "timeout" => timeout(state, ctx, cmd, options).await,
        "untimeout" => untimeout(state, ctx, cmd, options).await,
        "kick" => kick(state, ctx, cmd, options).await,
        "ban" => ban(state, ctx, cmd, options).await,
        "unban" => unban(state, ctx, cmd, options).await,
        "purge" => purge(state, ctx, cmd, options).await,
        "slowmode" => slowmode(state, ctx, cmd, options).await,
        "lock" => lock(state, ctx, cmd, options, true).await,
        "unlock" => lock(state, ctx, cmd, options, false).await,
        "raidmode" => raidmode(state, ctx, cmd, options).await,
        "quarantine" => quarantine(state, ctx, cmd, options, true).await,
        "unquarantine" => quarantine(state, ctx, cmd, options, false).await,
        "verify-panel" => verify_panel(state, ctx, cmd, options).await,
        "reports" => reports(state, ctx, cmd, options).await,
        "report-close" => report_close(state, ctx, cmd, options).await,
        "evidence" => evidence(state, ctx, cmd, options).await,
        other => {
            respond::error(
                ctx,
                cmd,
                format!("Unknown moderation subcommand `{other}`."),
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

async fn warn(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;
    validate_member_target(cmd, ctx, guild_id, target).await?;

    let reason = clean_reason(opts::req_str(options, "reason")?);
    let evidence = opts::opt_str(options, "evidence")
        .map(|raw| truncate(raw.trim(), MAX_REASON))
        .filter(|raw| !raw.is_empty());

    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        "warn",
        target.get(),
        cmd.user.id.get(),
        &reason,
        evidence.as_deref(),
        None,
    )?;
    let active = state
        .db
        .count_active_warns(guild_id.get(), target.get())
        .unwrap_or(0);

    let embed = embeds::ok(
        "Member warned",
        format!("<@{target}> received a warning.\n**Case:** #{case_number} · **Active warnings:** {active}"),
    )
    .field("Reason", truncate(&escape_md(&reason), 900), false)
    .field("Moderator", format!("<@{}>", cmd.user.id), true);
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(
        "Member warned",
        Some(case_number),
        target,
        cmd.user.id,
        &reason,
        Some(("Active warnings", active.to_string())),
    );
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn warnings(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;

    let cases = state
        .db
        .list_cases(guild_id.get(), Some(target.get()), 50)?;
    let active = cases
        .into_iter()
        .filter(|case| case.kind == "warn" && case.active)
        .take(10)
        .collect::<Vec<_>>();

    let description = if active.is_empty() {
        format!("<@{target}> has no active warnings.")
    } else {
        format!("<@{target}> — {} active warning(s).", active.len())
    };
    let mut embed = embeds::info("Active warnings", description);
    for case in &active {
        embed = embed.field(
            format!("#{} — {}", case.case_number, human_date(case.created_at)),
            format!(
                "**Moderator:** <@{}>\n**Reason:** {}",
                case.moderator_id,
                truncate(&escape_md(&case.reason), 900)
            ),
            false,
        );
    }
    respond::ephemeral(ctx, cmd, embed).await
}

async fn timeout(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    permissions::require_bot(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;
    validate_member_target(cmd, ctx, guild_id, target).await?;
    ensure_hierarchy(ctx, guild_id, target).await?;

    let seconds = opts::duration(options, "duration")?.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
    let reason = clean_reason(opts::req_str(options, "reason")?);
    let until_unix = unix_now() + seconds;
    let until = Timestamp::from_unix_timestamp(until_unix)?;

    guild_id
        .edit_member(
            &ctx.http,
            target,
            EditMember::new()
                .disable_communication_until_datetime(until)
                .audit_log_reason(&reason),
        )
        .await?;

    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        "timeout",
        target.get(),
        cmd.user.id.get(),
        &reason,
        None,
        Some(until_unix),
    )?;

    let embed = embeds::ok(
        "Member timed out",
        format!(
            "<@{target}> is timed out for **{}**.\n**Case:** #{case_number} · **Until:** {}",
            human_duration(seconds),
            human_date(until_unix)
        ),
    )
    .field("Reason", truncate(&escape_md(&reason), 900), false);
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(
        "Member timed out",
        Some(case_number),
        target,
        cmd.user.id,
        &reason,
        Some(("Until", human_date(until_unix))),
    );
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn untimeout(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    permissions::require_bot(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;
    validate_member_target(cmd, ctx, guild_id, target).await?;

    let reason = opts::opt_str(options, "reason")
        .map(clean_reason)
        .unwrap_or_else(|| "Timeout cleared".to_owned());

    guild_id
        .edit_member(
            &ctx.http,
            target,
            EditMember::new()
                .enable_communication()
                .audit_log_reason(&reason),
        )
        .await?;

    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        "timeout_end",
        target.get(),
        cmd.user.id.get(),
        &reason,
        None,
        None,
    )?;

    let embed = embeds::ok(
        "Timeout cleared",
        format!("<@{target}> can talk again.\n**Case:** #{case_number}"),
    );
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(
        "Timeout cleared",
        Some(case_number),
        target,
        cmd.user.id,
        &reason,
        None,
    );
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn kick(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::KICK_MEMBERS)?;
    permissions::require_bot(cmd, Permissions::KICK_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;
    validate_member_target(cmd, ctx, guild_id, target).await?;
    ensure_hierarchy(ctx, guild_id, target).await?;

    let reason = clean_reason(opts::req_str(options, "reason")?);
    guild_id
        .kick_with_reason(&ctx.http, target, &reason)
        .await?;

    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        "kick",
        target.get(),
        cmd.user.id.get(),
        &reason,
        None,
        None,
    )?;

    let embed = embeds::ok(
        "Member kicked",
        format!("**{target}** was kicked.\n**Case:** #{case_number}"),
    )
    .field("Reason", truncate(&escape_md(&reason), 900), false);
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(
        "Member kicked",
        Some(case_number),
        target,
        cmd.user.id,
        &reason,
        None,
    );
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn ban(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::BAN_MEMBERS)?;
    permissions::require_bot(cmd, Permissions::BAN_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;
    validate_member_target(cmd, ctx, guild_id, target).await?;
    ensure_hierarchy(ctx, guild_id, target).await?;

    let reason = clean_reason(opts::req_str(options, "reason")?);
    let delete_days = opts::opt_i64(options, "delete_days")
        .unwrap_or(0)
        .clamp(0, 7) as u8;

    guild_id
        .ban_with_reason(&ctx.http, target, delete_days, &reason)
        .await?;

    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        "ban",
        target.get(),
        cmd.user.id.get(),
        &reason,
        None,
        None,
    )?;

    let embed = embeds::ok(
        "Member banned",
        format!(
            "**{target}** was banned.\n**Case:** #{case_number} · **Message days deleted:** {delete_days}"
        ),
    )
    .field("Reason", truncate(&escape_md(&reason), 900), false);
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(
        "Member banned",
        Some(case_number),
        target,
        cmd.user.id,
        &reason,
        Some(("Deleted days", delete_days.to_string())),
    );
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn unban(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::BAN_MEMBERS)?;
    permissions::require_bot(cmd, Permissions::BAN_MEMBERS)?;
    let guild_id = guild_of(cmd)?;

    let raw = opts::req_str(options, "user_id")?.trim();
    let target = match raw.parse::<u64>() {
        Ok(id) if id != 0 => UserId::new(id),
        _ => bail!("`{raw}` is not a valid user id. Right-click a user → Copy User ID."),
    };
    validate_member_target(cmd, ctx, guild_id, target).await?;

    let reason = clean_reason(opts::req_str(options, "reason")?);
    guild_id.unban(&ctx.http, target).await?;

    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        "unban",
        target.get(),
        cmd.user.id.get(),
        &reason,
        None,
        None,
    )?;

    let embed = embeds::ok(
        "User unbanned",
        format!("**{target}** was unbanned.\n**Case:** #{case_number}"),
    )
    .field("Reason", truncate(&escape_md(&reason), 900), false);
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(
        "User unbanned",
        Some(case_number),
        target,
        cmd.user.id,
        &reason,
        None,
    );
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn purge(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_MESSAGES)?;
    permissions::require_bot(cmd, Permissions::MANAGE_MESSAGES)?;
    let guild_id = guild_of(cmd)?;

    let count = opts::opt_i64(options, "count")
        .unwrap_or(1)
        .clamp(1, MAX_PURGE);
    let author = opts::opt_user(options, "user");
    let needle = opts::opt_str(options, "contains")
        .map(|raw| raw.trim().to_ascii_lowercase())
        .filter(|raw| !raw.is_empty());

    respond::working(ctx, cmd, format!("Purging up to {count} message(s)…")).await?;

    let messages = cmd
        .channel_id
        .messages(&ctx.http, GetMessages::new().limit(count as u8))
        .await?;

    let cutoff = unix_now() - PURGE_MAX_AGE_SECS;
    let selected = messages
        .iter()
        .filter(|message| !message.pinned)
        .filter(|message| message.timestamp.unix_timestamp() >= cutoff)
        .filter(|message| author.is_none_or(|id| message.author.id == id))
        .filter(|message| {
            needle
                .as_ref()
                .is_none_or(|needle| message.content.to_ascii_lowercase().contains(needle))
        })
        .take(count as usize)
        .collect::<Vec<&Message>>();

    for message in selected.iter().take(PURGE_EVIDENCE_LIMIT) {
        let content = redact_secrets(&message.content);
        if let Err(error) = state.db.add_evidence(
            guild_id.get(),
            cmd.channel_id.get(),
            message.id.get(),
            message.author.id.get(),
            &content,
            "Purge",
        ) {
            tracing::warn!("Purge could not store evidence for {}: {error}", message.id);
        }
    }

    let ids = selected
        .iter()
        .map(|message| message.id)
        .collect::<Vec<MessageId>>();
    let mut deleted = 0usize;
    for chunk in ids.chunks(100) {
        match cmd
            .channel_id
            .delete_messages(&ctx.http, chunk.to_vec())
            .await
        {
            Ok(()) => deleted += chunk.len(),
            Err(error) => tracing::warn!("Purge batch failed in {}: {error}", cmd.channel_id),
        }
    }

    let description = if selected.is_empty() {
        "No matching messages were found in the last 14 days.".to_owned()
    } else {
        format!(
            "Deleted **{deleted}** of {} matching message(s) in <#{}>.",
            selected.len(),
            cmd.channel_id
        )
    };
    respond::edit(ctx, cmd, embeds::ok("Purge complete", description)).await?;

    if deleted > 0 {
        let embed = channel_embed(
            "Messages purged",
            cmd.user.id,
            cmd.channel_id,
            &format!("**Deleted:** {deleted}"),
        );
        post_modlog(state, ctx, guild_id, embed).await;
    }
    Ok(())
}

async fn slowmode(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_CHANNELS)?;
    permissions::require_bot(cmd, Permissions::MANAGE_CHANNELS)?;
    let guild_id = guild_of(cmd)?;

    let seconds = opts::opt_i64(options, "seconds")
        .unwrap_or(0)
        .clamp(0, 21_600);
    let channel = opts::opt_channel(options, "channel").unwrap_or(cmd.channel_id);

    channel
        .edit(
            &ctx.http,
            EditChannel::new().rate_limit_per_user(seconds as u16),
        )
        .await?;

    let description = if seconds == 0 {
        format!("<#{channel}> is back to normal speed.")
    } else {
        format!(
            "<#{channel}> now allows one message every **{}**.",
            human_duration(seconds)
        )
    };
    respond::public(ctx, cmd, embeds::ok("Slowmode updated", description)).await?;

    let embed = channel_embed(
        "Slowmode updated",
        cmd.user.id,
        channel,
        &format!("**Interval:** {}", human_duration(seconds)),
    );
    post_modlog(state, ctx, guild_id, embed).await;
    Ok(())
}

async fn lock(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
    locking: bool,
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_CHANNELS)?;
    permissions::require_bot(cmd, Permissions::MANAGE_CHANNELS)?;
    let guild_id = guild_of(cmd)?;

    let channel = opts::opt_channel(options, "channel").unwrap_or(cmd.channel_id);
    let default_reason = if locking {
        "Channel locked"
    } else {
        "Channel unlocked"
    };
    let reason = opts::opt_str(options, "reason")
        .map(clean_reason)
        .unwrap_or_else(|| default_reason.to_owned());

    let everyone = guild_id.everyone_role();
    let mut overwrites = match channel.to_channel(&ctx.http).await {
        Ok(serenity::all::Channel::Guild(channel)) => channel.permission_overwrites,
        _ => Vec::new(),
    };

    match overwrites
        .iter_mut()
        .find(|overwrite| overwrite.kind == PermissionOverwriteType::Role(everyone))
    {
        Some(overwrite) => {
            if locking {
                overwrite.deny |= Permissions::SEND_MESSAGES;
                overwrite.allow.remove(Permissions::SEND_MESSAGES);
            } else {
                overwrite.allow |= Permissions::SEND_MESSAGES;
                overwrite.deny.remove(Permissions::SEND_MESSAGES);
            }
        }
        None => overwrites.push(PermissionOverwrite {
            allow: if locking {
                Permissions::empty()
            } else {
                Permissions::SEND_MESSAGES
            },
            deny: if locking {
                Permissions::SEND_MESSAGES
            } else {
                Permissions::empty()
            },
            kind: PermissionOverwriteType::Role(everyone),
        }),
    }

    channel
        .edit(
            &ctx.http,
            EditChannel::new()
                .permissions(overwrites)
                .audit_log_reason(&reason),
        )
        .await?;

    let (title, description) = if locking {
        (
            "Channel locked",
            format!("<#{channel}> is now read-only for @everyone."),
        )
    } else {
        ("Channel unlocked", format!("<#{channel}> is open again."))
    };
    respond::public(ctx, cmd, embeds::ok(title, description)).await?;

    let embed = channel_embed(
        title,
        cmd.user.id,
        channel,
        &format!("**Reason:** {}", escape_md(&reason)),
    );
    post_modlog(state, ctx, guild_id, embed).await;
    Ok(())
}

async fn case(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;

    // `opts::subcommand` flattens a subcommand group to its nested subcommand.
    let (name, sub) =
        opts::subcommand(&cmd.data).ok_or_else(|| anyhow::anyhow!("Missing case subcommand."))?;
    match name {
        "view" => {
            let number = opts::opt_i64(sub, "id").unwrap_or(0);
            if number <= 0 {
                return respond::error(ctx, cmd, "Missing case number.").await;
            }
            match state.db.get_case(guild_id.get(), number)? {
                Some(case) => respond::ephemeral(ctx, cmd, case_embed(&case)).await,
                None => respond::error(ctx, cmd, format!("Case #{number} not found.")).await,
            }
        }
        "list" => {
            let target = opts::opt_user(sub, "user");
            let limit = opts::opt_i64(sub, "limit").unwrap_or(10).clamp(1, 50) as usize;
            let cases = state
                .db
                .list_cases(guild_id.get(), target.map(|id| id.get()), limit)?;
            if cases.is_empty() {
                return respond::ephemeral(
                    ctx,
                    cmd,
                    embeds::info("Cases", "No cases recorded yet."),
                )
                .await;
            }
            let lines = cases
                .iter()
                .map(|case| {
                    format!(
                        "**#{}** · {} · <@{}> · {} · {}",
                        case.case_number,
                        case.kind,
                        case.target_id,
                        human_date(case.created_at),
                        truncate(&escape_md(&case.reason), 80)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let description = truncate(&lines, 4_000);
            respond::ephemeral(
                ctx,
                cmd,
                embeds::info(
                    "Cases",
                    format!("Showing {} case(s).\n\n{description}", cases.len()),
                ),
            )
            .await
        }
        "close" => {
            let number = opts::opt_i64(sub, "id").unwrap_or(0);
            if number <= 0 {
                return respond::error(ctx, cmd, "Missing case number.").await;
            }
            if state.db.deactivate_case(guild_id.get(), number)? {
                respond::ephemeral(
                    ctx,
                    cmd,
                    embeds::ok("Case closed", format!("Case #{number} is now inactive.")),
                )
                .await
            } else {
                respond::error(
                    ctx,
                    cmd,
                    format!("Case #{number} not found or already inactive."),
                )
                .await
            }
        }
        other => respond::error(ctx, cmd, format!("Unknown case subcommand `{other}`.")).await,
    }
}

async fn raidmode(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_GUILD)?;
    let guild_id = guild_of(cmd)?;

    let raw = opts::req_str(options, "state")?.trim().to_ascii_lowercase();
    let enabled = match raw.as_str() {
        "on" => true,
        "off" => false,
        _ => bail!("Unknown state `{raw}`. Use `on` or `off`."),
    };

    let mut settings = state.db.get_settings(guild_id.get())?;
    settings.raid.enabled = enabled;
    state.db.save_settings(guild_id.get(), &settings)?;

    let (title, detail) = if enabled {
        (
            "Anti-raid enabled",
            format!(
                "Accounts younger than {}h will be quarantined on join.",
                settings.raid.min_account_age_hours
            ),
        )
    } else {
        (
            "Anti-raid disabled",
            "New members join normally again.".to_owned(),
        )
    };
    respond::public(
        ctx,
        cmd,
        embeds::ok(
            title,
            format!("{detail}\nThresholds and roles are managed from `/config`."),
        ),
    )
    .await?;

    post_modlog(
        state,
        ctx,
        guild_id,
        settings_embed(title, cmd.user.id, &detail),
    )
    .await;
    Ok(())
}

async fn quarantine(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
    applying: bool,
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_ROLES)?;
    permissions::require_bot(cmd, Permissions::MANAGE_ROLES)?;
    let guild_id = guild_of(cmd)?;
    let target = target_user(options, "user")?;
    validate_member_target(cmd, ctx, guild_id, target).await?;
    if applying {
        ensure_hierarchy(ctx, guild_id, target).await?;
    }

    let settings = state.db.get_settings(guild_id.get())?;
    let Some(quarantine_role) = settings.quarantine_role else {
        bail!("No quarantine role configured. Run `/config set quarantine_role <role>` first.");
    };
    let reason = opts::opt_str(options, "reason")
        .map(clean_reason)
        .unwrap_or_else(|| {
            if applying {
                "Quarantined".to_owned()
            } else {
                "Released from quarantine".to_owned()
            }
        });

    let summary = if applying {
        ctx.http
            .add_member_role(
                guild_id,
                target,
                RoleId::new(quarantine_role),
                Some(&reason),
            )
            .await?;
        if let Some(verify_role) = settings.verify_role
            && let Err(error) = ctx
                .http
                .remove_member_role(guild_id, target, RoleId::new(verify_role), Some(&reason))
                .await
        {
            tracing::warn!("Could not remove the verify role from {target}: {error}");
        }
        format!("<@{target}> was quarantined.")
    } else {
        ctx.http
            .remove_member_role(
                guild_id,
                target,
                RoleId::new(quarantine_role),
                Some(&reason),
            )
            .await?;
        if let Some(verify_role) = settings.verify_role
            && let Err(error) = ctx
                .http
                .add_member_role(guild_id, target, RoleId::new(verify_role), Some(&reason))
                .await
        {
            tracing::warn!("Could not restore the verify role to {target}: {error}");
        }
        format!("<@{target}> was released from quarantine.")
    };

    let kind = if applying {
        "quarantine"
    } else {
        "unquarantine"
    };
    let (_id, case_number) = state.db.add_case(
        guild_id.get(),
        kind,
        target.get(),
        cmd.user.id.get(),
        &reason,
        None,
        None,
    )?;

    let title = if applying {
        "Member quarantined"
    } else {
        "Member released"
    };
    let embed = embeds::ok(title, format!("{summary}\n**Case:** #{case_number}")).field(
        "Reason",
        truncate(&escape_md(&reason), 900),
        false,
    );
    respond::public(ctx, cmd, embed).await?;

    let modlog = action_embed(title, Some(case_number), target, cmd.user.id, &reason, None);
    post_modlog(state, ctx, guild_id, modlog).await;
    Ok(())
}

async fn verify_panel(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_GUILD)?;
    let guild_id = guild_of(cmd)?;

    let role = opts::opt_role(options, "role")
        .ok_or_else(|| anyhow::anyhow!("Missing required option `role`."))?;
    let channel = opts::opt_channel(options, "channel").unwrap_or(cmd.channel_id);

    let mut settings = state.db.get_settings(guild_id.get())?;
    settings.verify_role = Some(role.get());
    state.db.save_settings(guild_id.get(), &settings)?;

    let embed = embeds::info(
        "Verification",
        "Click the button below to verify and unlock the server.",
    )
    .field("Role", format!("<@&{role}>"), true);
    let row = CreateActionRow::Buttons(vec![
        CreateButton::new("verify:grant")
            .label("Verify")
            .style(ButtonStyle::Success),
    ]);
    channel
        .send_message(
            &ctx.http,
            CreateMessage::new().embed(embed).components(vec![row]),
        )
        .await?;

    respond::ephemeral(
        ctx,
        cmd,
        embeds::ok(
            "Verification panel posted",
            format!("Panel posted in <#{channel}> · role <@&{role}>."),
        ),
    )
    .await
}

// ---------------------------------------------------------------------------
// Staff inbox: /report queue + automod evidence
// ---------------------------------------------------------------------------

async fn reports(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let status = opts::opt_str(options, "status").unwrap_or("open");
    let limit = opts::opt_i64(options, "limit").unwrap_or(10).clamp(1, 25) as usize;

    let reports = state.db.list_reports(guild_id.get(), status, limit)?;
    let mut embed = embeds::info(
        "Report queue",
        if reports.is_empty() {
            format!("No `{status}` reports.")
        } else {
            format!("{} `{status}` report(s), newest first.", reports.len())
        },
    );

    for report in &reports {
        let value = format!(
            "**Reporter:** <@{}>\n**Target:** <@{}>\n**When:** {}\n**Message id:** {}\n**Reason:** {}",
            report.reporter_id,
            report.target_id,
            human_date(report.created_at),
            report
                .message_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "—".to_owned()),
            truncate(&escape_md(&report.reason), 600),
        );
        embed = embed.field(format!("#{} — {}", report.id, report.status), value, false);
    }

    respond::ephemeral(ctx, cmd, embed).await
}

async fn report_close(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let id = opts::opt_i64(options, "id").unwrap_or(0);
    if id <= 0 {
        return respond::error(ctx, cmd, "Missing report id.").await;
    }

    if state
        .db
        .resolve_report(guild_id.get(), id, cmd.user.id.get())?
    {
        respond::ephemeral(
            ctx,
            cmd,
            embeds::ok("Report handled", format!("Report #{id} is now closed.")),
        )
        .await
    } else {
        respond::error(
            ctx,
            cmd,
            format!("Report #{id} was not found or is already handled."),
        )
        .await
    }
}

async fn evidence(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MODERATE_MEMBERS)?;
    let guild_id = guild_of(cmd)?;
    let limit = opts::opt_i64(options, "limit").unwrap_or(10).clamp(1, 25) as usize;

    let rows = state.db.list_evidence(guild_id.get(), limit)?;
    let mut embed = embeds::info(
        "Automod evidence",
        if rows.is_empty() {
            "No evidence captured yet.".to_owned()
        } else {
            format!("{} most recent capture(s).", rows.len())
        },
    );

    for row in &rows {
        embed = embed.field(
            format!(
                "#{} — {} — {}",
                row.id,
                row.reason,
                human_date(row.created_at)
            ),
            format!(
                "**Author:** <@{}> · **Channel:** <#{}>\n{}",
                row.author_id,
                row.channel_id,
                truncate(&escape_md(&row.content), 700),
            ),
            false,
        );
    }

    respond::ephemeral(ctx, cmd, embed).await
}

// ---------------------------------------------------------------------------
// Component handler (`verify:grant`)
// ---------------------------------------------------------------------------

/// Returns `true` when this module handled the component.
pub async fn handle_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<bool> {
    if ci.data.custom_id != "verify:grant" {
        return Ok(false);
    }
    verify_grant(state, ctx, ci).await?;
    Ok(true)
}

async fn verify_grant(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<()> {
    let Some(guild_id) = ci.guild_id else {
        return reply_component(ci, ctx, "This button only works inside the server.").await;
    };
    if ci.member.is_none() {
        return reply_component(ci, ctx, "This button only works inside the server.").await;
    }

    let settings = state.db.get_settings(guild_id.get())?;
    let Some(verify_role) = settings.verify_role else {
        return reply_component(
            ci,
            ctx,
            "Verification is not configured yet. Ask staff to run `/mod verify-panel`.",
        )
        .await;
    };

    ctx.http
        .add_member_role(
            guild_id,
            ci.user.id,
            RoleId::new(verify_role),
            Some("Verification"),
        )
        .await?;
    if let Some(quarantine_role) = settings.quarantine_role
        && let Err(error) = ctx
            .http
            .remove_member_role(
                guild_id,
                ci.user.id,
                RoleId::new(quarantine_role),
                Some("Verification"),
            )
            .await
    {
        tracing::warn!(
            "Could not remove the quarantine role from {}: {error}",
            ci.user.id
        );
    }

    reply_component(ci, ctx, "✅ You're verified — welcome to the server!").await
}

async fn reply_component(ci: &ComponentInteraction, ctx: &Context, content: &str) -> BotResult<()> {
    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(content)
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Modlog + embed builders
// ---------------------------------------------------------------------------

/// Post an embed to the guild's modlog channel. Failures are logged, never fatal.
pub async fn post_modlog(
    state: &SharedState,
    ctx: &Context,
    guild_id: GuildId,
    embed: CreateEmbed,
) {
    let settings = match state.db.get_settings(guild_id.get()) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!("Could not load settings for modlog in {guild_id}: {error}");
            return;
        }
    };
    let Some(channel_id) = settings.modlog_channel else {
        return;
    };

    if let Err(error) = ChannelId::new(channel_id)
        .send_message(
            &ctx.http,
            CreateMessage::new()
                .embed(embed)
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await
    {
        tracing::warn!("Modlog post failed in guild {guild_id}: {error}");
    }
}

fn action_embed(
    action: &str,
    case_number: Option<i64>,
    target: UserId,
    moderator: UserId,
    reason: &str,
    extra: Option<(&str, String)>,
) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title(action)
        .description(format!(
            "**Target:** <@{target}> (`{target}`)\n\
             **Moderator:** <@{moderator}> (`{moderator}`)\n\
             **Reason:** {}",
            truncate(&escape_md(reason), 900)
        ))
        .colour(embeds::AMBER)
        .timestamp(Timestamp::now());

    if let Some(number) = case_number {
        embed = embed.field("Case", format!("#{number}"), true);
    }
    if let Some((name, value)) = extra {
        embed = embed.field(name, value, true);
    }
    embeds::footer(embed)
}

fn channel_embed(action: &str, actor: UserId, channel: ChannelId, detail: &str) -> CreateEmbed {
    embeds::footer(
        CreateEmbed::new()
            .title(action)
            .description(format!(
                "**Channel:** <#{channel}> (`{channel}`)\n**Moderator:** <@{actor}>\n{detail}"
            ))
            .colour(embeds::AMBER)
            .timestamp(Timestamp::now()),
    )
}

fn settings_embed(title: &str, actor: UserId, detail: &str) -> CreateEmbed {
    embeds::footer(
        CreateEmbed::new()
            .title(title)
            .description(format!("**Moderator:** <@{actor}>\n{detail}"))
            .colour(embeds::AMBER)
            .timestamp(Timestamp::now()),
    )
}

fn case_embed(case: &Case) -> CreateEmbed {
    let mut embed = embeds::info(
        format!("Case #{} — {}", case.case_number, case.kind),
        format!(
            "**Target:** <@{}> (`{}`)\n**Moderator:** <@{}> (`{}`)",
            case.target_id, case.target_id, case.moderator_id, case.moderator_id
        ),
    )
    .field("Reason", truncate(&escape_md(&case.reason), 900), false)
    .field("Created", human_date(case.created_at), true)
    .field(
        "Status",
        if case.active { "active" } else { "inactive" },
        true,
    );

    if let Some(expires_at) = case.expires_at {
        embed = embed.field("Expires", human_date(expires_at), true);
    }
    if let Some(evidence) = &case.evidence {
        embed = embed.field("Evidence", truncate(&escape_md(evidence), 900), false);
    }
    embed
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn guild_of(cmd: &CommandInteraction) -> BotResult<GuildId> {
    cmd.guild_id
        .ok_or_else(|| anyhow::anyhow!("This command only works in a server."))
}

fn target_user(options: &[CommandDataOption], name: &str) -> BotResult<UserId> {
    opts::opt_user(options, name)
        .ok_or_else(|| anyhow::anyhow!("Missing required option `{name}`."))
}

fn clean_reason(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "No reason provided".to_owned()
    } else {
        truncate(trimmed, MAX_REASON)
    }
}

/// Not self, not the bot, not the guild owner.
async fn validate_member_target(
    cmd: &CommandInteraction,
    ctx: &Context,
    guild_id: GuildId,
    target: UserId,
) -> BotResult<()> {
    if target == cmd.user.id {
        bail!("You cannot use this on yourself.");
    }
    if target == ctx.cache.current_user().id {
        bail!("You cannot use this on me.");
    }
    if guild_owner(ctx, guild_id).await == Some(target) {
        bail!("You cannot use this on the server owner.");
    }
    Ok(())
}

async fn guild_owner(ctx: &Context, guild_id: GuildId) -> Option<UserId> {
    if let Some(guild) = guild_id.to_guild_cached(&ctx.cache) {
        return Some(guild.owner_id);
    }
    guild_id
        .to_partial_guild(&ctx.http)
        .await
        .ok()
        .map(|guild| guild.owner_id)
}

/// Best-effort: the bot's highest role must outrank the target's.
///
/// Uses the guild cache first (no HTTP). Falls back to REST when the member cache
/// is empty (no `GUILD_MEMBERS` intent). If the target cannot be fetched (e.g. a
/// ban of a non-member) the check is skipped — Discord enforces the hierarchy.
async fn ensure_hierarchy(ctx: &Context, guild_id: GuildId, target: UserId) -> BotResult<()> {
    let bot_id = ctx.cache.current_user().id;

    let cached = guild_id.to_guild_cached(&ctx.cache).and_then(|guild| {
        let bot_top = guild
            .members
            .get(&bot_id)
            .map(|member| highest_position(&guild.roles, member));
        let target_top = guild
            .members
            .get(&target)
            .map(|member| highest_position(&guild.roles, member));
        bot_top.zip(target_top)
    });

    let positions = match cached {
        Some(positions) => Some(positions),
        None => {
            let roles = guild_id.roles(&ctx.http).await.unwrap_or_default();
            match (
                guild_id.member(&ctx.http, bot_id).await,
                guild_id.member(&ctx.http, target).await,
            ) {
                (Ok(bot), Ok(target_member)) => Some((
                    highest_position(&roles, &bot),
                    highest_position(&roles, &target_member),
                )),
                _ => None,
            }
        }
    };

    if let Some((bot_top, target_top)) = positions
        && bot_top <= target_top
    {
        bail!("My highest role must be above <@{target}>'s highest role.");
    }
    Ok(())
}

fn highest_position(roles: &HashMap<RoleId, Role>, member: &Member) -> u16 {
    member
        .roles
        .iter()
        .filter_map(|role_id| roles.get(role_id))
        .map(|role| role.position)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn command_surface_is_complete() {
        let commands = commands();
        assert_eq!(commands.len(), 1);

        let value = serde_json::to_value(&commands[0]).expect("serialize");
        assert_eq!(value["name"], "mod");
        assert_eq!(
            value["description"],
            "Moderation: warn, timeout, kick, ban, purge, cases"
        );

        let options = value["options"].as_array().expect("options");
        assert!(options.len() <= 25, "Discord allows at most 25 options");
        let names = options
            .iter()
            .filter_map(|option| option["name"].as_str())
            .collect::<Vec<_>>();

        for expected in [
            "warn",
            "warnings",
            "timeout",
            "untimeout",
            "kick",
            "ban",
            "unban",
            "purge",
            "slowmode",
            "lock",
            "unlock",
            "case",
            "raidmode",
            "quarantine",
            "unquarantine",
            "verify-panel",
        ] {
            assert!(names.contains(&expected), "missing subcommand {expected}");
        }

        let case = options
            .iter()
            .find(|option| option["name"] == "case")
            .expect("case group");
        assert_eq!(case["type"], 2, "case must be a subcommand group");
        let nested = case["options"]
            .as_array()
            .expect("nested")
            .iter()
            .filter_map(|option| option["name"].as_str())
            .collect::<Vec<_>>();
        assert!(nested.contains(&"view"));
        assert!(nested.contains(&"list"));

        for option in options {
            let description = option["description"].as_str().unwrap_or_default();
            assert!(
                !description.is_empty() && description.chars().count() <= 100,
                "bad description for {}",
                option["name"]
            );
        }
    }

    #[test]
    fn purge_count_bounds_match_discord_limits() {
        let commands = commands();
        let value = serde_json::to_value(&commands[0]).expect("serialize");
        let options = value["options"].as_array().expect("options");
        let purge = options
            .iter()
            .find(|option| option["name"] == "purge")
            .expect("purge");
        let count = purge["options"]
            .as_array()
            .expect("purge options")
            .iter()
            .find(|option| option["name"] == "count")
            .expect("count");
        assert_eq!(count["min_value"], Value::from(1));
        assert_eq!(count["max_value"], Value::from(100));
    }

    #[test]
    fn reasons_are_normalized() {
        assert_eq!(clean_reason("   "), "No reason provided");
        assert_eq!(clean_reason(" spam "), "spam");
        assert_eq!(clean_reason(&"x".repeat(500)).chars().count(), MAX_REASON);
    }

    #[test]
    fn case_embed_builds_for_all_states() {
        let case = Case {
            id: 1,
            guild_id: 1,
            case_number: 4,
            kind: "warn".to_owned(),
            target_id: 2,
            moderator_id: 3,
            reason: "spam".to_owned(),
            evidence: Some("https://example.com/msg".to_owned()),
            created_at: 0,
            expires_at: Some(10),
            active: true,
        };
        let _ = case_embed(&case);

        let inactive = Case {
            active: false,
            evidence: None,
            expires_at: None,
            ..case
        };
        let _ = case_embed(&inactive);
    }
}
