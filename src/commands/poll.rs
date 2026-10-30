//! `/poll` — button-based polls with live result bars.
//!
//! A poll is a public channel message: one row of vote buttons plus a close
//! button. Every component interaction edits that message in place, so the
//! result bars stay live without any extra state.

use serenity::all::{
    ButtonStyle, ChannelId, CommandDataOption, CommandInteraction, CommandOptionType,
    ComponentInteraction, Context, CreateActionRow, CreateAllowedMentions, CreateButton,
    CreateCommand, CreateCommandOption, CreateEmbed, CreateEmbedFooter, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, EditMessage, Member, MessageId, Permissions,
};

use crate::bot::{BotResult, SharedState};
use crate::db::poll::{MAX_OPTION_CHARS, MAX_QUESTION_CHARS, Poll, PollVoteOutcome};
use crate::util::{embeds, opts, respond, text};

const NUMBER_EMOJIS: [&str; 5] = ["1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣"];
const CLOSE_LABEL: &str = "🔒 Clôturer";
const OPTION_NAMES: [&str; 5] = ["option1", "option2", "option3", "option4", "option5"];

pub fn commands() -> Vec<CreateCommand> {
    let create = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "create",
        "Post a poll with live vote buttons",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "question", "Poll question")
            .required(true)
            .max_length(MAX_QUESTION_CHARS as u16),
    )
    .add_sub_option(option_option("option1", "First option", true))
    .add_sub_option(option_option("option2", "Second option", true))
    .add_sub_option(option_option("option3", "Third option (optional)", false))
    .add_sub_option(option_option("option4", "Fourth option (optional)", false))
    .add_sub_option(option_option("option5", "Fifth option (optional)", false));

    let close = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "close",
        "Close your poll (author or staff)",
    )
    .add_sub_option(poll_id_option());

    let results = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "results",
        "Show the current results",
    )
    .add_sub_option(poll_id_option());

    vec![
        CreateCommand::new("poll")
            .description("Create, close and inspect button polls")
            .add_option(create)
            .add_option(close)
            .add_option(results),
    ]
}

fn option_option(
    name: &'static str,
    description: &'static str,
    required: bool,
) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::String, name, description)
        .required(required)
        .max_length(MAX_OPTION_CHARS as u16)
}

fn poll_id_option() -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::Integer, "id", "Poll number")
        .required(true)
        .min_int_value(1)
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    match cmd.data.name.as_str() {
        "poll" => route(state, ctx, cmd).await,
        other => respond::error(ctx, cmd, format!("Unknown command `/{other}`.")).await,
    }
}

