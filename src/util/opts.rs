//! Typed access to slash-command options.
//!
//! All helpers work on plain `CommandDataOption` slices so feature modules can
//! share them for subcommands and subcommand groups alike.

use serenity::all::{
    ChannelId, CommandData, CommandDataOption, CommandDataOptionValue, RoleId, UserId,
};

use crate::bot::BotResult;
use crate::util::time::parse_duration;

/// Locate a subcommand and its options.
///
/// Supports both `SubCommand` and `SubCommandGroup` payloads. For a group the
/// first nested subcommand is returned, which is the only shape Discord sends
/// once the user has picked a group member.
pub fn subcommand(data: &CommandData) -> Option<(&str, &[CommandDataOption])> {
    match data.options.first()? {
        CommandDataOption {
            name,
            value: CommandDataOptionValue::SubCommand(options),
            ..
        } => Some((name.as_str(), options.as_slice())),
        CommandDataOption {
            value: CommandDataOptionValue::SubCommandGroup(groups),
            ..
        } => match groups.first()? {
            CommandDataOption {
                name,
                value: CommandDataOptionValue::SubCommand(options),
                ..
            } => Some((name.as_str(), options.as_slice())),
            CommandDataOption { name, .. } => Some((name.as_str(), &[])),
        },
        _ => None,
    }
}

pub fn find<'a>(opts: &'a [CommandDataOption], name: &str) -> Option<&'a CommandDataOptionValue> {
    opts.iter()
        .find(|option| option.name == name)
        .map(|option| &option.value)
}

pub fn opt_str<'a>(opts: &'a [CommandDataOption], name: &str) -> Option<&'a str> {
    find(opts, name).and_then(CommandDataOptionValue::as_str)
}

pub fn opt_i64(opts: &[CommandDataOption], name: &str) -> Option<i64> {
    find(opts, name).and_then(CommandDataOptionValue::as_i64)
}

pub fn opt_bool(opts: &[CommandDataOption], name: &str) -> Option<bool> {
    find(opts, name).and_then(CommandDataOptionValue::as_bool)
}

pub fn opt_user(opts: &[CommandDataOption], name: &str) -> Option<UserId> {
    find(opts, name).and_then(CommandDataOptionValue::as_user_id)
}

pub fn opt_channel(opts: &[CommandDataOption], name: &str) -> Option<ChannelId> {
    find(opts, name).and_then(CommandDataOptionValue::as_channel_id)
}

pub fn opt_role(opts: &[CommandDataOption], name: &str) -> Option<RoleId> {
    find(opts, name).and_then(CommandDataOptionValue::as_role_id)
}

/// Required string option; the error text is safe to show ephemerally.
pub fn req_str<'a>(opts: &'a [CommandDataOption], name: &str) -> BotResult<&'a str> {
    opt_str(opts, name).ok_or_else(|| anyhow::anyhow!("Missing required option `{name}`."))
}

/// Required duration option, returned in seconds (`10m`, `2h`, `7d`, `600`).
pub fn duration(opts: &[CommandDataOption], name: &str) -> BotResult<i64> {
    let raw = req_str(opts, name)?;
    parse_duration(raw)
        .ok_or_else(|| anyhow::anyhow!("Invalid duration `{raw}`. Use e.g. 10m, 2h, 7d."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::all::CommandDataOption;

    /// `CommandDataOption` is `#[non_exhaustive]`, so tests build options through
    /// its public serde implementation rather than struct literals.
    fn options(json: &str) -> Vec<CommandDataOption> {
        serde_json::from_str(json).expect("valid option fixture")
    }

    #[test]
    fn finds_typed_values() {
        let opts = options(
            r#"[
                {"name":"query","type":3,"value":"rust"},
                {"name":"limit","type":4,"value":25},
                {"name":"enabled","type":5,"value":true}
            ]"#,
        );
        assert_eq!(opt_str(&opts, "query"), Some("rust"));
        assert_eq!(opt_i64(&opts, "limit"), Some(25));
        assert_eq!(opt_bool(&opts, "enabled"), Some(true));
        assert_eq!(opt_str(&opts, "missing"), None);
        assert_eq!(opt_i64(&opts, "query"), None);
    }

    #[test]
    fn required_option_errors_are_readable() {
        let opts = options(r#"[{"name":"query","type":3,"value":"rust"}]"#);
        assert_eq!(req_str(&opts, "query").expect("present"), "rust");
        let error = req_str(&opts, "user").expect_err("missing");
        assert!(error.to_string().contains("user"));
    }

    #[test]
    fn duration_parses_units() {
        let opts = options(r#"[{"name":"ttl","type":3,"value":"2h"}]"#);
        assert_eq!(duration(&opts, "ttl").expect("2h"), 7_200);

        let bad = options(r#"[{"name":"ttl","type":3,"value":"nope"}]"#);
        assert!(duration(&bad, "ttl").is_err());
    }

    #[test]
    fn subcommand_extracts_options() {
        let data: CommandData = serde_json::from_str(
            r#"{
                "id":"1","name":"mod","type":1,
                "options":[{"name":"warn","type":1,"options":[
                    {"name":"user","type":6,"value":"123"}
                ]}]
            }"#,
        )
        .expect("command fixture");

        let (name, opts) = subcommand(&data).expect("subcommand");
        assert_eq!(name, "warn");
        assert_eq!(opt_user(opts, "user"), Some(UserId::new(123)));
    }
}
