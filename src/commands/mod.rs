//! Slash-command registry and dispatch.
//!
//! Each feature lane owns one module exposing `commands()` and `handle()`.
//! `all_commands()` is the single source of truth for registration; `dispatch`
//! routes by top-level name. Core-only commands (`help`, `stats`, `ping`,
//! `about`) live here because they have no lane.

pub mod community;
pub mod ctf;
pub mod intel;
pub mod learning;
pub mod links;
pub mod moderation;
pub mod toolbox;

use serenity::all::{
    CommandInteraction, CommandOptionType, Context, CreateCommand, CreateCommandOption,
};

use crate::bot::{BotResult, SharedState};
use crate::util::{embeds, opts, respond, time};

/// Every top-level command with a one-line description (used by `/help`).
const HELP_TABLE: &[(&str, &str)] = &[
    ("about", "About CipherBot"),
    ("config", "View or change server configuration"),
    ("ctf", "Create, solve and score CTF challenges"),
    ("help", "List commands or show details for one"),
    ("intel", "CVE, ATT&CK, IP, domain and hash intel"),
    ("leaderboard", "Server leaderboards"),
    ("learn", "Quiz, glossary, roadmap and resources"),
    ("links", "Browse the community link archive"),
    ("mod", "Moderation: warn, timeout, kick, ban, purge, cases"),
    ("ping", "Check that the bot is alive"),
    ("profile", "Show a member profile"),
    ("report", "Report a member or message to the staff"),
    ("stats", "Bot statistics"),
    ("suggest", "Submit a server suggestion"),
    ("tool", "Encoding, hashing, CVSS and tradecraft tools"),
];

pub fn help_data() -> Vec<(&'static str, &'static str)> {
    HELP_TABLE.to_vec()
}

/// Concatenation of every lane's command set.
pub fn all_commands() -> Vec<CreateCommand> {
    let mut commands = Vec::new();
    commands.extend(moderation::commands());
    commands.extend(ctf::commands());
    commands.extend(intel::commands());
    commands.extend(toolbox::commands());
    commands.extend(learning::commands());
    commands.extend(community::commands());
    commands.extend(links::commands());
    commands.extend(core_commands());
    commands
}

/// Commands implemented directly in this module (no feature lane).
fn core_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("help")
            .description("List commands or show details for one")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "command",
                    "Command name, e.g. mod",
                )
                .max_length(32),
            ),
        CreateCommand::new("stats").description("Bot statistics"),
        CreateCommand::new("ping").description("Check that the bot is alive"),
        CreateCommand::new("about").description("About CipherBot"),
    ]
}

pub async fn dispatch(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
) -> BotResult<()> {
    state.metrics.inc_commands();

    match cmd.data.name.as_str() {
        "mod" => moderation::handle(state, ctx, cmd).await,
        "ctf" => ctf::handle(state, ctx, cmd).await,
        "intel" => intel::handle(state, ctx, cmd).await,
        "tool" => toolbox::handle(state, ctx, cmd).await,
        "learn" => learning::handle(state, ctx, cmd).await,
        "links" => links::handle(state, ctx, cmd).await,
        "profile" | "leaderboard" | "report" | "suggest" | "config" => {
            community::handle(state, ctx, cmd).await
        }
        "help" => help(ctx, cmd).await,
        "stats" => stats(state, ctx, cmd).await,
        "ping" => ping(state, ctx, cmd).await,
        "about" => about(state, ctx, cmd).await,
        other => respond::error(ctx, cmd, format!("Unknown command `{other}`.")).await,
    }
}

async fn help(ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let table = help_data();
    if let Some(requested) = opts::opt_str(&cmd.data.options, "command") {
        let name = requested.trim().trim_start_matches('/');
        let Some((entry_name, description)) =
            table.iter().find(|(entry_name, _)| *entry_name == name)
        else {
            return respond::error(ctx, cmd, format!("Unknown command `/{name}`.")).await;
        };
        let details = format!(
            "**/{entry_name}** — {description}\n\n\
             Tip: Discord shows the full option list as soon as you type `/{entry_name} `."
        );
        return respond::ephemeral(ctx, cmd, embeds::info(format!("/{entry_name}"), details)).await;
    }

    let description = table
        .iter()
        .map(|(name, text)| format!("**/{name}** — {text}"))
        .collect::<Vec<_>>()
        .join("\n");
    respond::public(ctx, cmd, embeds::info("CipherBot commands", description)).await
}

