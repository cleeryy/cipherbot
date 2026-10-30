//! Permission checks.
//!
//! Slash-command interaction payloads already carry channel-computed
//! permissions, so no cache lookup is needed. `default_member_permissions` on
//! the command definition is cosmetic; these checks are authoritative.

use anyhow::bail;
use serenity::all::{CommandInteraction, Permissions};

use crate::bot::BotResult;

/// Require the invoking member to hold `needed` (administrators bypass).
pub fn require(cmd: &CommandInteraction, needed: Permissions) -> BotResult<()> {
    let granted = cmd
        .member
        .as_ref()
        .and_then(|member| member.permissions)
        .unwrap_or_else(Permissions::empty);
    check(granted, needed, "You need")
}

/// Require the bot itself to hold `needed` (administrators bypass).
pub fn require_bot(cmd: &CommandInteraction, needed: Permissions) -> BotResult<()> {
    let granted = cmd.app_permissions.unwrap_or_else(Permissions::empty);
    check(granted, needed, "I need")
}

fn check(granted: Permissions, needed: Permissions, subject: &str) -> BotResult<()> {
    if granted.contains(Permissions::ADMINISTRATOR) || granted.contains(needed) {
        return Ok(());
    }
    let names = permission_names(needed);
    if names.is_empty() {
        bail!("{subject} a permission that is not configured for this bot.");
    }
    bail!("{subject} the {names} permission(s) to run this command.");
}

/// Human-readable list of permission flags, e.g. `kick members, ban members`.
fn permission_names(permissions: Permissions) -> String {
    let mut names: Vec<String> = permissions
        .iter_names()
        .map(|(name, _)| name.to_ascii_lowercase().replace('_', " "))
        .collect();
    names.sort();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_names_are_readable() {
        let names = permission_names(Permissions::KICK_MEMBERS | Permissions::BAN_MEMBERS);
        assert!(names.contains("kick members"), "{names}");
        assert!(names.contains("ban members"), "{names}");
    }

    #[test]
    fn empty_permissions_yield_empty_names() {
        assert!(permission_names(Permissions::empty()).is_empty());
    }
}
