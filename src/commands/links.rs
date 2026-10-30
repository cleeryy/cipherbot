//! `/links` — link archive commands (community lane) **and** the live link
//! curation pipeline that predates v1 (core-owned, must keep working).
//!
//! Pipeline rules, per monitored category:
//! - a message containing a link gets a thread created from it (when enabled),
//!   unless the same URL was already threaded — then the duplicate is refused;
//! - every threaded (or, with auto-threading off, every posted) link is archived
//!   in the `links` table for `/links search|recent|stats`;
//! - a message without a link is forwarded to the channel's last thread, then
//!   deleted from the parent channel;
//! - forwarded copies are tracked for TTL deletion by [`cleanup_loop`].

use std::time::Duration;

use linkify::{LinkFinder, LinkKind};
use serenity::all::{
    Channel, ChannelId, ChannelType, CommandInteraction, Context, CreateCommand,
    CreateCommandOption, CreateEmbed, CreateThread, GuildChannel, Mentionable as _, Message,
};
use serenity::http::Http;

use crate::bot::{BotResult, SharedState};
use crate::db::links::ArchivedLink;
use crate::handler::HookResult;
use crate::util::{embeds, opts, respond, text};

pub fn commands() -> Vec<CreateCommand> {
    let search = CreateCommandOption::new(
        serenity::all::CommandOptionType::SubCommand,
        "search",
        "Search archived links",
    )
    .add_sub_option(
        CreateCommandOption::new(
            serenity::all::CommandOptionType::String,
            "query",
            "Text or domain to search for",
        )
        .required(true)
        .max_length(200),
    )
    .add_sub_option(
        CreateCommandOption::new(
            serenity::all::CommandOptionType::Integer,
            "limit",
            "Maximum results (1-25, default 10)",
        )
        .min_int_value(1)
        .max_int_value(25),
    );

    let recent = CreateCommandOption::new(
        serenity::all::CommandOptionType::SubCommand,
        "recent",
        "Show the most recent links",
    )
    .add_sub_option(
        CreateCommandOption::new(
            serenity::all::CommandOptionType::Integer,
            "limit",
            "Maximum results (1-25, default 10)",
        )
        .min_int_value(1)
        .max_int_value(25),
    );

    let stats = CreateCommandOption::new(
        serenity::all::CommandOptionType::SubCommand,
        "stats",
        "Show link archive statistics",
    );

    vec![
        CreateCommand::new("links")
            .description("Browse the community link archive")
            .add_option(search)
            .add_option(recent)
            .add_option(stats),
    ]
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "The link archive is only available in servers.").await;
    };
    let Some((sub, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/links` subcommand.").await;
    };

    match sub {
        "search" => {
            let query = opts::req_str(options, "query")?;
            let limit = limit_of(options);
            let links = state.db.search_links(guild_id.get(), query.trim(), limit)?;
            respond::public(
                ctx,
                cmd,
                link_embed(
                    guild_id.get(),
                    &format!("Links matching `{}`", text::escape_md(query.trim())),
                    &links,
                ),
            )
            .await
        }
        "recent" => {
            let limit = limit_of(options);
            let links = state.db.recent_links(guild_id.get(), limit)?;
            respond::public(ctx, cmd, link_embed(guild_id.get(), "Recent links", &links)).await
        }
        "stats" => {
            let (total, authors, domain, domain_count) = state.db.link_stats(guild_id.get())?;
            let embed = embeds::info(
                "Link archive",
                format!(
                    "**Archived links:** {total}\n\
                     **Contributors:** {authors}\n\
                     **Top domain:** {domain} ({domain_count} link(s))"
                ),
            );
            respond::public(ctx, cmd, embed).await
        }
        other => respond::error(ctx, cmd, format!("Unknown `/links` subcommand `{other}`.")).await,
    }
}

fn limit_of(options: &[serenity::all::CommandDataOption]) -> usize {
    opts::opt_i64(options, "limit").unwrap_or(10).clamp(1, 25) as usize
}

fn link_embed(guild_id: u64, title: &str, links: &[ArchivedLink]) -> CreateEmbed {
    if links.is_empty() {
        return embeds::info(title, "No archived links matched.");
    }
    let body = links
        .iter()
        .map(|link| link_line(guild_id, link))
        .collect::<Vec<_>>()
        .join("\n");
    embeds::info(title, text::truncate(&body, 4_000))
}

/// One result line: domain link, author, relative date and jump link.
fn link_line(guild_id: u64, link: &ArchivedLink) -> String {
    let target = match link.thread_id {
        Some(thread_id) => format!("https://discord.com/channels/{guild_id}/{thread_id}"),
        None => format!(
            "https://discord.com/channels/{guild_id}/{}/{}",
            link.channel_id, link.message_id
        ),
    };
    format!(
        "• [{}]({}) — <@{}> · <t:{}:R> · [open]({target})",
        text::escape_md(&link.domain),
        link.url,
        link.author_id,
        link.posted_at
    )
}

/// First http(s) URL in a message and its host (lowercased, no `www.`).
fn first_url(content: &str) -> Option<(&str, String)> {
    let finder = LinkFinder::new();
    finder
        .links(content)
        .find(|link| matches!(link.kind(), LinkKind::Url))
        .map(|link| {
            (
                link.as_str(),
                text::domain_of(link.as_str()).unwrap_or_else(|| "unknown".to_owned()),
            )
        })
}

fn duplicate_notice(guild_id: u64, author_id: u64, thread_id: u64) -> String {
    format!("<@{author_id}> already posted: https://discord.com/channels/{guild_id}/{thread_id}")
}

/// One message through the legacy link pipeline.
pub async fn on_message(state: &SharedState, ctx: &Context, msg: &Message) -> HookResult {
    if msg.author.bot || state.config.categories.is_empty() {
        return HookResult::Continue;
    }

    let Some(channel) = resolve_channel(ctx, msg.channel_id).await else {
        return HookResult::Continue;
    };

    if matches!(
        channel.kind,
        ChannelType::PublicThread | ChannelType::PrivateThread
    ) {
        return HookResult::Continue;
    }

    let Some(parent_id) = channel.parent_id else {
        return HookResult::Continue;
    };
    let Some(category) = state
        .config
        .categories
        .iter()
        .find(|category| category.id == parent_id.get())
    else {
        return HookResult::Continue;
    };

    // Guild-level `/config links` overrides the per-category YAML defaults when
    // the server has explicitly configured them.
    let overrides = msg
        .guild_id
        .and_then(|guild_id| state.db.get_settings(guild_id.get()).ok())
        .unwrap_or_default();
    let auto_thread = if overrides.links.links_custom {
        overrides.links.auto_thread
    } else {
        category.auto_thread_links
    };
    let ttl_hours = if overrides.links.links_custom {
        overrides.links.ttl_hours
    } else {
        category.message_ttl_hours
    };

    let finder = LinkFinder::new();
    let has_link = finder
        .links(&msg.content)
        .any(|link| matches!(link.kind(), LinkKind::Url));

    if has_link {
        let url_info = first_url(&msg.content);
        let guild_id = msg.guild_id.map(|id| id.get());

        if !auto_thread {
            // Archiving still happens when auto-threading is disabled, with no
            // thread to point at.
            if let (Some(guild_id), Some((url, domain))) = (guild_id, url_info.as_ref()) {
                archive_link(state, guild_id, msg, None, url, domain);
            }
            return HookResult::Continue;
        }

        // Refuse a URL already threaded: one thread per link.
        if let (Some(guild_id), Some((url, _))) = (guild_id, url_info.as_ref())
            && let Ok(Some((thread_id, _message_id, _channel_id))) =
                state.db.find_duplicate_link(guild_id, url)
        {
            let notice = duplicate_notice(guild_id, msg.author.id.get(), thread_id);
            if let Err(error) = msg.channel_id.say(&ctx.http, notice).await {
                tracing::warn!(
                    "Failed to announce duplicate link in '{}': {error}",
                    channel.name
                );
            }
            match msg.delete(&ctx.http).await {
                Ok(()) => return HookResult::Stop,
                Err(error) => {
                    tracing::warn!("Failed to delete duplicate link {}: {error}", msg.id);
                    return HookResult::Continue;
                }
            }
        }

        let name = thread_name(&msg.content, msg.id.get());
        match msg
            .channel_id
            .create_thread_from_message(&ctx.http, msg.id, CreateThread::new(name))
            .await
        {
            Ok(thread) => {
                state.metrics.inc_links_threaded();
                if let (Some(guild_id), Some((url, domain))) = (guild_id, url_info.as_ref()) {
                    archive_link(state, guild_id, msg, Some(thread.id.get()), url, domain);
                }
                if let Err(error) = state.db.set_last_thread(channel.id.get(), thread.id.get()) {
                    tracing::warn!("Failed to persist last thread: {error}");
                }
                tracing::info!(
                    "Created thread {} for link message {} in '{}'",
                    thread.id,
                    msg.id,
                    channel.name
                );
                return HookResult::Stop;
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to create link thread in '{}': {error}",
                    channel.name
                );
                // Still keep the link discoverable through `/links`.
                if let (Some(guild_id), Some((url, domain))) = (guild_id, url_info.as_ref()) {
                    archive_link(state, guild_id, msg, None, url, domain);
                }
                return HookResult::Continue;
            }
        }
    }

    // Non-link message: forward to the last thread, then remove from the parent.
    match state.db.get_last_thread(channel.id.get()) {
        Ok(Some(thread_id)) => {
            let forwarded = format!("{}\n> {}", msg.author.mention(), msg.content);
            match ChannelId::new(thread_id).say(&ctx.http, forwarded).await {
                Ok(sent) => {
                    if let Err(error) = state.db.track_message(sent.id.get(), thread_id, ttl_hours)
                    {
                        tracing::warn!("Failed to track forwarded message: {error}");
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to forward message {} to thread {thread_id}: {error}",
                        msg.id
                    );
                    // The thread was likely deleted: forget it so the next link
                    // creates a fresh one instead of losing messages forever.
                    if let Err(clear_error) = state.db.clear_last_thread(channel.id.get()) {
                        tracing::warn!("Failed to clear stale last thread: {clear_error}");
                    }
                }
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!("Failed to read last thread: {error}"),
    }

    match msg.delete(&ctx.http).await {
        Ok(()) => HookResult::Stop,
        Err(error) => {
            tracing::warn!("Failed to delete non-link message {}: {error}", msg.id);
            HookResult::Continue
        }
    }
}

fn archive_link(
    state: &SharedState,
    guild_id: u64,
    msg: &Message,
    thread_id: Option<u64>,
    url: &str,
    domain: &str,
) {
    if let Err(error) = state.db.archive_link(
        guild_id,
        msg.channel_id.get(),
        msg.id.get(),
        thread_id,
        msg.author.id.get(),
        url,
        domain,
        msg.timestamp.unix_timestamp(),
    ) {
        tracing::warn!("Failed to archive link message {}: {error}", msg.id);
    }
}

/// Cache-first channel resolution; REST only on a cache miss.
async fn resolve_channel(ctx: &Context, channel_id: ChannelId) -> Option<GuildChannel> {
    #[allow(deprecated)]
    let cached = ctx
        .cache
        .channel(channel_id)
        .map(|channel| (*channel).clone());
    if cached.is_some() {
        return cached;
    }

    match channel_id.to_channel(&ctx.http).await {
        Ok(Channel::Guild(channel)) => Some(channel),
        Ok(_) => None,
        Err(error) => {
            tracing::debug!("Failed to resolve channel {channel_id}: {error}");
            None
        }
    }
}

fn thread_name(content: &str, message_id: u64) -> String {
    let name: String = content
        .chars()
        .take(80)
        .collect::<String>()
        .replace(['\n', '\r'], " ")
        .trim()
        .to_owned();
    if name.is_empty() {
        format!("link-{message_id}")
    } else {
        name
    }
}

/// Delete TTL-expired messages every 60 seconds. Runs for the process lifetime.
pub async fn cleanup_loop(state: SharedState) {
    let http = Http::new(state.config.discord_token.expose());
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        let expired = match state.db.get_expired_messages() {
            Ok(messages) => messages,
            Err(error) => {
                tracing::error!("Cleanup: failed to query expired messages: {error}");
                continue;
            }
        };

        if expired.is_empty() {
            continue;
        }

        tracing::info!("Cleanup: deleting {} expired message(s)", expired.len());

        for (message_id, channel_id) in expired {
            match ChannelId::new(channel_id)
                .delete_message(&http, message_id)
                .await
            {
                Ok(()) => tracing::debug!("Deleted message {message_id} from channel {channel_id}"),
                Err(error) => {
                    let text = error.to_string();
                    if text.contains("Unknown Message") || text.contains("Missing Access") {
                        tracing::debug!("Message {message_id} already gone: {text}");
                    } else {
                        tracing::warn!("Failed to delete message {message_id}: {text}");
                    }
                }
            }

            if let Err(error) = state.db.remove_message(message_id) {
                tracing::error!("Cleanup: failed to remove message {message_id} from DB: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_names_are_bounded_and_clean() {
        assert_eq!(thread_name("hello\nworld", 1), "hello world");
        assert_eq!(thread_name("   ", 42), "link-42");
        let long = "x".repeat(200);
        assert_eq!(thread_name(&long, 1).chars().count(), 80);
    }

    #[test]
    fn extracts_first_url_and_domain() {
        let (url, domain) =
            first_url("read https://www.Example.COM/a?b=1 and https://other.example").expect("url");
        assert_eq!(url, "https://www.Example.COM/a?b=1");
        assert_eq!(domain, "example.com");

        assert!(first_url("no links here").is_none());
    }

    #[test]
    fn duplicate_notice_links_the_thread() {
        let notice = duplicate_notice(1, 42, 900);
        assert!(notice.contains("<@42>"));
        assert!(notice.contains("https://discord.com/channels/1/900"));
    }

    fn sample(thread_id: Option<u64>) -> ArchivedLink {
        ArchivedLink {
            url: "https://example.com/x".to_owned(),
            domain: "example.com".to_owned(),
            channel_id: 10,
            message_id: 11,
            thread_id,
            author_id: 42,
            posted_at: 1_790_000_000,
        }
    }

    #[test]
    fn link_lines_use_thread_or_message_jump_links() {
        let threaded = link_line(7, &sample(Some(99)));
        assert!(threaded.contains("https://discord.com/channels/7/99"));
        assert!(threaded.contains("<@42>"));
        assert!(threaded.contains("<t:1790000000:R>"));

        let plain = link_line(7, &sample(None));
        assert!(plain.contains("https://discord.com/channels/7/10/11"));
    }
}
