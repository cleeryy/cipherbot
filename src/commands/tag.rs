//! `/tag` — reusable snippets and FAQ answers (tag lane).
//!
//! Reads are public so the answer helps everyone in the channel; writes require
//! `MANAGE_MESSAGES`. Content is stored raw and markdown-escaped at render time,
//! so re-saving a tag never stacks backslashes.

use serenity::all::{
    CommandDataOption, CommandInteraction, CommandOptionType, Context, CreateCommand,
    CreateCommandOption, CreateEmbed, CreateEmbedFooter, Permissions,
};

use crate::bot::{BotResult, SharedState};
use crate::db::tag::{self, Tag};
use crate::util::{embeds, opts, permissions, respond, text};

const MAX_NAME_CHARS: usize = tag::MAX_NAME_CHARS;
/// Hard cap on stored content (also enforced by the option definition).
const MAX_CONTENT_CHARS: usize = 1_500;
/// Embed description budget, leaving headroom under Discord's 4096 limit.
const MAX_EMBED_CONTENT: usize = 1_900;
/// Names shown by `/tag list`.
const MAX_LISTED: usize = 50;

pub fn commands() -> Vec<CreateCommand> {
    let get = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "get",
        "Show a saved tag by name",
    )
    .add_sub_option(name_option());

    let list = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "list",
        "List every tag saved in this server",
    );

    let add = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "add",
        "Save a new tag or replace its content (staff only)",
    )
    .add_sub_option(name_option())
    .add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "content",
            "Text shown by /tag get — max 1500 characters",
        )
        .required(true)
        .max_length(MAX_CONTENT_CHARS as u16),
    );

    let remove = CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "remove",
        "Delete a saved tag (staff only)",
    )
    .add_sub_option(name_option());

    vec![
        CreateCommand::new("tag")
            .description("Reusable snippets and FAQ answers")
            .add_option(get)
            .add_option(list)
            .add_option(add)
            .add_option(remove),
    ]
}

