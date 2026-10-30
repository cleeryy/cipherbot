//! Component (button / select menu) router.
//!
//! `custom_id` format: `module:action[:args...]`, max 100 chars, stateless.
//! Unknown or stale components get a polite ephemeral message.

use serenity::all::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
};

use crate::bot::{BotResult, SharedState};
use crate::commands::{community, ctf, learning, moderation, poll, start, ticket};

pub async fn dispatch(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<()> {
    let custom_id = ci.data.custom_id.as_str();

    let handled = if custom_id.starts_with("verify:") {
        moderation::handle_component(state, ctx, ci).await?
    } else if custom_id.starts_with("quiz:") {
        learning::handle_component(state, ctx, ci).await?
    } else if custom_id.starts_with("vote:") {
        community::handle_component(state, ctx, ci).await?
    } else if custom_id.starts_with("ctf:") {
        ctf::handle_component(state, ctx, ci).await?
    } else if custom_id.starts_with("start:") {
        start::handle_component(state, ctx, ci).await?
    } else if custom_id.starts_with("poll:") {
        poll::handle_component(state, ctx, ci).await?
    } else if custom_id.starts_with("ticket:") {
        ticket::handle_component(state, ctx, ci).await?
    } else {
        false
    };

    if !handled {
        ci.create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("This button is no longer available.")
                    .ephemeral(true),
            ),
        )
        .await?;
    }

    Ok(())
}
