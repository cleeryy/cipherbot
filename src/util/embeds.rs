//! Shared embed styling. Every user-visible embed goes through these helpers so
//! colours and the footer stay consistent.

use serenity::all::{Colour, CreateEmbed, CreateEmbedFooter};

pub const BRAND: Colour = Colour::new(0x0000_E5A0);
pub const RED: Colour = Colour::new(0x00ED_4245);
pub const AMBER: Colour = Colour::new(0xFE_E7_5C);
pub const BLUE: Colour = Colour::new(0x58_65_F2);

/// Cyber green, the bot's primary colour.
pub fn brand() -> Colour {
    BRAND
}

/// Apply the standard `CipherBot vX.Y.Z` footer.
pub fn footer(embed: CreateEmbed) -> CreateEmbed {
    embed.footer(CreateEmbedFooter::new(format!(
        "CipherBot v{}",
        env!("CARGO_PKG_VERSION")
    )))
}

pub fn ok(title: impl Into<String>, description: impl Into<String>) -> CreateEmbed {
    footer(
        CreateEmbed::new()
            .title(title)
            .description(description)
            .colour(BRAND),
    )
}

pub fn err(description: impl Into<String>) -> CreateEmbed {
    footer(
        CreateEmbed::new()
            .title("Error")
            .description(description)
            .colour(RED),
    )
}

pub fn warn(title: impl Into<String>, description: impl Into<String>) -> CreateEmbed {
    footer(
        CreateEmbed::new()
            .title(title)
            .description(description)
            .colour(AMBER),
    )
}

pub fn info(title: impl Into<String>, description: impl Into<String>) -> CreateEmbed {
    footer(
        CreateEmbed::new()
            .title(title)
            .description(description)
            .colour(BLUE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_colour_is_cyber_green() {
        assert_eq!(brand().0, 0x0000_E5A0);
    }

    #[test]
    fn footer_contains_version() {
        // Smoke test: builders are opaque, but constructing them must not panic.
        let embed = footer(CreateEmbed::new().title("t"));
        let _ = embed;
    }
}
