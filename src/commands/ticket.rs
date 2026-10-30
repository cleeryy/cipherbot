//! `/ticket` — private support threads (owned by the ticket lane).
//!
//! Flow: staff post a panel with `/ticket panel`; the persistent `ticket:open`
//! button creates a private thread in that panel channel, adds the opener and
//! posts a close button. `ticket:close` and `/ticket close` share the same
//! permission rules and archive+lock the thread after flipping the DB row.
//!
//! The panel and its buttons are stateless (fixed `custom_id`s, no arguments),
//! so they keep working across restarts with no in-memory state to rebuild.

use serenity::all::{
    AutoArchiveDuration, ButtonStyle, ChannelId, ChannelType, CommandDataOption,
    CommandInteraction, CommandOptionType, ComponentInteraction, Context, CreateActionRow,
    CreateAllowedMentions, CreateButton, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, CreateThread,
    EditThread, Member, Permissions, RoleId,
};

use crate::bot::{BotResult, SharedState};
use crate::db::ticket::Ticket;
use crate::util::text::truncate;
use crate::util::time::human_date;
use crate::util::{embeds, opts, permissions, respond};

/// Discord caps embeds at 25 fields; one line per ticket also keeps the list
/// readable. `open_ticket_count` still reports the true total.
const MAX_LISTED: usize = 25;
const THREAD_NAME_MAX: usize = 90;

// ---------------------------------------------------------------------------
// Command definitions
// ---------------------------------------------------------------------------

pub fn commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("ticket")
            .description("Support tickets: post a panel, close or list tickets")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "panel",
                    "Post the public support-ticket panel",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::Channel,
                        "channel",
                        "Channel to post the panel in (default: current)",
                    )
                    .channel_types(vec![ChannelType::Text]),
                )
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::Role,
                    "role",
                    "Staff role notified for new tickets",
                )),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "close",
                "Close the ticket thread you are in",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "list",
                "List open tickets in this server",
            )),
    ]
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some((name, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown ticket subcommand.").await;
    };

    match name {
        "panel" => panel(state, ctx, cmd, options).await,
        "close" => close(state, ctx, cmd).await,
        "list" => list(state, ctx, cmd).await,
        other => respond::error(ctx, cmd, format!("Unknown ticket subcommand `{other}`.")).await,
    }
}

// ---------------------------------------------------------------------------
// /ticket panel
// ---------------------------------------------------------------------------

