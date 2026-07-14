use std::sync::Arc;

use linkify::{LinkFinder, LinkKind};
use serenity::all::ChannelType;
use serenity::async_trait;
use serenity::builder::CreateThread;
use serenity::model::channel::Channel;
use serenity::model::prelude::*;
use serenity::prelude::*;

use crate::config::Config;
use crate::db::Database;

pub struct Handler {
    pub config: Config,
    pub database: Arc<Database>,
}

impl Handler {
    pub fn new(config: Config, database: Arc<Database>) -> Self {
        tracing::debug!("Handler created with {} monitored category(ies)", config.categories.len());
        Handler { config, database }
    }

    fn is_monitored(&self, channel: &GuildChannel) -> bool {
        let Some(parent_id) = channel.parent_id else {
            tracing::trace!("Channel {} has no parent category", channel.id);
            return false;
        };
        let matched = self
            .config
            .categories
            .iter()
            .any(|cat| cat.id == parent_id.get());

        tracing::trace!(
            "is_monitored: channel {} parent={}, matched={}",
            channel.id,
            parent_id.get(),
            matched
        );
        matched
    }

    fn category_config(&self, channel: &GuildChannel) -> Option<crate::config::CategoryConfig> {
        let parent_id = channel.parent_id?;
        let result = self
            .config
            .categories
            .iter()
            .find(|cat| cat.id == parent_id.get())
            .cloned();
        tracing::trace!("category_config for channel {} (parent={}): {:?}", channel.id, parent_id.get(), result.as_ref().map(|c| c.id));
        result
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        tracing::info!(
            "[MESSAGE] Received from {} in channel {} | content(len={}): {:?}",
            msg.author.name,
            msg.channel_id,
            msg.content.len(),
            msg.content.chars().take(120).collect::<String>(),
        );

        if msg.author.bot {
            tracing::debug!("Skipping bot message from {}", msg.author.name);
            return;
        }

        if let Some(guild_id) = msg.guild_id {
            if let Some(guild) = ctx.cache.guild(guild_id) {
                tracing::debug!("Message guild: '{}' ({}), channel_id: {}", guild.name, guild_id, msg.channel_id);
            } else {
                tracing::debug!("Message guild {} not in cache", guild_id);
            }
        } else {
            tracing::debug!("Message has no guild (likely DM)");
        }

        let channel = match msg.channel_id.to_channel(&ctx.http).await {
            Ok(Channel::Guild(ch)) => {
                tracing::debug!(
                    "Resolved as guild channel: '{}' (kind={:?}, parent={:?})",
                    ch.name,
                    ch.kind,
                    ch.parent_id.map(|id| id.get())
                );
                ch
            }
            Ok(_) => {
                tracing::debug!("Message not in a guild channel, skipping");
                return;
            }
            Err(e) => {
                tracing::warn!("Failed to resolve channel {}: {}", msg.channel_id, e);
                return;
            }
        };

        if matches!(
            channel.kind,
            ChannelType::PublicThread | ChannelType::PrivateThread
        ) {
            tracing::debug!("Message is inside a thread, skipping");
            return;
        }

        let parent_id = channel.parent_id.map(|id| id.get());
        let monitored_ids: Vec<u64> = self.config.categories.iter().map(|c| c.id).collect();
        tracing::info!(
            "[CHECK] Channel '{}' ({}) — parent category: {:?} — monitored categories: {:?}",
            channel.name,
            channel.id,
            parent_id,
            monitored_ids,
        );

        if !self.is_monitored(&channel) {
            tracing::info!(
                "[SKIP] Channel '{}' is NOT in any monitored category (parent={:?}, monitored={:?})",
                channel.name,
                parent_id,
                monitored_ids,
            );
            return;
        }

        tracing::info!(
            "[IN SCOPE] Channel '{}' IS in monitored category {:?}",
            channel.name,
            parent_id,
        );

        let cat_config = match self.category_config(&channel) {
            Some(c) => c,
            None => {
                tracing::warn!("category_config returned None despite is_monitored passing — this should not happen");
                return;
            }
        };

        tracing::info!(
            "[PROCESSING] Message from {} in '{}': {:?}",
            msg.author.name,
            channel.name,
            msg.content.chars().take(150).collect::<String>(),
        );

        let finder = LinkFinder::new();
        let links: Vec<String> = finder
            .links(&msg.content)
            .filter(|l| matches!(l.kind(), LinkKind::Url))
            .map(|l| l.as_str().to_string())
            .collect();

        let has_link = !links.is_empty();
        tracing::info!(
            "[LINK CHECK] has_link={}, found {} URL(s): {:?}",
            has_link,
            links.len(),
            links,
        );

        if has_link && cat_config.auto_thread_links {
            let thread_name = msg
                .content
                .chars()
                .take(80)
                .collect::<String>()
                .replace('\n', " ")
                .trim()
                .to_string();

            let thread_name = if thread_name.is_empty() {
                "Lien".to_string()
            } else {
                thread_name
            };

            tracing::info!(
                "[THREAD] Attempting to create thread '{}' from message {} in channel '{}' ({})...",
                thread_name,
                msg.id,
                channel.name,
                channel.id,
            );

            match msg
                .channel_id
                .create_thread_from_message(&ctx.http, msg.id, CreateThread::new(thread_name.clone()))
                .await
            {
                Ok(thread) => {
                    tracing::info!(
                        "[THREAD] SUCCESS: Created thread '{}' (id={}) from message {} in channel '{}' ({})",
                        thread_name,
                        thread.id,
                        msg.id,
                        channel.name,
                        channel.id,
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "[THREAD] FAILED to create thread for message {} in channel '{}' ({}): {}",
                        msg.id,
                        channel.name,
                        channel.id,
                        e,
                    );
                    tracing::error!(
                        "[THREAD] This is likely a permissions issue — the bot needs 'Create Threads' and 'Send Messages in Threads' permissions in channel '{}'",
                        channel.name,
                    );
                }
            }
        } else if !has_link {
            tracing::info!(
                "[TRACK] Message {} has no link — tracking for deletion in {}h",
                msg.id,
                cat_config.message_ttl_hours,
            );
            if let Err(e) = self.database.track_message(
                msg.id.get(),
                msg.channel_id.get(),
                cat_config.message_ttl_hours,
            ) {
                tracing::error!("[TRACK] FAILED to track message {}: {}", msg.id, e);
            } else {
                tracing::info!(
                    "[TRACK] SUCCESS: Message {} will be deleted in {}h",
                    msg.id,
                    cat_config.message_ttl_hours,
                );
            }
        } else {
            tracing::debug!(
                "Message has link but auto_thread_links is disabled for this category"
            );
        }
    }

    async fn ready(&self, ctx: Context, data_about_bot: Ready) {
        tracing::info!("{} is connected!", data_about_bot.user.name);

        match ctx.http.get_guilds(None, None).await {
            Ok(guilds) => {
                tracing::info!("Connected to {} guild(s):", guilds.len());
                for guild in &guilds {
                    tracing::info!("  - {} (id={})", guild.name, guild.id);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch guild list: {}", e);
            }
        }
    }
}
