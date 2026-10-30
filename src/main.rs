//! CipherBot bootstrap: config, database, shared state, gateway client and
//! graceful shutdown. The health server is also spawned here.

use std::sync::Arc;

use serenity::all::{Client, GatewayIntents};
use tracing_subscriber::EnvFilter;

mod bot;
mod commands;
mod components;
mod config;
mod db;
mod handler;
mod ops;
mod security;
mod services;
mod util;

#[allow(dead_code)]
pub mod content;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(tracing::Level::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let config = match config::Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[FATAL] Configuration error: {error}");
            std::process::exit(1);
        }
    };

    let database = Arc::new(db::Database::open(&config.database.path)?);
    database.initialize()?;
    tracing::info!("Database ready at '{}'", config.database.path);

    let state = bot::build_state(config, database)?;

    let mut intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MODERATION;
    if state.config.features.member_intent {
        intents |= GatewayIntents::GUILD_MEMBERS;
    }

    tracing::info!(
        "Starting CipherBot v{} — categories={:?}, health='{}' (enabled={}), member_intent={}",
        env!("CARGO_PKG_VERSION"),
        state
            .config
            .categories
            .iter()
            .map(|category| category.id)
            .collect::<Vec<_>>(),
        state.config.health.addr,
        state.config.health.enabled,
        state.config.features.member_intent,
    );

    let mut client = Client::builder(state.config.discord_token.expose(), intents)
        .event_handler(handler::Handler::new(state.clone()))
        .await?;

    let _health_handle = ops::health::spawn(state.clone());

    let shard_manager = client.shard_manager.clone();

    tokio::select! {
        result = client.start() => {
            if let Err(error) = result {
                tracing::error!("Gateway client stopped: {error}");
                return Err(error.into());
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutdown signal received — draining shards");
        }
    }

    shard_manager.shutdown_all().await;
    tracing::info!("CipherBot stopped");
    Ok(())
}