async fn panel(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    permissions::require(cmd, Permissions::MANAGE_GUILD)?;
    permissions::require_bot(cmd, Permissions::CREATE_PRIVATE_THREADS)?;
    let guild_id = guild_of(cmd)?;

    let channel = opts::opt_channel(options, "channel").unwrap_or(cmd.channel_id);
    let mut settings = state.db.get_settings(guild_id.get())?;
    if let Some(role) = opts::opt_role(options, "role") {
        settings.ticket_role = Some(role.get());
        state.db.save_settings(guild_id.get(), &settings)?;
    }

    let description = match settings.ticket_role {
        Some(role) => format!(
            "Need help? Click the button below to open a private ticket with <@&{role}>.\n\
             Only you and the staff can see your thread."
        ),
        None => "Need help? Click the button below to open a private ticket with the staff.\n\
                 Only you and the staff can see your thread."
            .to_owned(),
    };

    let embed = embeds::info("Support tickets", description);
    let row = CreateActionRow::Buttons(vec![
        CreateButton::new("ticket:open")
            .style(ButtonStyle::Primary)
            .label("📩 Ouvrir un ticket"),
    ]);
    channel
        .send_message(
            &ctx.http,
            CreateMessage::new()
                .embed(embed)
                .components(vec![row])
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await?;

    // The panel is a normal message: Discord stores the button row, and the
    // fixed `ticket:open` custom id needs no state, so restarts change nothing.
    respond::ephemeral(
        ctx,
        cmd,
        embeds::ok(
            "Ticket panel posted",
            format!("Panel posted in <#{channel}>. The button stays active after restarts."),
        ),
    )
    .await
}

// ---------------------------------------------------------------------------
// /ticket close
// ---------------------------------------------------------------------------

async fn close(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let guild_id = guild_of(cmd)?;
    let thread_id = cmd.channel_id;

    let Some(ticket) = state.db.get_ticket(thread_id.get())? else {
        return respond::error(
            ctx,
            cmd,
            "This channel is not a ticket thread. Run this inside the ticket.",
        )
        .await;
    };
    if ticket.guild_id != guild_id.get() {
        return respond::error(ctx, cmd, "This ticket belongs to another server.").await;
    }

    let settings = state.db.get_settings(guild_id.get())?;
    let member = cmd.member.as_deref();
    if !may_close(
        &ticket,
        cmd.user.id.get(),
        member_perms(member),
        settings.ticket_role,
        &role_ids(member),
    ) {
        return respond::error(
            ctx,
            cmd,
            "Only the ticket opener or staff can close this ticket.",
        )
        .await;
    }

    if !state.db.close_ticket(ticket.thread_id, cmd.user.id.get())? {
        return respond::error(ctx, cmd, "This ticket is already closed.").await;
    }

    // The DB write is authoritative: a failed archive must not resurrect the
    // ticket, so report it instead of rolling back.
    let archive_error = archive_thread(ctx, thread_id).await;
    respond::ephemeral(ctx, cmd, closed_embed(archive_error)).await
}

// ---------------------------------------------------------------------------
// /ticket list
// ---------------------------------------------------------------------------

async fn list(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let guild_id = guild_of(cmd)?;
    let settings = state.db.get_settings(guild_id.get())?;
    let member = cmd.member.as_deref();
    if !may_view_queue(
        member_perms(member),
        settings.ticket_role,
        &role_ids(member),
    ) {
        return respond::error(
            ctx,
            cmd,
            "You need the Moderate Members permission or the ticket role to list tickets.",
        )
        .await;
    }

    let tickets = state.db.list_open_tickets(guild_id.get(), MAX_LISTED)?;
    let total = state.db.open_ticket_count(guild_id.get())?;

    let description = if tickets.is_empty() {
        "No open tickets.".to_owned()
    } else {
        let lines = tickets
            .iter()
            .map(|ticket| {
                format!(
                    "• <#{}> — <@{}> — opened {}",
                    ticket.thread_id,
                    ticket.user_id,
                    human_date(ticket.created_at)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        truncate(
            &format!(
                "**{total}** open ticket(s) — showing the {} oldest.\n\n{lines}",
                tickets.len()
            ),
            4_000,
        )
    };

    respond::ephemeral(ctx, cmd, embeds::info("Open tickets", description)).await
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Button actions carried by `ticket:*` custom ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TicketAction {
    Open,
    Close,
}

impl TicketAction {
    /// Only the two fixed ids are accepted — no channel/user ids are ever
    /// embedded in a custom id, so a stale or forged button cannot retarget.
    fn parse(custom_id: &str) -> Option<Self> {
        match custom_id {
            "ticket:open" => Some(Self::Open),
            "ticket:close" => Some(Self::Close),
            _ => None,
        }
    }
}

/// Returns `true` when this module handled the component.
pub async fn handle_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<bool> {
    let Some(action) = TicketAction::parse(&ci.data.custom_id) else {
        return Ok(false);
    };
    match action {
        TicketAction::Open => open_ticket(state, ctx, ci).await?,
        TicketAction::Close => close_ticket(state, ctx, ci).await?,
    }
    Ok(true)
}

async fn open_ticket(
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

    let opener = ci.user.id;
    if let Some(existing) = state.db.find_open_ticket(guild_id.get(), opener.get())? {
        return reply_component(
            ci,
            ctx,
            format!(
                "You already have an open ticket: <#{}>. Please continue there.",
                existing.thread_id
            ),
        )
        .await;
    }

    let settings = state.db.get_settings(guild_id.get())?;
    let name = format!("ticket-{}", sanitize_thread_name(&ci.user.name));

    // Always create in the channel the panel lives in (`ci.channel_id`); the
    // button never carries a channel id, so it cannot be pointed elsewhere.
    let thread = ci
        .channel_id
        .create_thread(
            &ctx.http,
            CreateThread::new(name)
                .kind(ChannelType::PrivateThread)
                .invitable(true)
                .auto_archive_duration(AutoArchiveDuration::OneDay),
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                "Could not create a ticket thread in {} for {}: {error}",
                ci.channel_id,
                opener
            );
            error
        });
    let thread = match thread {
        Ok(thread) => thread,
        Err(_) => {
            return reply_component(
                ci,
                ctx,
                "I could not create your ticket thread. Please try again in a moment.",
            )
            .await;
        }
    };

    // Private threads have members, not role overwrites: staff access comes from
    // the panel channel's permissions (`MANAGE_THREADS` / `VIEW_CHANNEL`). Adding
    // every staff member individually is not viable, so only the opener is
    // added here and the intro pings the staff role.
    if let Err(error) = thread.id.add_thread_member(&ctx.http, opener).await {
        tracing::warn!(
            "Could not add {} to ticket thread {}: {error}",
            opener,
            thread.id
        );
    }

    let intro = match settings.ticket_role {
        Some(role) => format!(
            "<@{}> opened a ticket. <@&{role}> will be with you shortly.\n\n\
             Describe your issue below — screenshots and links help. \
             Close the ticket with the button when you are done.",
            opener
        ),
        None => format!(
            "<@{}> opened a ticket. Staff will be with you shortly.\n\n\
             Describe your issue below — screenshots and links help. \
             Close the ticket with the button when you are done.",
            opener
        ),
    };
    let embed = embeds::info("Support ticket", intro);
    let row = CreateActionRow::Buttons(vec![
        CreateButton::new("ticket:close")
            .style(ButtonStyle::Danger)
            .label("Close ticket"),
    ]);
    if let Err(error) = thread
        .id
        .send_message(
            &ctx.http,
            CreateMessage::new()
                .embed(embed)
                .components(vec![row])
                // Mentions are rendered but never notify: tickets must not ping.
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await
    {
        tracing::warn!(
            "Could not post the intro in ticket thread {}: {error}",
            thread.id
        );
    }

    if let Err(error) = state.db.create_ticket(
        guild_id.get(),
        ci.channel_id.get(),
        thread.id.get(),
        opener.get(),
    ) {
        tracing::warn!("Could not record ticket {}: {error}", thread.id);
    }

    // Private threads are invisible to a role unless its members have
    // MANAGE_THREADS. A short panel-channel notification (with a deliberate
    // role ping) gives staff a reliable entry point even then.
    if let Some(role) = settings.ticket_role {
        let notice = CreateMessage::new()
            .content(format!(
                "📩 New ticket for <@&{role}>: <#{}> (opened by <@{opener}>)",
                thread.id
            ))
            .allowed_mentions(CreateAllowedMentions::new().roles(vec![RoleId::new(role)]));
        if let Err(error) = ci.channel_id.send_message(&ctx.http, notice).await {
            tracing::warn!(
                "Could not announce ticket {} in panel channel: {error}",
                thread.id
            );
        }
    }

    reply_component(ci, ctx, format!("Ticket created: <#{}>", thread.id)).await
}

async fn close_ticket(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<()> {
    let Some(guild_id) = ci.guild_id else {
        return reply_component(ci, ctx, "This button only works inside the server.").await;
    };

    let Some(ticket) = state.db.get_ticket(ci.channel_id.get())? else {
        return reply_component(ci, ctx, "This channel is not a ticket thread.").await;
    };
    if ticket.guild_id != guild_id.get() {
        return reply_component(ci, ctx, "This ticket belongs to another server.").await;
    }

    let settings = state.db.get_settings(guild_id.get())?;
    let member = ci.member.as_ref();
    if !may_close(
        &ticket,
        ci.user.id.get(),
        member_perms(member),
        settings.ticket_role,
        &role_ids(member),
    ) {
        return reply_component(
            ci,
            ctx,
            "Only the ticket opener or staff can close this ticket.",
        )
        .await;
    }

    if !state.db.close_ticket(ticket.thread_id, ci.user.id.get())? {
        return reply_component(ci, ctx, "This ticket is already closed.").await;
    }

    let archive_error = archive_thread(ctx, ci.channel_id).await;
    reply_component_embed(ci, ctx, closed_embed(archive_error)).await
}

// ---------------------------------------------------------------------------
// Permission decisions (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Opener, ticket staff role, or a moderator may close a ticket.
fn may_close(
    ticket: &Ticket,
    actor_id: u64,
    actor_perms: Permissions,
    ticket_role: Option<u64>,
    actor_roles: &[u64],
) -> bool {
    actor_id == ticket.user_id
        || actor_perms.contains(Permissions::ADMINISTRATOR)
        || actor_perms.contains(Permissions::MANAGE_MESSAGES)
        || ticket_role.is_some_and(|role| actor_roles.contains(&role))
}

/// The ticket queue is staff-only: `MODERATE_MEMBERS` or the ticket role.
fn may_view_queue(actor_perms: Permissions, ticket_role: Option<u64>, actor_roles: &[u64]) -> bool {
    actor_perms.contains(Permissions::ADMINISTRATOR)
        || actor_perms.contains(Permissions::MODERATE_MEMBERS)
        || ticket_role.is_some_and(|role| actor_roles.contains(&role))
}

fn member_perms(member: Option<&Member>) -> Permissions {
    member
        .and_then(|member| member.permissions)
        .unwrap_or_else(Permissions::empty)
}

fn role_ids(member: Option<&Member>) -> Vec<u64> {
    member
        .map(|member| member.roles.iter().map(|role| role.get()).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Lowercase, `[a-z0-9-]` only, at most 90 chars; `ticket` when nothing usable
/// remains. Discord thread names must be 2-100 chars — the `ticket-` prefix
/// added by the caller keeps the final name in range.
fn sanitize_thread_name(username: &str) -> String {
    let mut out = String::with_capacity(username.len());
    for ch in username.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }

    let trimmed = out.trim_matches('-');
    let mut name: String = trimmed.chars().take(THREAD_NAME_MAX).collect();
    while name.ends_with('-') {
        name.pop();
    }
    if name.is_empty() {
        "ticket".to_owned()
    } else {
        name
    }
}

/// Archive + lock the thread. Returns the human-readable error when Discord
/// refused, so callers can still report the DB side as closed.
async fn archive_thread(ctx: &Context, thread_id: ChannelId) -> Option<String> {
    match thread_id
        .edit_thread(&ctx.http, EditThread::new().archived(true).locked(true))
        .await
    {
        Ok(_) => None,
        Err(error) => {
            tracing::warn!("Could not archive ticket thread {thread_id}: {error}");
            Some(error.to_string())
        }
    }
}

fn closed_embed(archive_error: Option<String>) -> CreateEmbed {
    match archive_error {
        None => embeds::ok("Ticket closed", "This ticket is now archived and locked."),
        Some(error) => embeds::warn(
            "Ticket closed",
            format!(
                "The ticket was marked closed, but I could not archive the thread: {}",
                truncate(&error, 300)
            ),
        ),
    }
}

fn guild_of(cmd: &CommandInteraction) -> BotResult<serenity::all::GuildId> {
    cmd.guild_id
        .ok_or_else(|| anyhow::anyhow!("This command only works in a server."))
}

async fn reply_component(
    ci: &ComponentInteraction,
    ctx: &Context,
    content: impl Into<String>,
) -> BotResult<()> {
    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(content)
                .allowed_mentions(CreateAllowedMentions::new())
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

async fn reply_component_embed(
    ci: &ComponentInteraction,
    ctx: &Context,
    embed: CreateEmbed,
) -> BotResult<()> {
    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .embed(embed)
                .allowed_mentions(CreateAllowedMentions::new())
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_ticket() -> Ticket {
        Ticket {
            thread_id: 100,
            guild_id: 10,
            channel_id: 20,
            user_id: 42,
            status: "open".to_owned(),
            created_at: 1_700_000_000,
            closed_by: None,
            closed_at: None,
        }
    }

    #[test]
    fn sanitize_produces_discord_safe_names() {
        assert_eq!(sanitize_thread_name("Alice_99"), "alice-99");
        assert_eq!(sanitize_thread_name("  Bob  "), "bob");
        assert_eq!(sanitize_thread_name("!!!"), "ticket");
        assert_eq!(sanitize_thread_name(""), "ticket");
        assert_eq!(sanitize_thread_name("a---b"), "a-b");

        let long = "A".repeat(200);
        let name = sanitize_thread_name(&long);
        assert_eq!(name.chars().count(), THREAD_NAME_MAX);

        for input in ["Héllo Wörld", "user#1234", "a b c", "ÜBER"] {
            let name = sanitize_thread_name(input);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "unsafe name {name:?} for input {input:?}"
            );
            assert!(!name.starts_with('-') && !name.ends_with('-'));
            assert!(name.chars().count() <= THREAD_NAME_MAX);
        }
    }

    #[test]
    fn custom_id_parser_accepts_only_known_actions() {
        assert_eq!(TicketAction::parse("ticket:open"), Some(TicketAction::Open));
        assert_eq!(
            TicketAction::parse("ticket:close"),
            Some(TicketAction::Close)
        );
        assert_eq!(TicketAction::parse("ticket:open:123"), None);
        assert_eq!(TicketAction::parse("ticket:delete"), None);
        assert_eq!(TicketAction::parse("verify:grant"), None);
        assert_eq!(TicketAction::parse(""), None);
    }

    #[test]
    fn opener_and_staff_can_close_but_strangers_cannot() {
        let ticket = fixture_ticket();
        let none = Permissions::empty();

        assert!(may_close(&ticket, 42, none, None, &[]));
        assert!(may_close(
            &ticket,
            1,
            Permissions::MANAGE_MESSAGES,
            None,
            &[]
        ));
        assert!(may_close(&ticket, 1, Permissions::ADMINISTRATOR, None, &[]));
        assert!(may_close(&ticket, 1, none, Some(7), &[7]));
        assert!(!may_close(&ticket, 1, none, Some(7), &[8]));
        assert!(!may_close(&ticket, 1, none, None, &[]));
        assert!(!may_close(&ticket, 1, none, Some(7), &[]));
    }

    #[test]
    fn queue_requires_moderator_or_ticket_role() {
        let none = Permissions::empty();

        assert!(may_view_queue(Permissions::MODERATE_MEMBERS, None, &[]));
        assert!(may_view_queue(Permissions::ADMINISTRATOR, None, &[]));
        assert!(may_view_queue(none, Some(7), &[7]));
        assert!(!may_view_queue(none, Some(7), &[8]));
        assert!(!may_view_queue(none, None, &[]));
        assert!(!may_view_queue(Permissions::MANAGE_MESSAGES, None, &[]));
    }
}
