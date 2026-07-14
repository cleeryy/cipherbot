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
        Handler { config, database }
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

        let channel = match msg.channel_id.to_channel(&ctx.http).await {
            Ok(Channel::Guild(ch)) => ch,
            Ok(_) => {
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
            return;
        }

        let parent_id = channel.parent_id.map(|id| id.get());
        tracing::trace!(
            "Message in channel {} (parent category: {:?}) — monitored categories: {:?}",
            msg.channel_id,
            parent_id,
            self.config.categories.iter().map(|c| c.id).collect::<Vec<_>>()
        );

        if !self.is_monitored(&channel) {
            tracing::debug!(
                "Channel {} not in monitored categories (parent: {:?})",
                msg.channel_id,
                parent_id
            );
            return;
        }

        let cat_config = match self.category_config(&channel) {
            Some(c) => c,
            None => return,
        };

        tracing::debug!("Message content length: {}", msg.content.len());

        let finder = LinkFinder::new();
        let has_link = finder
            .links(&msg.content)
            .any(|l| matches!(l.kind(), LinkKind::Url));

        tracing::debug!("Message has_link={}, auto_thread_links={}", has_link, cat_config.auto_thread_links);

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

            let thread_name_display = thread_name.clone();
            match msg
                .channel_id
                .create_thread_from_message(&ctx.http, msg.id, CreateThread::new(thread_name))
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        "Created thread '{}' for message {} in channel {}",
                        thread_name_display,
                        msg.id,
                        msg.channel_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to create thread for message {}: {}",
                        msg.id,
                        e
                    );
                }
            }
        } else if !has_link {
            if let Err(e) = self.database.track_message(
                msg.id.get(),
                msg.channel_id.get(),
                cat_config.message_ttl_hours,
            ) {
                tracing::error!("Failed to track message {}: {}", msg.id, e);
            } else {
                tracing::debug!(
                    "Tracking message {} for deletion after {}h",
                    msg.id,
                    cat_config.message_ttl_hours
                );
            }
        }
    }

    async fn ready(&self, _ctx: Context, data_about_bot: Ready) {
        tracing::info!("{} is connected!", data_about_bot.user.name);
    }
}
