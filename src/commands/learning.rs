//! `/learn` — quiz, glossary, roadmaps, resources and tips (community lane).
//!
//! All content is compiled into the binary ([`crate::content`]); nothing here
//! touches the network. The daily quiz rotates deterministically on the UTC day
//! number and is answered through the `quiz:<question_id>:<choice>` buttons.

use chrono::Utc;
use serenity::all::{
    ButtonStyle, CommandInteraction, CommandOptionType, ComponentInteraction, Context,
    CreateActionRow, CreateButton, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
};

use crate::bot::{BotResult, SharedState};
use crate::content::{self, Category, QuizQuestion, ResourceKind, Track};
use crate::util::{embeds, opts, respond, text};

pub fn commands() -> Vec<CreateCommand> {
    let quiz = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "quiz",
        "Answer the daily cybersecurity quiz question",
    )
    .add_sub_option(category_option());

    let glossary = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "glossary",
        "Look up a security term",
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "term",
            "Term or keyword to look up",
        )
        .required(true)
        .max_length(100),
    );

    let roadmap = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "roadmap",
        "Show the learning roadmap for a track",
    )
    .add_sub_option(track_option());

    let resources = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "resources",
        "Browse curated courses, books, labs and tools",
    )
    .add_sub_option(
        CreateCommandOption::new(CommandOptionType::String, "kind", "Filter by resource type")
            .add_string_choice("Course", "course")
            .add_string_choice("Book", "book")
            .add_string_choice("Blog", "blog")
            .add_string_choice("Tool", "tool")
            .add_string_choice("Lab", "lab")
            .add_string_choice("Certification", "certification")
            .add_string_choice("Cheatsheet", "cheatsheet")
            .add_string_choice("Podcast", "podcast"),
    )
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "search",
            "Only show resources matching this text",
        )
        .max_length(100),
    );

    let tip = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "tip",
        "Show the tip of the day",
    );

    vec![
        CreateCommand::new("learn")
            .description("Quiz, glossary, roadmap and resources")
            .add_option(quiz)
            .add_option(glossary)
            .add_option(roadmap)
            .add_option(resources)
            .add_option(tip),
    ]
}

fn category_option() -> CreateCommandOption {
    let mut option = CreateCommandOption::new(
        CommandOptionType::String,
        "category",
        "Restrict the question to one category (default: today's pick)",
    );
    for category in Category::ALL {
        option = option.add_string_choice(category.label(), category.slug());
    }
    option
}

fn track_option() -> CreateCommandOption {
    let mut option = CreateCommandOption::new(
        CommandOptionType::String,
        "track",
        "Learning track to map out",
    )
    .required(true);
    for track in Track::ALL {
        option = option.add_string_choice(track.label(), track.slug());
    }
    option
}

