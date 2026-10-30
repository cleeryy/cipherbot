//! Interaction response helpers.
//!
//! Every handler acknowledges first. `error` is defensive: if the interaction was
//! already acknowledged it falls back to editing the existing response.

use std::fmt::Display;

use serenity::all::{
    CommandInteraction, Context, CreateAllowedMentions, CreateEmbed, CreateInteractionResponse,
    CreateInteractionResponseMessage, EditInteractionResponse,
};

use crate::bot::BotResult;
use crate::util::text::truncate;

const MAX_CONTENT: usize = 2_000;

/// Acknowledge with an ephemeral placeholder while slow work runs.
pub async fn working(
    ctx: &Context,
    cmd: &CommandInteraction,
    text: impl Into<String>,
) -> BotResult<()> {
    let content = truncate(&text.into(), MAX_CONTENT);
    cmd.create_response(
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

/// Public reply carrying an embed.
pub async fn public(ctx: &Context, cmd: &CommandInteraction, embed: CreateEmbed) -> BotResult<()> {
    cmd.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .embed(embed)
                .allowed_mentions(CreateAllowedMentions::new()),
        ),
    )
    .await?;
    Ok(())
}

/// Ephemeral reply carrying an embed.
pub async fn ephemeral(
    ctx: &Context,
    cmd: &CommandInteraction,
    embed: CreateEmbed,
) -> BotResult<()> {
    cmd.create_response(
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

/// Replace the placeholder created earlier.
pub async fn edit(ctx: &Context, cmd: &CommandInteraction, embed: CreateEmbed) -> BotResult<()> {
    cmd.edit_response(&ctx.http, EditInteractionResponse::new().embed(embed))
        .await?;
    Ok(())
}

/// Plain-text reply, public or ephemeral.
pub async fn text(
    ctx: &Context,
    cmd: &CommandInteraction,
    content: impl Into<String>,
    ephemeral: bool,
) -> BotResult<()> {
    let content = truncate(&content.into(), MAX_CONTENT);
    cmd.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(content)
                .allowed_mentions(CreateAllowedMentions::new())
                .ephemeral(ephemeral),
        ),
    )
    .await?;
    Ok(())
}

/// Always-ephemeral error reply. Never leaks `Debug` output.
pub async fn error(ctx: &Context, cmd: &CommandInteraction, err: impl Display) -> BotResult<()> {
    let content = truncate(&format!("❌ {err}"), MAX_CONTENT);

    let message = CreateInteractionResponseMessage::new()
        .content(content.clone())
        .allowed_mentions(CreateAllowedMentions::new())
        .ephemeral(true);

    if cmd
        .create_response(&ctx.http, CreateInteractionResponse::Message(message))
        .await
        .is_ok()
    {
        return Ok(());
    }

    // Most likely already acknowledged: edit the placeholder instead.
    cmd.edit_response(&ctx.http, EditInteractionResponse::new().content(content))
        .await?;
    Ok(())
}