async fn route(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some((sub, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/poll` subcommand.").await;
    };

    match sub {
        "create" => create(state, ctx, cmd, options).await,
        "close" => close(state, ctx, cmd, options).await,
        "results" => results(state, ctx, cmd, options).await,
        other => respond::error(ctx, cmd, format!("Unknown `/poll` subcommand `{other}`.")).await,
    }
}

async fn create(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Polls are only available in servers.").await;
    };

    let question = opts::req_str(options, "question")?.trim();
    if question.is_empty() {
        return respond::error(ctx, cmd, "The poll question must not be empty.").await;
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return respond::error(
            ctx,
            cmd,
            format!("The question must be at most {MAX_QUESTION_CHARS} characters."),
        )
        .await;
    }

    let mut choices: Vec<String> = Vec::new();
    for name in OPTION_NAMES {
        if let Some(raw) = opts::opt_str(options, name) {
            let choice = raw.trim();
            if !choice.is_empty() {
                choices.push(choice.to_owned());
            }
        }
    }

    if choices.len() < 2 {
        return respond::error(ctx, cmd, "A poll needs at least two distinct options.").await;
    }
    if has_duplicates(&choices) {
        return respond::error(ctx, cmd, "Poll options must be distinct.").await;
    }

    let poll_id = state.db.create_poll(
        guild_id.get(),
        cmd.channel_id.get(),
        cmd.user.id.get(),
        question,
        &choices,
    )?;
    let poll = load_poll_or_error(state, poll_id).await?;

    let counts = vec![0i64; poll.options.len()];
    let embed = poll_embed(&poll, &counts, 0, false);
    let message = cmd
        .channel_id
        .send_message(
            &ctx.http,
            CreateMessage::new()
                .embed(embed)
                .components(poll_components(poll.id, &poll.options, false))
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await?;
    state.db.set_poll_message(poll_id, message.id.get())?;

    let link = jump_link(guild_id.get(), cmd.channel_id.get(), message.id.get());
    respond::ephemeral(
        ctx,
        cmd,
        embeds::ok(
            "Poll created",
            format!(
                "Poll #{poll_id} is live in <#{}>.\n[Jump to the poll]({link})",
                cmd.channel_id.get()
            ),
        ),
    )
    .await
}

async fn close(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Polls are only available in servers.").await;
    };
    let Some(poll_id) = opts::opt_i64(options, "id") else {
        return respond::error(ctx, cmd, "Missing required option `id`.").await;
    };

    let Some(poll) = find_poll(state, ctx, cmd, guild_id.get(), poll_id).await? else {
        return Ok(());
    };

    let granted = invoker_permissions(cmd.member.as_deref());
    if !can_manage_poll(poll.author_id, cmd.user.id.get(), granted) {
        return respond::error(
            ctx,
            cmd,
            "Only the poll author or a member with `Manage Messages` can close this poll.",
        )
        .await;
    }

    let changed = state.db.close_poll(poll_id)?;
    refresh_poll_message(state, ctx, poll_id).await;

    let status = if changed {
        format!("Poll #{poll_id} is now closed.")
    } else {
        format!("Poll #{poll_id} was already closed.")
    };
    respond::ephemeral(ctx, cmd, embeds::ok("Poll closed", status)).await
}

async fn results(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "Polls are only available in servers.").await;
    };
    let Some(poll_id) = opts::opt_i64(options, "id") else {
        return respond::error(ctx, cmd, "Missing required option `id`.").await;
    };

    let Some(poll) = find_poll(state, ctx, cmd, guild_id.get(), poll_id).await? else {
        return Ok(());
    };

    let counts = state.db.poll_counts(poll_id)?;
    let total = state.db.total_votes(poll_id)?;
    respond::public(
        ctx,
        cmd,
        poll_embed(&poll, &counts, total, poll.closed_at.is_some()),
    )
    .await
}

/// Fetch a poll and enforce guild scoping. Responds ephemerally and returns
/// `None` when the poll is missing or belongs to another guild.
async fn find_poll(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    poll_id: i64,
) -> BotResult<Option<Poll>> {
    match state.db.get_poll(poll_id) {
        Ok(Some(poll)) if poll.guild_id == guild_id => Ok(Some(poll)),
        Ok(_) => {
            respond::error(
                ctx,
                cmd,
                format!("Poll #{poll_id} was not found in this server."),
            )
            .await?;
            Ok(None)
        }
        Err(error) => {
            tracing::warn!("Failed to load poll #{poll_id}: {error}");
            respond::error(ctx, cmd, format!("Poll #{poll_id} could not be loaded.")).await?;
            Ok(None)
        }
    }
}

async fn load_poll_or_error(state: &SharedState, poll_id: i64) -> BotResult<Poll> {
    state
        .db
        .get_poll(poll_id)?
        .ok_or_else(|| anyhow::anyhow!("poll #{poll_id} vanished right after creation"))
}

/// Rebuild and edit the poll message after a `/poll close` command.
async fn refresh_poll_message(state: &SharedState, ctx: &Context, poll_id: i64) {
    let Ok(Some(poll)) = state.db.get_poll(poll_id) else {
        return;
    };
    let Some(message_id) = poll.message_id else {
        return;
    };
    let counts = state
        .db
        .poll_counts(poll_id)
        .unwrap_or_else(|_| vec![0; poll.options.len()]);
    let total = state.db.total_votes(poll_id).unwrap_or(0);
    let embed = poll_embed(&poll, &counts, total, poll.closed_at.is_some());

    if let Err(error) = ChannelId::new(poll.channel_id)
        .edit_message(
            &ctx.http,
            MessageId::new(message_id),
            EditMessage::new()
                .embed(embed)
                .components(Vec::new())
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await
    {
        tracing::warn!("Failed to refresh poll #{poll_id}: {error}");
    }
}

pub async fn handle_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<bool> {
    let Some(action) = parse_custom_id(&ci.data.custom_id) else {
        component_error(ctx, ci, "This poll button is malformed.").await?;
        return Ok(true);
    };

    match action {
        PollAction::Vote { poll_id, choice } => {
            vote_component(state, ctx, ci, poll_id, choice).await?;
        }
        PollAction::Close { poll_id } => {
            close_component(state, ctx, ci, poll_id).await?;
        }
    }

    Ok(true)
}

async fn vote_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
    poll_id: i64,
    choice: usize,
) -> BotResult<()> {
    let Some(poll) = load_component_poll(state, ctx, ci, poll_id).await? else {
        return Ok(());
    };
    if poll.closed_at.is_some() {
        return component_error(ctx, ci, "Sondage terminé.").await;
    }

    let outcome = match state.db.vote_poll(poll_id, ci.user.id.get(), choice) {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::debug!("Vote on poll #{poll_id} failed: {error}");
            return component_error(ctx, ci, "This poll no longer exists.").await;
        }
    };

    match outcome {
        PollVoteOutcome::Closed => return component_error(ctx, ci, "Sondage terminé.").await,
        PollVoteOutcome::InvalidChoice => {
            return component_error(ctx, ci, "This poll option does not exist.").await;
        }
        PollVoteOutcome::Voted | PollVoteOutcome::Changed => {}
    }

    let counts = match state.db.poll_counts(poll_id) {
        Ok(counts) => counts,
        Err(error) => {
            tracing::warn!("Failed to count poll #{poll_id}: {error}");
            return component_error(ctx, ci, "This poll could not be refreshed.").await;
        }
    };
    let total = state.db.total_votes(poll_id).unwrap_or(0);
    let embed = poll_embed(&poll, &counts, total, false);

    update_message(
        ctx,
        ci,
        embed,
        poll_components(poll_id, &poll.options, false),
    )
    .await
}

async fn close_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
    poll_id: i64,
) -> BotResult<()> {
    let Some(poll) = load_component_poll(state, ctx, ci, poll_id).await? else {
        return Ok(());
    };

    let granted = invoker_permissions(ci.member.as_ref());
    if !can_manage_poll(poll.author_id, ci.user.id.get(), granted) {
        return component_error(
            ctx,
            ci,
            "Only the poll author or a member with `Manage Messages` can close this poll.",
        )
        .await;
    }

    if let Err(error) = state.db.close_poll(poll_id) {
        tracing::warn!("Failed to close poll #{poll_id}: {error}");
        return component_error(ctx, ci, "This poll could not be closed.").await;
    }

    let closed = match state.db.get_poll(poll_id) {
        Ok(Some(closed)) => closed,
        Ok(None) => return component_error(ctx, ci, "This poll no longer exists.").await,
        Err(error) => {
            tracing::warn!("Failed to reload poll #{poll_id}: {error}");
            return component_error(ctx, ci, "This poll could not be refreshed.").await;
        }
    };
    let counts = state
        .db
        .poll_counts(poll_id)
        .unwrap_or_else(|_| vec![0; closed.options.len()]);
    let total = state.db.total_votes(poll_id).unwrap_or(0);
    let embed = poll_embed(&closed, &counts, total, true);

    update_message(ctx, ci, embed, Vec::new()).await
}

async fn load_component_poll(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
    poll_id: i64,
) -> BotResult<Option<Poll>> {
    let Some(guild_id) = ci.guild_id else {
        component_error(ctx, ci, "Polls are only available in servers.").await?;
        return Ok(None);
    };

    match state.db.get_poll(poll_id) {
        Ok(Some(poll)) if poll.guild_id == guild_id.get() => Ok(Some(poll)),
        Ok(_) => {
            component_error(ctx, ci, "This poll no longer exists.").await?;
            Ok(None)
        }
        Err(error) => {
            tracing::warn!("Failed to load poll #{poll_id}: {error}");
            component_error(ctx, ci, "This poll could not be loaded.").await?;
            Ok(None)
        }
    }
}

async fn update_message(
    ctx: &Context,
    ci: &ComponentInteraction,
    embed: CreateEmbed,
    components: Vec<CreateActionRow>,
) -> BotResult<()> {
    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .embed(embed)
                .components(components)
                .allowed_mentions(CreateAllowedMentions::new()),
        ),
    )
    .await?;
    Ok(())
}

async fn component_error(ctx: &Context, ci: &ComponentInteraction, message: &str) -> BotResult<()> {
    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(message)
                .allowed_mentions(CreateAllowedMentions::new())
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

/// Author or moderator (`Manage Messages` / `Administrator`).
fn can_manage_poll(author_id: u64, user_id: u64, granted: Permissions) -> bool {
    author_id == user_id
        || granted.contains(Permissions::ADMINISTRATOR)
        || granted.contains(Permissions::MANAGE_MESSAGES)
}

fn invoker_permissions(member: Option<&Member>) -> Permissions {
    member
        .and_then(|member| member.permissions)
        .unwrap_or_else(Permissions::empty)
}

fn has_duplicates(options: &[String]) -> bool {
    options.iter().enumerate().any(|(index, option)| {
        options[..index]
            .iter()
            .any(|other| other.eq_ignore_ascii_case(option))
    })
}

fn jump_link(guild_id: u64, channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
}

/// One row of vote buttons (max 5) plus a close button row.
fn poll_components(poll_id: i64, options: &[String], closed: bool) -> Vec<CreateActionRow> {
    if closed {
        return Vec::new();
    }

    let mut rows = Vec::with_capacity(2);
    if !options.is_empty() {
        let buttons = options
            .iter()
            .take(NUMBER_EMOJIS.len())
            .enumerate()
            .map(|(index, option)| {
                CreateButton::new(format!("poll:vote:{poll_id}:{index}"))
                    .style(ButtonStyle::Primary)
                    .label(button_label(index, option))
            })
            .collect();
        rows.push(CreateActionRow::Buttons(buttons));
    }
    rows.push(CreateActionRow::Buttons(vec![
        CreateButton::new(format!("poll:close:{poll_id}"))
            .style(ButtonStyle::Danger)
            .label(CLOSE_LABEL),
    ]));
    rows
}

fn button_label(index: usize, option: &str) -> String {
    format!("{} {}", number_emoji(index), text::truncate(option, 60))
}

fn number_emoji(index: usize) -> &'static str {
    NUMBER_EMOJIS.get(index).copied().unwrap_or("•")
}

/// Ten-character proportional bar; zero votes renders ten empty cells.
fn vote_bar(count: i64, total: i64) -> String {
    let filled = if total <= 0 {
        0
    } else {
        (count.max(0) * 10 / total).clamp(0, 10) as usize
    };
    format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
}

/// Result embed: title, one bar line per option, total in the footer.
fn poll_embed(poll: &Poll, counts: &[i64], total: i64, closed: bool) -> CreateEmbed {
    let total = total.max(0);
    let mut lines = Vec::with_capacity(poll.options.len() + 1);
    if closed {
        lines.push("🔒 **Sondage terminé**".to_owned());
    }

    for (index, option) in poll.options.iter().enumerate() {
        let count = counts.get(index).copied().unwrap_or(0).max(0);
        let percent = if total > 0 { count * 100 / total } else { 0 };
        lines.push(format!(
            "{emoji} {option}\n`{bar}` **{count}** vote(s) — {percent}%",
            emoji = number_emoji(index),
            option = text::escape_md(option),
            bar = vote_bar(count, total),
        ));
    }

    CreateEmbed::new()
        .title(format!(
            "📊 {}",
            text::truncate(&poll.question, MAX_QUESTION_CHARS)
        ))
        .description(lines.join("\n\n"))
        .colour(if closed {
            embeds::AMBER
        } else {
            embeds::brand()
        })
        .footer(CreateEmbedFooter::new(format!(
            "Total: {total} vote(s) · Poll #{}",
            poll.id
        )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollAction {
    Vote { poll_id: i64, choice: usize },
    Close { poll_id: i64 },
}

fn parse_custom_id(custom_id: &str) -> Option<PollAction> {
    let rest = custom_id.strip_prefix("poll:")?;
    let (action, tail) = rest.split_once(':')?;

    match action {
        "vote" => {
            let (id, choice) = tail.split_once(':')?;
            Some(PollAction::Vote {
                poll_id: parse_poll_id(id)?,
                choice: choice.parse::<usize>().ok()?,
            })
        }
        "close" => Some(PollAction::Close {
            poll_id: parse_poll_id(tail)?,
        }),
        _ => None,
    }
}

fn parse_poll_id(raw: &str) -> Option<i64> {
    raw.parse::<i64>().ok().filter(|id| *id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample_poll(closed: bool) -> Poll {
        Poll {
            id: 7,
            guild_id: 1,
            channel_id: 2,
            message_id: None,
            author_id: 3,
            question: "Best scanner?".to_owned(),
            options: vec!["nmap".to_owned(), "rustscan".to_owned()],
            created_at: 0,
            closed_at: closed.then_some(1),
        }
    }

    fn embed_json(poll: &Poll, counts: &[i64], total: i64, closed: bool) -> Value {
        serde_json::to_value(poll_embed(poll, counts, total, closed)).expect("embed serializes")
    }

    #[test]
    fn custom_ids_parse() {
        assert_eq!(
            parse_custom_id("poll:vote:12:2"),
            Some(PollAction::Vote {
                poll_id: 12,
                choice: 2
            })
        );
        assert_eq!(
            parse_custom_id("poll:vote:1:0"),
            Some(PollAction::Vote {
                poll_id: 1,
                choice: 0
            })
        );
        assert_eq!(
            parse_custom_id("poll:close:7"),
            Some(PollAction::Close { poll_id: 7 })
        );

        for bad in [
            "poll:vote:0:0",
            "poll:vote:12",
            "poll:vote:12:x",
            "poll:vote:12:2:3",
            "poll:close:0",
            "poll:close:7:1",
            "poll:nope:7",
            "vote:up:1",
            "",
            "poll:",
        ] {
            assert_eq!(parse_custom_id(bad), None, "should reject {bad}");
        }
    }

    #[test]
    fn embed_shows_counts_percent_and_bars() {
        let value = embed_json(&sample_poll(false), &[3, 1], 4, false);
        assert_eq!(value["title"], "📊 Best scanner?");
        let description = value["description"].as_str().expect("description");

        assert!(description.contains("1️⃣ nmap"), "{description}");
        assert!(description.contains("**3** vote(s) — 75%"), "{description}");
        assert!(description.contains("`███████░░░`"), "{description}");
        assert!(description.contains("`██░░░░░░░░`"), "{description}");
        assert!(description.contains("**1** vote(s) — 25%"), "{description}");
        assert!(!description.contains("Sondage terminé"));

        let footer = value["footer"]["text"].as_str().expect("footer");
        assert!(footer.contains("Total: 4 vote(s)"), "{footer}");
    }

    #[test]
    fn zero_votes_render_empty_bars() {
        let value = embed_json(&sample_poll(false), &[0, 0], 0, false);
        let description = value["description"].as_str().expect("description");
        assert_eq!(description.matches("`░░░░░░░░░░`").count(), 2);
        assert!(description.contains("**0** vote(s) — 0%"));
        assert_eq!(vote_bar(0, 0), "░".repeat(10));
        assert_eq!(vote_bar(0, 5), "░".repeat(10));
        assert_eq!(vote_bar(5, 5), "█".repeat(10));
    }

    #[test]
    fn closed_embed_is_marked_and_has_no_buttons() {
        let poll = sample_poll(true);
        let value = embed_json(&poll, &[1, 1], 2, true);
        let description = value["description"].as_str().expect("description");
        assert!(description.contains("Sondage terminé"), "{description}");
        assert!(poll_components(poll.id, &poll.options, true).is_empty());
    }

    #[test]
    fn components_carry_custom_ids() {
        let poll = sample_poll(false);
        let rows = poll_components(poll.id, &poll.options, false);
        assert_eq!(rows.len(), 2);

        let value = serde_json::to_value(&rows).expect("components serialize");
        assert_eq!(value[0]["components"][0]["custom_id"], "poll:vote:7:0");
        assert_eq!(value[0]["components"][1]["custom_id"], "poll:vote:7:1");
        assert_eq!(value[1]["components"][0]["custom_id"], "poll:close:7");
        assert_eq!(value[1]["components"][0]["style"], 4);
        assert!(
            value[0]["components"][0]["label"]
                .as_str()
                .expect("label")
                .starts_with("1️⃣")
        );
    }

    #[test]
    fn permission_decision() {
        let none = Permissions::empty();
        assert!(can_manage_poll(5, 5, none), "author may close");
        assert!(!can_manage_poll(5, 6, none), "stranger may not");
        assert!(can_manage_poll(5, 6, Permissions::MANAGE_MESSAGES));
        assert!(can_manage_poll(5, 6, Permissions::ADMINISTRATOR));
        assert!(!can_manage_poll(5, 6, Permissions::KICK_MEMBERS));
        assert!(!can_manage_poll(5, 6, Permissions::SEND_MESSAGES));
    }

    #[test]
    fn duplicate_options_are_detected() {
        assert!(has_duplicates(&["nmap".to_owned(), "NMAP".to_owned()]));
        assert!(has_duplicates(&[
            "a".to_owned(),
            "b".to_owned(),
            "a".to_owned()
        ]));
        assert!(!has_duplicates(&["nmap".to_owned(), "rustscan".to_owned()]));
        assert!(!has_duplicates(&[]));
    }

    #[test]
    fn command_definition_exposes_three_subcommands() {
        let commands = commands();
        assert_eq!(commands.len(), 1);
        let value = serde_json::to_value(&commands[0]).expect("serialize");
        assert_eq!(value["name"], "poll");

        let names: Vec<&str> = value["options"]
            .as_array()
            .expect("options")
            .iter()
            .map(|option| option["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, vec!["create", "close", "results"]);
    }
}
