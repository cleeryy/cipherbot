use std::sync::Arc;

use serenity::all::ChannelId;
use serenity::prelude::*;
use tracing_subscriber::EnvFilter;

mod config;
mod db;
mod handler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(tracing::Level::INFO.into())
                .from_env_lossy(),
        )
        .init();

    dotenvy::dotenv().ok();

    let config = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[FATAL] Configuration error: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(
        "Configuration loaded — monitoring {} categorie(s): {:?}",
        config.categories.len(),
        config.categories.iter().map(|c| c.id).collect::<Vec<_>>()
    );

    let database = Arc::new(db::Database::open(&config.database.path)?);
    database.initialize()?;
    tracing::info!("Database initialized at {}", config.database.path);

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILDS;

    let handler = handler::Handler::new(config.clone(), database.clone());
    let mut client = Client::builder(&config.discord_token, intents)
        .event_handler(handler)
        .await?;

    let database_clone = database.clone();
    let token = config.discord_token.clone();
    tokio::spawn(async move {
        cleanup_task(database_clone, token).await;
    });

    tracing::info!("Starting cipherbot...");
    client.start().await?;

    Ok(())
}

async fn cleanup_task(db: Arc<db::Database>, token: String) {
    let http = serenity::http::Http::new(&token);
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

    loop {
        interval.tick().await;

        let messages = match db.get_expired_messages() {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::error!("Cleanup: failed to query expired messages: {}", e);
                continue;
            }
        };

        if messages.is_empty() {
            continue;
        }

        tracing::info!("Cleanup: deleting {} expired message(s)", messages.len());

        for (msg_id, ch_id) in &messages {
            let channel_id = ChannelId::new(*ch_id);

            match channel_id.delete_message(&http, *msg_id).await {
                Ok(_) => {
                    tracing::debug!("Deleted message {} from channel {}", msg_id, ch_id);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Unknown Message")
                        || err_str.contains("Missing Access")
                    {
                        tracing::debug!(
                            "Message {} already deleted or inaccessible: {}",
                            msg_id,
                            err_str
                        );
                    } else {
                        tracing::error!("Failed to delete message {}: {}", msg_id, err_str);
                    }
                }
            }

            if let Err(e) = db.remove_message(*msg_id) {
                tracing::error!("Cleanup: failed to remove message {} from DB: {}", msg_id, e);
            }
        }
    }
}
