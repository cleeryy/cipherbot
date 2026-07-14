use linkify::{LinkFinder, LinkKind};
use serenity::all::ChannelType;
use serenity::async_trait;
use serenity::builder::CreateThread;
use serenity::model::channel::Channel;
use serenity::model::prelude::*;
use serenity::prelude::*;

use crate::config::Config;

pub struct Handler {
    pub config: Config,
}

impl Handler {
    pub fn new(config: Config) -> Self {
        tracing::debug!("Handler created with {} monitored category(ies)", config.categories.len());
        Handler { config }
    }

    fn is_monitored(&self, channel: &GuildChannel) -> bool {
        let Some(parent_id) = channel.parent_id else {
            return false;
        };
        self.config
            .categories
            .iter()
            .any(|cat| cat.id == parent_id.get())
    }

    fn category_config(&self, channel: &GuildChannel) -> Option<crate::config::CategoryConfig> {
        let parent_id = channel.parent_id?;
        self.config
            .categories
            .iter()
            .find(|cat| cat.id == parent_id.get())
            .cloned()
    }

}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        tracing::info!(
            "[MESSAGE] author={} channel={} guild={:?} content(len={})={:?}",
            msg.author.name,
            msg.channel_id,
            msg.guild_id,
            msg.content.len(),
            msg.content.chars().take(150).collect::<String>(),
        );

        let channel = match msg.channel_id.to_channel(&ctx.http).await {
            Ok(Channel::Guild(ch)) => ch,
            Ok(_) => {
                tracing::debug!("Non-guild channel, skipping");
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
            tracing::debug!("Message in thread, skipping");
            return;
        }

        let parent_id = channel.parent_id.map(|id| id.get());
        let monitored_ids: Vec<u64> = self.config.categories.iter().map(|c| c.id).collect();

        tracing::info!(
            "[CHECK] channel='{}' ({}) parent={:?} monitored={:?}",
            channel.name,
            channel.id,
            parent_id,
            monitored_ids,
        );

        if !self.is_monitored(&channel) {
            tracing::info!("[SKIP] '{}' not in monitored categories", channel.name);
            return;
        }

        tracing::info!("[IN] '{}' — processing message", channel.name);

        let cat_config = match self.category_config(&channel) {
            Some(c) => c,
            None => return,
        };

        let finder = LinkFinder::new();
        let links: Vec<String> = finder
            .links(&msg.content)
            .filter(|l| matches!(l.kind(), LinkKind::Url))
            .map(|l| l.as_str().to_string())
            .collect();

        let has_link = !links.is_empty();
        tracing::info!("[LINKS] has_link={} urls={:?}", has_link, links);

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
                "[THREAD] Creating '{}' from msg {} in '{}'",
                thread_name,
                msg.id,
                channel.name,
            );

            match msg
                .channel_id
                .create_thread_from_message(&ctx.http, msg.id, CreateThread::new(thread_name.clone()))
                .await
            {
                Ok(thread) => {
                    tracing::info!("[THREAD] ✅ Created '{}' (id={})", thread_name, thread.id);
                }
                Err(e) => {
                    tracing::error!("[THREAD] ❌ Failed: {}", e);
                }
            }
        } else if !has_link {
            tracing::info!(
                "[DELETE] No link — deleting message {} from {}",
                msg.id,
                channel.name,
            );
            if let Err(e) = msg.delete(&ctx.http).await {
                tracing::warn!("[DELETE] Failed to delete message {}: {}", msg.id, e);
            } else {
                tracing::debug!("[DELETE] Deleted non-link message {}", msg.id);
            }
        }
    }

    async fn ready(&self, ctx: Context, _data_about_bot: Ready) {
        tracing::info!("cipherbot is connected!");

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
