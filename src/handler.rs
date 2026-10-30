//! Serenity event router. Logic lives in `commands`, `components`, `security`,
//! `services` and `commands::links`; this file only wires events to them.
//!
//! The health server is spawned from `main` (single owner, easier shutdown),
//! while `cleanup_loop` and `scheduler::run` are spawned from `ready` because
//! they need a `Context`.

use std::sync::OnceLock;
use std::time::Duration;

use serenity::all::{
    ActivityData, Context, EventHandler, Guild, GuildId, Interaction, Member, Message,
    OnlineStatus, Ready, User,
};
use serenity::async_trait;

use crate::bot::SharedState;
use crate::commands;
use crate::components;
use crate::security;
use crate::services;
use crate::util::respond;

/// Result of one message hook. Any `Stop` ends the pipeline for that message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookResult {
    Continue,
    Stop,
}

pub struct Handler {
    pub state: SharedState,
}

impl Handler {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

const STATUSES: &[&str] = &[
    "🔍 scanning ports",
    "🛡️ bypassing firewall",
    "💀 escalating privileges",
    "📡 sniffing packets",
    "🔓 cracking hashes",
    "🌐 tracing routes",
    "🎯 exploiting vectors",
];

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        let _ = self.state.bot_user_id.set(ready.user.id);
        tracing::info!(
            "Connected as {} ({}) — {} guild(s)",
            ready.user.name,
            ready.user.id,
            ready.guilds.len()
        );

        static BACKGROUND_STARTED: OnceLock<()> = OnceLock::new();
        if BACKGROUND_STARTED.set(()).is_ok() {
            spawn_presence_rotation(&ctx);
            tokio::spawn(commands::links::cleanup_loop(self.state.clone()));
            tokio::spawn(services::scheduler::run(self.state.clone(), ctx.clone()));

            // Warm the phishing blocklist immediately instead of waiting for the
            // first 15-minute scheduler tick.
            let state = self.state.clone();
            tokio::spawn(async move {
                match security::phishing::refresh(&state).await {
                    Ok(count) => tracing::info!("Phishing feed warmed: {count} domain(s)"),
                    Err(error) => tracing::warn!("Initial phishing feed refresh failed: {error}"),
                }
            });

            tracing::info!("Background loops spawned (presence, cleanup, scheduler)");
        }

        match ctx.http.get_guilds(None, None).await {
            Ok(guilds) => {
                self.state.metrics.set_guilds(guilds.len() as u64);
                for guild in &guilds {
                    match GuildId::set_commands(guild.id, &ctx.http, commands::all_commands()).await
                    {
                        Ok(registered) => tracing::info!(
                            "Registered {} command(s) in '{}' ({})",
                            registered.len(),
                            guild.name,
                            guild.id
                        ),
                        Err(error) => tracing::warn!(
                            "Failed to register commands in '{}' ({}): {error}",
                            guild.name,
                            guild.id
                        ),
                    }
                }
            }
            Err(error) => tracing::warn!("Failed to fetch guild list: {error}"),
        }
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: Option<bool>) {
        tracing::info!(
            "Guild available: '{}' ({}), new={:?}",
            guild.name,
            guild.id,
            is_new
        );

        match GuildId::set_commands(guild.id, &ctx.http, commands::all_commands()).await {
            Ok(registered) => tracing::info!(
                "Registered {} command(s) in '{}' ({})",
                registered.len(),
                guild.name,
                guild.id
            ),
            Err(error) => tracing::warn!("Command registration failed for {}: {error}", guild.id),
        }

        self.state
            .metrics
            .set_guilds(ctx.cache.guilds().len() as u64);
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => {
                if let Err(error) = commands::dispatch(&self.state, &ctx, &cmd).await {
                    self.state.metrics.inc_errors();
                    tracing::error!("Command /{} failed: {error:#}", cmd.data.name);
                    if let Err(reply_error) = respond::error(&ctx, &cmd, format!("{error:#}")).await
                    {
                        tracing::warn!("Could not report command error: {reply_error}");
                    }
                }
            }
            Interaction::Component(ci) => {
                if let Err(error) = components::dispatch(&self.state, &ctx, &ci).await {
                    self.state.metrics.inc_errors();
                    tracing::error!("Component '{}' failed: {error:#}", ci.data.custom_id);
                    let _ = ci
                        .create_response(
                            &ctx.http,
                            serenity::all::CreateInteractionResponse::Message(
                                serenity::all::CreateInteractionResponseMessage::new()
                                    .content("Something went wrong. Please try again.")
                                    .ephemeral(true),
                            ),
                        )
                        .await;
                }
            }
            Interaction::Autocomplete(cmd) => {
                // No option uses autocomplete today; answer with an empty list so
                // Discord never shows "loading failed" if one is added later.
                let _ = cmd
                    .create_response(
                        &ctx.http,
                        serenity::all::CreateInteractionResponse::Autocomplete(
                            serenity::all::CreateAutocompleteResponse::new(),
                        ),
                    )
                    .await;
            }
            Interaction::Modal(mi) => {
                let _ = mi
                    .create_response(
                        &ctx.http,
                        serenity::all::CreateInteractionResponse::Message(
                            serenity::all::CreateInteractionResponseMessage::new()
                                .content("Modal submissions are not supported.")
                                .ephemeral(true),
                        ),
                    )
                    .await;
            }
            _ => {}
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot || msg.webhook_id.is_some() {
            return;
        }

        self.state.metrics.inc_messages();

        if security::automod::on_message(&self.state, &ctx, &msg).await == HookResult::Stop {
            return;
        }
        if security::flagwatch::on_message(&self.state, &ctx, &msg).await == HookResult::Stop {
            return;
        }
        // Community XP is awarded for every message that survives moderation;
        // curation runs afterwards and may forward or remove the message.
        let _ = crate::db::users::on_message(&self.state, &ctx, &msg).await;
        if commands::links::on_message(&self.state, &ctx, &msg).await == HookResult::Stop {
            return;
        }
    }

    async fn guild_member_addition(&self, ctx: Context, member: Member) {
        if !self.state.config.features.member_intent {
            return;
        }
        if let Err(error) = security::raid::on_member_join(&self.state, &ctx, &member).await {
            self.state.metrics.inc_errors();
            tracing::warn!("Anti-raid join hook failed for {}: {error}", member.user.id);
        }
    }

    async fn guild_member_removal(
        &self,
        ctx: Context,
        guild_id: serenity::all::GuildId,
        user: User,
        _member_data: Option<Member>,
    ) {
        if !self.state.config.features.member_intent {
            return;
        }
        if let Err(error) = security::raid::on_member_remove(&self.state, &ctx, &user).await {
            self.state.metrics.inc_errors();
            tracing::warn!(
                "Anti-raid leave hook failed for {} in {guild_id}: {error}",
                user.id
            );
        }
    }
}

fn spawn_presence_rotation(ctx: &Context) {
    ctx.set_presence(
        Some(ActivityData::custom(STATUSES[0])),
        OnlineStatus::Online,
    );

    let ctx = ctx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3_600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // skip the immediate tick
        let mut index = 1usize;
        loop {
            interval.tick().await;
            let status = STATUSES[index % STATUSES.len()];
            ctx.set_presence(Some(ActivityData::custom(status)), OnlineStatus::Online);
            tracing::debug!("Presence rotated to \"{status}\"");
            index += 1;
        }
    });
}