async fn stats(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let uptime = time::human_duration(state.started_at.elapsed().as_secs() as i64);
    let tracked = state
        .db
        .tracked_message_count()
        .map(|count| count.to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let db_ok = if state.db.health_check() {
        "ok"
    } else {
        "degraded"
    };
    let configured_guilds = state
        .db
        .all_guild_ids()
        .map(|ids| ids.len().to_string())
        .unwrap_or_else(|_| "unknown".to_owned());

    let description = format!(
        "**Uptime:** {uptime}\n\
         **Guilds:** {}\n\
         **Configured guilds:** {configured_guilds}\n\
         **Commands served:** {}\n\
         **Messages seen:** {}\n\
         **Automod actions:** {}\n\
         **Tracked messages:** {tracked}\n\
         **Phishing domains loaded:** {}\n\
         **Handler errors:** {}\n\
         **Database:** {db_ok}\n\
         **Version:** {}",
        state.metrics.guilds(),
        state.metrics.commands_total(),
        state.metrics.messages_total(),
        state.metrics.automod_actions(),
        state.phishing.len(),
        state.metrics.errors(),
        env!("CARGO_PKG_VERSION"),
    );

    respond::public(ctx, cmd, embeds::ok("Statistics", description)).await
}

async fn ping(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let uptime = time::human_duration(state.started_at.elapsed().as_secs() as i64);
    respond::text(ctx, cmd, format!("Pong! Uptime: {uptime}"), true).await
}

async fn about(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let description = format!(
        "CipherBot v{} — the operating system of a cybersecurity community.\n\
         Guilds: {}\n\
         Source: https://github.com/cleeryy/cipherbot",
        env!("CARGO_PKG_VERSION"),
        state.metrics.guilds(),
    );
    respond::public(ctx, cmd, embeds::info("About CipherBot", description)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use serde_json::Value;
    use std::collections::HashSet;

    #[test]
    fn command_definitions_pass_discord_lint() {
        let name_re = Regex::new(r"^[a-z0-9_-]{1,32}$").expect("regex");
        let commands = all_commands();
        let mut seen = HashSet::new();

        for command in &commands {
            let value = serde_json::to_value(command).expect("command serializes");
            let name = value["name"].as_str().expect("name");
            assert!(name_re.is_match(name), "bad command name: {name}");
            assert!(seen.insert(name.to_owned()), "duplicate command: {name}");

            let description = value["description"].as_str().expect("description");
            assert!(
                !description.is_empty() && description.chars().count() <= 100,
                "bad description for {name}"
            );

            let options = value["options"].as_array().cloned().unwrap_or_default();
            assert!(options.len() <= 25, "too many options for {name}");

            let mut option_names = HashSet::new();
            for option in &options {
                let option_name = option["name"].as_str().expect("option name");
                assert!(
                    name_re.is_match(option_name),
                    "bad option name {option_name} in {name}"
                );
                assert!(
                    option_names.insert(option_name.to_owned()),
                    "duplicate option {option_name} in {name}"
                );
                let option_description = option["description"].as_str().unwrap_or_default();
                assert!(
                    !option_description.is_empty() && option_description.chars().count() <= 100,
                    "bad option description for {name}/{option_name}"
                );
            }
        }
    }

    #[test]
    fn help_table_covers_every_top_level_name() {
        let names: HashSet<&str> = help_data().iter().map(|(name, _)| *name).collect();
        for expected in [
            "mod",
            "ctf",
            "intel",
            "tool",
            "learn",
            "links",
            "profile",
            "leaderboard",
            "report",
            "suggest",
            "config",
            "help",
            "stats",
            "ping",
            "about",
        ] {
            assert!(names.contains(expected), "help is missing /{expected}");
        }
        assert_eq!(names.len(), help_data().len(), "duplicate help entry");
    }

    #[test]
    fn all_commands_includes_links() {
        let value = all_commands()
            .iter()
            .map(|command| serde_json::to_value(command).expect("serialize"))
            .collect::<Vec<Value>>();
        assert!(value.iter().any(|command| command["name"] == "links"));
    }

    #[test]
    fn every_help_entry_is_registered() {
        let registered: HashSet<String> = all_commands()
            .iter()
            .map(|command| {
                serde_json::to_value(command).expect("serialize")["name"]
                    .as_str()
                    .expect("name")
                    .to_owned()
            })
            .collect();

        for (name, _) in help_data() {
            assert!(
                registered.contains(name),
                "/{name} is documented but never registered"
            );
        }
        assert_eq!(
            registered.len(),
            help_data().len(),
            "registered commands and help entries diverged"
        );
    }
}