pub async fn handle(
    _state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
) -> BotResult<()> {
    let Some((sub, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/learn` subcommand.").await;
    };

    match sub {
        "quiz" => quiz_command(ctx, cmd, options).await,
        "glossary" => glossary_command(ctx, cmd, options).await,
        "roadmap" => roadmap_command(ctx, cmd, options).await,
        "resources" => resources_command(ctx, cmd, options).await,
        "tip" => tip_command(ctx, cmd).await,
        other => respond::error(ctx, cmd, format!("Unknown `/learn` subcommand `{other}`.")).await,
    }
}

async fn quiz_command(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let category = opts::opt_str(options, "category").and_then(Category::from_slug);
    let question = daily_quiz(category, utc_day());

    let embed = quiz_embed(question);
    cmd.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .embed(embed)
                .components(vec![quiz_buttons(question)]),
        ),
    )
    .await?;
    Ok(())
}

async fn glossary_command(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let term = opts::req_str(options, "term")?;
    let entries = content::search_glossary(term, 5);
    if entries.is_empty() {
        return respond::ephemeral(
            ctx,
            cmd,
            embeds::warn(
                "No match",
                format!(
                    "No glossary entry matches `{}`. Try a shorter keyword.",
                    text::escape_md(term)
                ),
            ),
        )
        .await;
    }

    let mut embed = embeds::info(
        "Glossary",
        format!(
            "{} match(es) for `{}`.",
            entries.len(),
            text::escape_md(term)
        ),
    );
    for entry in entries {
        embed = embed.field(
            format!("{} · {}", entry.term, entry.category.label()),
            text::truncate(entry.definition, 900),
            false,
        );
    }
    respond::ephemeral(ctx, cmd, embed).await
}

async fn roadmap_command(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let track = opts::opt_str(options, "track")
        .and_then(Track::from_slug)
        .ok_or_else(|| anyhow::anyhow!("Pick a valid learning track."))?;
    let steps = content::steps_for_track(track);
    if steps.is_empty() {
        return respond::ephemeral(
            ctx,
            cmd,
            embeds::warn("No roadmap", "This track has no roadmap yet."),
        )
        .await;
    }

    let mut embed = embeds::info(
        format!("{} roadmap", track.label()),
        "Work through the steps in order; each one lists the resources to use.",
    );
    for step in steps {
        let resources = if step.resources.is_empty() {
            "—".to_owned()
        } else {
            step.resources.join(", ")
        };
        embed = embed.field(
            format!("{}. {}", step.order, step.title),
            text::truncate(
                &format!("{}\n**Resources:** {resources}", step.description),
                1_000,
            ),
            false,
        );
    }
    respond::ephemeral(ctx, cmd, embed).await
}

async fn resources_command(
    ctx: &Context,
    cmd: &CommandInteraction,
    options: &[serenity::all::CommandDataOption],
) -> BotResult<()> {
    let kind = opts::opt_str(options, "kind").and_then(resource_kind_from_slug);
    let needle = opts::opt_str(options, "search").map(|raw| raw.trim().to_ascii_lowercase());

    let matches: Vec<_> = content::resources::RESOURCES
        .iter()
        .filter(|resource| kind.is_none_or(|kind| resource.kind == kind))
        .filter(|resource| {
            needle.as_deref().is_none_or(|needle| {
                resource.name.to_ascii_lowercase().contains(needle)
                    || resource.description.to_ascii_lowercase().contains(needle)
            })
        })
        .take(15)
        .collect();

    if matches.is_empty() {
        return respond::ephemeral(
            ctx,
            cmd,
            embeds::warn("No resources", "Nothing matches those filters."),
        )
        .await;
    }

    let mut embed = embeds::info(
        "Resources",
        format!("Showing {} curated resource(s).", matches.len()),
    );
    for resource in matches {
        let price = if resource.free { "Free" } else { "Paid" };
        embed = embed.field(
            format!("{} · {}", resource.name, resource.kind.label()),
            format!(
                "[Open]({})\n{}\n*{price}*",
                resource.url,
                text::truncate(resource.description, 800)
            ),
            false,
        );
    }
    respond::ephemeral(ctx, cmd, embed).await
}

async fn tip_command(ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let tip = content::tip_of_day(utc_day());
    let embed =
        embeds::ok("Tip of the day", tip.text).field("Category", tip.category.label(), true);
    respond::ephemeral(ctx, cmd, embed).await
}

/// Buttons answer through [`handle_component`].
pub async fn handle_component(
    state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<bool> {
    let Some((question_id, choice)) = parse_quiz_custom_id(&ci.data.custom_id) else {
        component_error(ctx, ci, "This quiz button is malformed.").await?;
        return Ok(true);
    };
    let Some(question) = content::quiz_by_id(question_id) else {
        component_error(ctx, ci, "This quiz question no longer exists.").await?;
        return Ok(true);
    };
    let Some(guild_id) = ci.guild_id else {
        component_error(ctx, ci, "Quizzes are only available in servers.").await?;
        return Ok(true);
    };

    ci.defer_ephemeral(&ctx.http).await?;

    let correct = is_correct(question, choice);
    let outcome = match state.db.record_quiz_answer(
        guild_id.get(),
        ci.user.id.get(),
        correct,
        &utc_day_string(),
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!("Failed to record quiz answer for {}: {error}", ci.user.id);
            followup_ephemeral(
                ctx,
                ci,
                embeds::err("Could not record your answer. Please try again later."),
            )
            .await?;
            return Ok(true);
        }
    };

    if outcome.already_answered {
        followup_ephemeral(
            ctx,
            ci,
            embeds::warn(
                "Already answered",
                "You already played today's quiz. A new question arrives tomorrow (UTC).",
            ),
        )
        .await?;
        return Ok(true);
    }

    let embed = if correct {
        embeds::ok(
            "Correct!",
            format!(
                "{}\n\n**+{} XP** · streak **{}**",
                question.explanation, outcome.xp_awarded, outcome.streak
            ),
        )
    } else {
        embeds::footer(
            CreateEmbed::new()
                .title("Not quite")
                .description(format!(
                    "The correct answer is **{}**.\n\n{}\n\n**+{} XP** for trying · streak **{}**",
                    question.choices[question.answer],
                    question.explanation,
                    outcome.xp_awarded,
                    outcome.streak
                ))
                .colour(embeds::RED),
        )
    };

    followup_ephemeral(ctx, ci, embed).await?;
    Ok(true)
}

fn quiz_embed(question: &QuizQuestion) -> CreateEmbed {
    embeds::info("Daily quiz", question.question)
        .field("Category", question.category.label(), true)
        .field("Difficulty", question.difficulty.label(), true)
        .field("Reward", "+10 XP if correct · +2 XP for trying", true)
}

fn quiz_buttons(question: &QuizQuestion) -> CreateActionRow {
    let buttons = question
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let letter = (b'A' + index as u8) as char;
            CreateButton::new(format!("quiz:{}:{index}", question.id))
                .style(ButtonStyle::Secondary)
                .label(text::truncate(&format!("{letter} {choice}"), 80))
        })
        .collect();
    CreateActionRow::Buttons(buttons)
}

/// Parse `quiz:<question_id>:<choice_index>`; rejects out-of-range choices.
fn parse_quiz_custom_id(custom_id: &str) -> Option<(&str, usize)> {
    let rest = custom_id.strip_prefix("quiz:")?;
    let (question_id, index) = rest.rsplit_once(':')?;
    let index: usize = index.parse().ok()?;
    if question_id.is_empty() || index >= 4 {
        return None;
    }
    Some((question_id, index))
}

fn is_correct(question: &QuizQuestion, choice: usize) -> bool {
    choice < 4 && question.answer == choice
}

/// Deterministic daily pick, optionally restricted to one category.
fn daily_quiz(category: Option<Category>, day: u64) -> &'static QuizQuestion {
    match category {
        None => content::quiz_of_day(day),
        Some(category) => {
            let matching: Vec<&'static QuizQuestion> = content::quiz::QUESTIONS
                .iter()
                .filter(|question| question.category == category)
                .collect();
            if matching.is_empty() {
                content::quiz_of_day(day)
            } else {
                matching[(day as usize) % matching.len()]
            }
        }
    }
}

fn resource_kind_from_slug(raw: &str) -> Option<ResourceKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "course" => Some(ResourceKind::Course),
        "book" => Some(ResourceKind::Book),
        "blog" => Some(ResourceKind::Blog),
        "tool" => Some(ResourceKind::Tool),
        "lab" => Some(ResourceKind::Lab),
        "certification" => Some(ResourceKind::Certification),
        "cheatsheet" => Some(ResourceKind::Cheatsheet),
        "podcast" => Some(ResourceKind::Podcast),
        _ => None,
    }
}

/// UTC day number, used as the deterministic content seed.
fn utc_day() -> u64 {
    Utc::now().timestamp().div_euclid(86_400) as u64
}

/// `YYYY-MM-DD` (UTC), the quiz-day marker stored in the database.
fn utc_day_string() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

async fn followup_ephemeral(
    ctx: &Context,
    ci: &ComponentInteraction,
    embed: CreateEmbed,
) -> BotResult<()> {
    ci.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new()
            .embed(embed)
            .ephemeral(true),
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
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiz_custom_ids_are_parsed_strictly() {
        assert_eq!(
            parse_quiz_custom_id("quiz:crypto-rsa-small-e:3"),
            Some(("crypto-rsa-small-e", 3))
        );
        assert_eq!(
            parse_quiz_custom_id("quiz:crypto-rsa-small-e:0"),
            Some(("crypto-rsa-small-e", 0))
        );
        assert_eq!(parse_quiz_custom_id("quiz:abc:4"), None);
        assert_eq!(parse_quiz_custom_id("quiz:abc:-1"), None);
        assert_eq!(parse_quiz_custom_id("quiz:abc"), None);
        assert_eq!(parse_quiz_custom_id("quiz::1"), None);
        assert_eq!(parse_quiz_custom_id("vote:abc:1"), None);
    }

    #[test]
    fn answers_map_to_the_correct_index() {
        let question = content::quiz_of_day(0);
        assert!(is_correct(question, question.answer));
        assert!(!is_correct(question, (question.answer + 1) % 4));
        assert!(!is_correct(question, 4));
    }

    #[test]
    fn daily_quiz_is_deterministic_per_day() {
        let day = 20_000_u64;
        assert_eq!(daily_quiz(None, day).id, content::quiz_of_day(day).id);
        assert_eq!(daily_quiz(None, day).id, daily_quiz(None, day).id);

        let category = daily_quiz(Some(Category::Web), day);
        assert_eq!(category.category, Category::Web);
        assert_eq!(category.id, daily_quiz(Some(Category::Web), day).id);
    }

    #[test]
    fn every_learning_subcommand_is_registered() {
        let commands = commands();
        assert_eq!(commands.len(), 1);
        let value = serde_json::to_value(&commands[0]).expect("serialize");
        assert_eq!(value["name"], "learn");
        let names: Vec<&str> = value["options"]
            .as_array()
            .expect("options")
            .iter()
            .filter_map(|option| option["name"].as_str())
            .collect();
        assert_eq!(
            names,
            vec!["quiz", "glossary", "roadmap", "resources", "tip"]
        );
    }
}