fn name_option() -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::String,
        "name",
        "Tag name (a-z, 0-9, _ or -)",
    )
    .required(true)
    .max_length(MAX_NAME_CHARS as u16)
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some(guild_id) = cmd.guild_id else {
        return respond::error(ctx, cmd, "`/tag` only works in a server.").await;
    };
    let guild_id = guild_id.get();

    let Some((subcommand, options)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/tag` subcommand.").await;
    };

    match subcommand {
        "get" => get(state, ctx, cmd, guild_id, options).await,
        "list" => list(state, ctx, cmd, guild_id).await,
        "add" => add(state, ctx, cmd, guild_id, options).await,
        "remove" => remove(state, ctx, cmd, guild_id, options).await,
        other => respond::error(ctx, cmd, format!("Unknown subcommand `{other}`.")).await,
    }
}

async fn get(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    let Some(raw_name) = opts::opt_str(options, "name") else {
        return respond::error(ctx, cmd, "Missing required option `name`.").await;
    };
    let name = match parse_name(raw_name) {
        Ok(name) => name,
        Err(message) => return respond::error(ctx, cmd, message).await,
    };

    match state.db.get_tag(guild_id, &name) {
        Ok(Some(tag)) => respond::public(ctx, cmd, format_tag_embed(&tag)).await,
        Ok(None) => {
            respond::error(ctx, cmd, format!("No tag named `{name}`. Try `/tag list`.")).await
        }
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn list(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
) -> BotResult<()> {
    match state.db.list_tags(guild_id) {
        Ok(tags) => respond::public(ctx, cmd, format_list_embed(&tags)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn add(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    if let Err(error) = permissions::require(cmd, Permissions::MANAGE_MESSAGES) {
        return respond::error(ctx, cmd, error).await;
    }

    let Some(raw_name) = opts::opt_str(options, "name") else {
        return respond::error(ctx, cmd, "Missing required option `name`.").await;
    };
    let name = match parse_name(raw_name) {
        Ok(name) => name,
        Err(message) => return respond::error(ctx, cmd, message).await,
    };

    let Some(raw_content) = opts::opt_str(options, "content") else {
        return respond::error(ctx, cmd, "Missing required option `content`.").await;
    };
    let content = text::truncate(raw_content.trim(), MAX_CONTENT_CHARS);
    if content.is_empty() {
        return respond::error(ctx, cmd, "Tag content cannot be empty.").await;
    }

    if let Err(error) = state
        .db
        .upsert_tag(guild_id, &name, &content, cmd.user.id.get())
    {
        return respond::error(ctx, cmd, error).await;
    }

    respond::ephemeral(
        ctx,
        cmd,
        embeds::ok(
            "Tag saved",
            format!("`{name}` is ready — read it with `/tag get {name}`."),
        ),
    )
    .await
}

async fn remove(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    guild_id: u64,
    options: &[CommandDataOption],
) -> BotResult<()> {
    if let Err(error) = permissions::require(cmd, Permissions::MANAGE_MESSAGES) {
        return respond::error(ctx, cmd, error).await;
    }

    let Some(raw_name) = opts::opt_str(options, "name") else {
        return respond::error(ctx, cmd, "Missing required option `name`.").await;
    };
    let name = match parse_name(raw_name) {
        Ok(name) => name,
        Err(message) => return respond::error(ctx, cmd, message).await,
    };

    match state.db.delete_tag(guild_id, &name) {
        Ok(true) => {
            respond::ephemeral(
                ctx,
                cmd,
                embeds::ok("Tag deleted", format!("`{name}` no longer exists.")),
            )
            .await
        }
        Ok(false) => respond::error(ctx, cmd, format!("No tag named `{name}`.")).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

/// Normalize a slash-command name, returning a user-facing error when invalid.
fn parse_name(raw: &str) -> Result<String, String> {
    tag::normalize_name(raw).ok_or_else(|| {
        format!(
            "Invalid tag name '{}' — use 1-32 characters from a-z, 0-9, `_` or `-`.",
            text::escape_md(raw.trim())
        )
    })
}

/// Public embed for `/tag get`.
fn format_tag_embed(tag: &Tag) -> CreateEmbed {
    let body = text::truncate(&text::escape_md(&tag.content), MAX_EMBED_CONTENT);
    embeds::info(format!("Tag: {}", tag.name), body).footer(CreateEmbedFooter::new(format!(
        "Created by {} · {} use(s)",
        tag.created_by, tag.uses
    )))
}

/// Public embed for `/tag list`, with an explicit empty state.
fn format_list_embed(tags: &[Tag]) -> CreateEmbed {
    if tags.is_empty() {
        return embeds::info(
            "Tags",
            "No tags saved yet. Staff can add one with `/tag add`.",
        );
    }

    let names = tags
        .iter()
        .take(MAX_LISTED)
        .map(|tag| format!("`{}`", tag.name))
        .collect::<Vec<_>>()
        .join(" · ");

    let mut description = format!("**{}** tag(s) saved.\n\n{names}", tags.len());
    if tags.len() > MAX_LISTED {
        description.push_str(&format!(
            "\n\nShowing the first {MAX_LISTED} — use `/tag get` to read any tag."
        ));
    }

    embeds::info("Tags", description)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::all::CommandData;

    fn tag(name: &str, content: &str) -> Tag {
        Tag {
            name: name.to_owned(),
            content: content.to_owned(),
            created_by: 42,
            uses: 7,
            created_at: 0,
        }
    }

    fn command_data(json: &str) -> CommandData {
        serde_json::from_str(json).expect("valid command fixture")
    }

    #[test]
    fn names_are_normalized_or_rejected() {
        assert_eq!(parse_name(" FAQ ").expect("valid"), "faq");
        assert_eq!(parse_name("Build-Order").expect("valid"), "build-order");
        assert!(parse_name("two words").is_err());
        assert!(parse_name("faq!").is_err());
        assert!(parse_name("").is_err());
        assert!(parse_name(&"a".repeat(MAX_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn tag_embed_stays_within_discord_limits() {
        let huge = "*_~`|[]>\\".repeat(500);
        let value = serde_json::to_value(format_tag_embed(&tag("faq", &huge))).expect("serialize");

        assert_eq!(value["title"], "Tag: faq");
        let description = value["description"].as_str().expect("description");
        assert!(
            description.chars().count() <= MAX_EMBED_CONTENT,
            "{description:.80}"
        );
        assert!(description.contains("\\*"), "user content is escaped");

        let footer = value["footer"]["text"].as_str().expect("footer");
        assert!(footer.contains("42"), "{footer}");
        assert!(footer.contains('7'), "{footer}");
    }

    #[test]
    fn long_tag_names_still_fit_the_title() {
        let name = "a".repeat(MAX_NAME_CHARS);
        let value = serde_json::to_value(format_tag_embed(&tag(&name, "x"))).expect("serialize");
        let title = value["title"].as_str().expect("title");
        assert!(title.chars().count() <= 256);
    }

    #[test]
    fn list_embed_handles_empty_and_long_lists() {
        let empty = serde_json::to_value(format_list_embed(&[])).expect("serialize");
        assert!(
            empty["description"]
                .as_str()
                .expect("description")
                .contains("No tags saved yet")
        );

        let tags: Vec<Tag> = (0..60)
            .map(|index| tag(&format!("tag{index}"), "answer"))
            .collect();
        let value = serde_json::to_value(format_list_embed(&tags)).expect("serialize");
        let description = value["description"].as_str().expect("description");

        assert!(description.chars().count() <= 4_096);
        assert!(description.contains("60"));
        assert!(description.contains("`tag0`"));
        assert!(
            !description.contains("`tag55`"),
            "only the first 50 are shown"
        );
        assert!(description.contains(&format!("first {MAX_LISTED}")));
    }

    #[test]
    fn subcommand_parser_reads_add_options() {
        let data = command_data(
            r#"{
                "id":"1","name":"tag","type":1,
                "options":[{"name":"add","type":1,"options":[
                    {"name":"name","type":3,"value":"Build Order"},
                    {"name":"content","type":3,"value":"Start with the flag."}
                ]}]
            }"#,
        );

        let (subcommand, options) = opts::subcommand(&data).expect("subcommand");
        assert_eq!(subcommand, "add");
        assert_eq!(opts::opt_str(options, "name"), Some("Build Order"));
        assert_eq!(
            opts::opt_str(options, "content"),
            Some("Start with the flag.")
        );
    }

    #[test]
    fn command_definition_is_well_formed() {
        let value = serde_json::to_value(commands()).expect("serialize");
        let command = &value[0];
        assert_eq!(command["name"], "tag");
        assert!(
            command["description"]
                .as_str()
                .expect("description")
                .chars()
                .count()
                <= 100
        );

        let options = command["options"].as_array().expect("options");
        let names: Vec<&str> = options
            .iter()
            .map(|option| option["name"].as_str().expect("option name"))
            .collect();
        assert_eq!(names, vec!["get", "list", "add", "remove"]);

        for option in options {
            let description = option["description"].as_str().expect("description");
            assert!(
                !description.is_empty() && description.chars().count() <= 100,
                "bad description for {}",
                option["name"]
            );
            for sub in option["options"].as_array().cloned().unwrap_or_default() {
                let description = sub["description"].as_str().expect("sub description");
                assert!(
                    !description.is_empty() && description.chars().count() <= 100,
                    "bad sub-option description for {}",
                    sub["name"]
                );
            }
        }
    }
}
