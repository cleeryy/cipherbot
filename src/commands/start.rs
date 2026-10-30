//! `/start` — interactive feature hub (owned by the start lane).
//!
//! The hub is stateless: every panel is addressed by its `start:<slug>`
//! custom id, so a click never depends on stored session data. Staff-only
//! panels re-check the clicker's permissions on every click (administrators
//! bypass the check). Button visibility is cosmetic; the check is
//! authoritative.

use serenity::all::{
    ButtonStyle, Colour, CommandInteraction, ComponentInteraction, Context, CreateActionRow,
    CreateAllowedMentions, CreateButton, CreateCommand, CreateEmbed, CreateInteractionResponse,
    CreateInteractionResponseMessage, Permissions, ReactionType,
};

use crate::bot::{BotResult, SharedState};
use crate::util::embeds;

const GITHUB_URL: &str = "https://github.com/cleeryy/cipherbot";

/// One navigable panel of the hub. `Close` is a terminal action, not a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Home,
    Mod,
    Ctf,
    Intel,
    Tools,
    Learn,
    Links,
    Profile,
    Recon,
    Poll,
    Tag,
    Ticket,
    Config,
    Help,
    Close,
}

impl Panel {
    pub const ALL: &'static [Panel] = &[
        Panel::Home,
        Panel::Mod,
        Panel::Ctf,
        Panel::Intel,
        Panel::Tools,
        Panel::Learn,
        Panel::Links,
        Panel::Profile,
        Panel::Recon,
        Panel::Poll,
        Panel::Tag,
        Panel::Ticket,
        Panel::Config,
        Panel::Help,
        Panel::Close,
    ];

    /// Full `custom_id` sent to Discord (≤100 chars).
    pub const fn custom_id(self) -> &'static str {
        match self {
            Panel::Home => "start:home",
            Panel::Mod => "start:mod",
            Panel::Ctf => "start:ctf",
            Panel::Intel => "start:intel",
            Panel::Tools => "start:tools",
            Panel::Learn => "start:learn",
            Panel::Links => "start:links",
            Panel::Profile => "start:profile",
            Panel::Recon => "start:recon",
            Panel::Poll => "start:poll",
            Panel::Tag => "start:tag",
            Panel::Ticket => "start:ticket",
            Panel::Config => "start:config",
            Panel::Help => "start:help",
            Panel::Close => "start:close",
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Panel::Home => "home",
            Panel::Mod => "mod",
            Panel::Ctf => "ctf",
            Panel::Intel => "intel",
            Panel::Tools => "tools",
            Panel::Learn => "learn",
            Panel::Links => "links",
            Panel::Profile => "profile",
            Panel::Recon => "recon",
            Panel::Poll => "poll",
            Panel::Tag => "tag",
            Panel::Ticket => "ticket",
            Panel::Config => "config",
            Panel::Help => "help",
            Panel::Close => "close",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Panel::Home => "Accueil",
            Panel::Mod => "Modération",
            Panel::Ctf => "CTF",
            Panel::Intel => "Renseignement",
            Panel::Tools => "Outils",
            Panel::Learn => "Apprendre",
            Panel::Links => "Liens",
            Panel::Profile => "Profil",
            Panel::Recon => "Recon",
            Panel::Poll => "Sondages",
            Panel::Tag => "Tags",
            Panel::Ticket => "Tickets",
            Panel::Config => "Config",
            Panel::Help => "Aide",
            Panel::Close => "Fermer",
        }
    }

    const fn emoji(self) -> &'static str {
        match self {
            Panel::Home => "🏠",
            Panel::Mod => "🛡️",
            Panel::Ctf => "🚩",
            Panel::Intel => "🔎",
            Panel::Tools => "🧰",
            Panel::Learn => "🎓",
            Panel::Links => "🔗",
            Panel::Profile => "👤",
            Panel::Recon => "🛰️",
            Panel::Poll => "📊",
            Panel::Tag => "🏷️",
            Panel::Ticket => "🎫",
            Panel::Config => "⚙️",
            Panel::Help => "❓",
            Panel::Close => "✖",
        }
    }

    fn style(self) -> ButtonStyle {
        match self {
            Panel::Home => ButtonStyle::Primary,
            Panel::Close => ButtonStyle::Danger,
            _ => ButtonStyle::Secondary,
        }
    }

    /// Staff-only surfaces. Administrators already set both flags upstream.
    const fn allowed(self, access: Access) -> bool {
        match self {
            Panel::Mod => access.staff,
            Panel::Config => access.admin,
            _ => true,
        }
    }

    /// Optional sibling shortcut shown next to "Accueil" and "Fermer".
    const fn related(self) -> Option<Panel> {
        match self {
            Panel::Mod => Some(Panel::Config),
            Panel::Ctf => Some(Panel::Learn),
            Panel::Intel => Some(Panel::Recon),
            Panel::Recon => Some(Panel::Intel),
            Panel::Tools => Some(Panel::Intel),
            Panel::Learn => Some(Panel::Ctf),
            Panel::Links => Some(Panel::Profile),
            Panel::Profile => Some(Panel::Poll),
            Panel::Poll => Some(Panel::Profile),
            Panel::Tag => Some(Panel::Help),
            Panel::Ticket => Some(Panel::Mod),
            Panel::Config => Some(Panel::Mod),
            Panel::Help => Some(Panel::Tag),
            Panel::Home | Panel::Close => None,
        }
    }
}

/// Effective access of the interacting member.
#[derive(Clone, Copy, Debug)]
struct Access {
    staff: bool,
    admin: bool,
}

impl Access {
    fn from_permissions(granted: Option<Permissions>) -> Self {
        let granted = granted.unwrap_or_else(Permissions::empty);
        let administrator = granted.contains(Permissions::ADMINISTRATOR);
        Self {
            staff: administrator || granted.contains(Permissions::MODERATE_MEMBERS),
            admin: administrator || granted.contains(Permissions::MANAGE_GUILD),
        }
    }
}

/// Resolved panel plus the permission verdict for the clicker.
pub struct PanelView {
    pub panel: Panel,
    pub allowed: bool,
}

/// Parse a `start:<slug>` custom id and decide whether the clicker may see it.
pub fn panel_view(custom_id: &str, is_staff: bool, is_admin: bool) -> Option<PanelView> {
    let panel = panel_from_id(custom_id)?;
    Some(PanelView {
        panel,
        allowed: panel.allowed(Access {
            staff: is_staff,
            admin: is_admin,
        }),
    })
}

fn panel_from_id(custom_id: &str) -> Option<Panel> {
    let slug = custom_id.strip_prefix("start:")?;
    if slug.is_empty() {
        return None;
    }
    Panel::ALL
        .iter()
        .copied()
        .find(|panel| panel.slug() == slug)
}

// ---------------------------------------------------------------- navigation --

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nav {
    Panel(Panel),
    Github,
}

/// Pure button layout: rows of at most 5, at most 20 buttons in total.
fn panel_layout(panel: Panel) -> Vec<Vec<Nav>> {
    match panel {
        Panel::Home => vec![
            vec![
                Nav::Panel(Panel::Mod),
                Nav::Panel(Panel::Ctf),
                Nav::Panel(Panel::Intel),
                Nav::Panel(Panel::Recon),
                Nav::Panel(Panel::Tools),
            ],
            vec![
                Nav::Panel(Panel::Learn),
                Nav::Panel(Panel::Links),
                Nav::Panel(Panel::Profile),
                Nav::Panel(Panel::Poll),
                Nav::Panel(Panel::Tag),
            ],
            vec![
                Nav::Panel(Panel::Ticket),
                Nav::Panel(Panel::Config),
                Nav::Panel(Panel::Help),
                Nav::Github,
            ],
        ],
        other => {
            let mut row = vec![Nav::Panel(Panel::Home)];
            if let Some(related) = other.related() {
                row.push(Nav::Panel(related));
            }
            row.push(Nav::Panel(Panel::Close));
            vec![row]
        }
    }
}

fn nav_button(nav: Nav) -> CreateButton {
    match nav {
        Nav::Panel(panel) => CreateButton::new(panel.custom_id())
            .label(panel.label())
            .emoji(ReactionType::Unicode(panel.emoji().to_owned()))
            .style(panel.style()),
        Nav::Github => CreateButton::new_link(GITHUB_URL)
            .label("GitHub")
            .emoji(ReactionType::Unicode("🐙".to_owned())),
    }
}

fn panel_components(panel: Panel) -> Vec<CreateActionRow> {
    panel_layout(panel)
        .into_iter()
        .map(|row| CreateActionRow::Buttons(row.into_iter().map(nav_button).collect()))
        .collect()
}

// ------------------------------------------------------------------- content --

struct PanelContent {
    title: &'static str,
    description: String,
    colour: Colour,
    fields: Vec<(&'static str, String, bool)>,
}

fn panel_embed(panel: Panel, access: Access) -> CreateEmbed {
    let content = panel_content(panel, access);
    let mut embed = CreateEmbed::new()
        .title(content.title)
        .description(content.description)
        .colour(content.colour);
    for (name, value, inline) in content.fields {
        embed = embed.field(name, value, inline);
    }
    embeds::footer(embed)
}

fn panel_content(panel: Panel, access: Access) -> PanelContent {
    match panel {
        Panel::Home => {
            let mut fields = vec![
                (
                    "🎓 Apprendre",
                    "Quiz quotidien, glossaire, roadmap et ressources triées.".to_owned(),
                    true,
                ),
                (
                    "🚩 Compétition",
                    "CTF maison : flags hachés, first blood et XP.".to_owned(),
                    true,
                ),
                (
                    "🔎 Renseignement",
                    "CVE, MITRE ATT&CK, IP, domaines et hashes.".to_owned(),
                    true,
                ),
            ];
            if access.staff {
                fields.push((
                    "🛡️ Staff",
                    if access.admin {
                        "Modération et configuration débloquées."
                    } else {
                        "Panneau modération débloqué."
                    }
                    .to_owned(),
                    false,
                ));
            }
            PanelContent {
                title: "🕸️ CipherBot — ton QG cybersécurité",
                description: "Quiz quotidien, CTF, renseignement, outils, recon passif, sondages et support : **tout au même endroit**.\n\n\
                              **Démarrage rapide**\n\
                              `/help` · `/learn quiz` · `/intel cve id:CVE-2024-3094` · `/ctf scoreboard`"
                    .to_owned(),
                colour: embeds::BRAND,
                fields,
            }
        }
        Panel::Mod => PanelContent {
            title: "🛡️ Modération",
            description: "Sanctions, dossiers et sécurité du serveur. Chaque action est consignée dans le salon modlog."
                .to_owned(),
            colour: embeds::RED,
            fields: vec![
                (
                    "🛠️ Actions",
                    "`/mod warn user:@membre reason:spam`\n\
                     `/mod timeout user:@membre duration:10m reason:…`\n\
                     `/mod kick user:@membre reason:…`\n\
                     `/mod ban user:@membre reason:…`\n\
                     `/mod purge count:50`"
                        .to_owned(),
                    false,
                ),
                (
                    "🗂️ Suivi",
                    "`/mod case view id:42` · `/mod reports status:open`\n\
                     `/mod evidence limit:25` · `/mod quarantine user:@membre`\n\
                     `/mod lock` / `/mod unlock` · `/mod verify-panel role:@vérifié`"
                        .to_owned(),
                    false,
                ),
                (
                    "🔐 Accès",
                    "Réservé au staff : permission **Modérer les membres**. Les administrateurs passent outre."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Ctf => PanelContent {
            title: "🚩 Capture The Flag",
            description: "Crée des challenges, soumets tes flags et grimpe au classement, sans quitter Discord."
                .to_owned(),
            colour: embeds::AMBER,
            fields: vec![
                (
                    "🚩 Commandes",
                    "`/ctf create name:SQLi 101 category:web points:100 flag:flag{…}`\n\
                     `/ctf list` · `/ctf info name:SQLi 101`\n\
                     `/ctf solve name:SQLi 101 flag:flag{…}`\n\
                     `/ctf scoreboard` · `/ctf delete name:SQLi 101`"
                        .to_owned(),
                    false,
                ),
                (
                    "⚙️ Fonctionnement",
                    "Les flags sont stockés **hachés** et ne sont jamais réaffichés. Le premier solveur (first blood) gagne un bonus ; chaque solve crédite de l'XP au classement.\n\
                     Création et suppression : staff uniquement."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Intel => PanelContent {
            title: "🔎 Renseignement",
            description: "CVE, techniques MITRE ATT&CK, IP, domaines et hashes — à la demande."
                .to_owned(),
            colour: embeds::BLUE,
            fields: vec![
                (
                    "🧨 CVE & ATT&CK",
                    "`/intel cve id:CVE-2024-3094`\n\
                     `/intel cve-latest severity:critical limit:5`\n\
                     `/intel cve-search query:log4j`\n\
                     `/intel mitre query:T1059`"
                        .to_owned(),
                    false,
                ),
                (
                    "🌐 Réseau & fichiers",
                    "`/intel ip address:1.1.1.1`\n\
                     `/intel domain name:example.com`\n\
                     `/intel hash value:<sha256>`\n\
                     `/intel vt query:… type:file`"
                        .to_owned(),
                    false,
                ),
                (
                    "⏱️ Bon à savoir",
                    "Certaines requêtes partagent un cooldown anti-abus de quelques secondes."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Tools => PanelContent {
            title: "🧰 Boîte à outils",
            description: "Encodage, hashing, CVSS et tradecraft : les petits outils du quotidien."
                .to_owned(),
            colour: embeds::BRAND,
            fields: vec![
                (
                    "🧪 Transformation",
                    "`/tool encode scheme:base64 input:hello`\n\
                     `/tool decode scheme:hex input:…`\n\
                     `/tool hashid value:…` · `/tool hash value:…`\n\
                     `/tool entropy value:…` · `/tool cvss vector:CVSS:3.1/AV:N/…`"
                        .to_owned(),
                    false,
                ),
                (
                    "🕵️ Tradecraft",
                    "`/tool password value:…` (éphémère)\n\
                     `/tool dork target:example.com`\n\
                     `/tool jwt token:…` (éphémère)\n\
                     `/tool ipcalc` · `/tool timestamp`"
                        .to_owned(),
                    false,
                ),
                (
                    "🔒 Confidentialité",
                    "`password` et `jwt` répondent en éphémère : rien n'est conservé."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Learn => PanelContent {
            title: "🎓 Apprendre",
            description: "Progresse un peu chaque jour : quiz, glossaire, roadmap, ressources et astuce du jour."
                .to_owned(),
            colour: embeds::BLUE,
            fields: vec![
                (
                    "📘 Commandes",
                    "`/learn quiz` · `/learn quiz category:web`\n\
                     `/learn glossary term:phishing`\n\
                     `/learn roadmap track:blue`\n\
                     `/learn resources kind:lab search:…`\n\
                     `/learn tip`"
                        .to_owned(),
                    false,
                ),
                (
                    "📈 Progression",
                    "`/profile` : XP, précision au quiz, points CTF.\n\
                     `/leaderboard metric:xp` : classement du serveur."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Links => PanelContent {
            title: "🔗 Archive de liens",
            description: "La mémoire collective du serveur : chaque lien partagé est archivé et retrouvable."
                .to_owned(),
            colour: embeds::BRAND,
            fields: vec![
                (
                    "🔍 Commandes",
                    "`/links search query:reverse engineering`\n\
                     `/links recent limit:10`\n\
                     `/links stats`"
                        .to_owned(),
                    false,
                ),
                (
                    "🧵 Fonctionnement",
                    "Les doublons sont ignorés. Un thread peut être ouvert automatiquement (`auto_thread`), et les messages forwards expirent après `ttl_hours` s'il est configuré.\n\
                     Réglage : staff, via `/config links`."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Profile => PanelContent {
            title: "👤 Profil & réputation",
            description: "Ton identité cyber : XP, précision au quiz, points CTF."
                .to_owned(),
            colour: embeds::AMBER,
            fields: vec![
                (
                    "👤 Commandes",
                    "`/profile` · `/profile user:@membre`\n\
                     `/leaderboard metric:xp limit:10`\n\
                     `/leaderboard metric:quiz` · `metric:ctf`"
                        .to_owned(),
                    false,
                ),
                (
                    "📨 Contribuer",
                    "`/report user:@membre reason:…` — signalement privé au staff (ID de message optionnel).\n\
                     `/suggest text:…` — proposition envoyée au salon des suggestions."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Recon => PanelContent {
            title: "🛰️ Recon passif",
            description: "OSINT passif : sous-domaines, ports et CVE, archives web et scans publics — sans jamais toucher la cible."
                .to_owned(),
            colour: embeds::BLUE,
            fields: vec![
                (
                    "🛰️ Commandes",
                    "`/recon subdomains domain:example.com`\n\
                     `/recon ip address:1.1.1.1`\n\
                     `/recon wayback domain:example.com`\n\
                     `/recon urlscan domain:example.com`"
                        .to_owned(),
                    false,
                ),
                (
                    "🛡️ Éthique",
                    "**Passif uniquement : aucun paquet n'est envoyé à la cible.** Sources publiques : crt.sh, Shodan InternetDB, Wayback Machine, urlscan.io. Cooldown anti-abus."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Poll => PanelContent {
            title: "📊 Sondages",
            description: "Lance un vote en un clic : boutons, barres en direct et clôture par l'auteur ou le staff."
                .to_owned(),
            colour: embeds::AMBER,
            fields: vec![
                (
                    "📊 Commandes",
                    "`/poll create question:\"…\" option1:\"…\" option2:\"…\"`\n\
                     `/poll close id:…`\n\
                     `/poll results id:…`"
                        .to_owned(),
                    false,
                ),
                (
                    "🗳️ Fonctionnement",
                    "Jusqu'à 5 options. Chaque vote met à jour les barres en direct ; un seul vote par membre, modifiable. L'auteur ou le staff peut clôturer."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Tag => PanelContent {
            title: "🏷️ Tags",
            description: "Des réponses et snippets réutilisables, accessibles à tous en une commande."
                .to_owned(),
            colour: embeds::BRAND,
            fields: vec![
                (
                    "🏷️ Commandes",
                    "`/tag get name:…`\n\
                     `/tag list`\n\
                     `/tag add name:… content:\"…\"`\n\
                     `/tag remove name:…`"
                        .to_owned(),
                    false,
                ),
                (
                    "🔐 Accès",
                    "`add` et `remove` demandent la permission **Gérer les messages** ; la consultation est ouverte à tous."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Ticket => PanelContent {
            title: "🎫 Support",
            description: "Un salon d'aide privé par demande, ouvert d'un clic et fermé proprement."
                .to_owned(),
            colour: embeds::BLUE,
            fields: vec![
                (
                    "🎫 Commandes",
                    "`/ticket panel channel:#support role:@Staff`\n\
                     `/ticket list`\n\
                     `/ticket close`"
                        .to_owned(),
                    false,
                ),
                (
                    "🧵 Fonctionnement",
                    "Le bouton du panneau crée un **thread privé** avec le membre et le rôle staff. La fermeture archive le thread ; `list` montre la file d'attente. Panneau : staff (**Gérer le serveur**)."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Config => PanelContent {
            title: "⚙️ Configuration",
            description: "Réglages du serveur : journaux, salons, automod, vérification et archive de liens."
                .to_owned(),
            colour: embeds::RED,
            fields: vec![
                (
                    "⚙️ Commandes",
                    "`/config view`\n\
                     `/config modlog channel:#mod-log`\n\
                     `/config ctf-channel channel:#ctf`\n\
                     `/config automod feature:invites enabled:true`\n\
                     `/config verify role:@vérifié`\n\
                     `/config set key:… value:…`\n\
                     `/config links auto_thread:true ttl_hours:24`"
                        .to_owned(),
                    false,
                ),
                (
                    "🔐 Accès",
                    "Réservé aux administrateurs : permission **Gérer le serveur**."
                        .to_owned(),
                    false,
                ),
            ],
        },
        Panel::Help => PanelContent {
            title: "❓ Aide",
            description: help_description(),
            colour: embeds::BLUE,
            fields: vec![(
                "💡 Astuce",
                "Tape `/` puis le nom d'une commande : Discord affiche ses options et leurs valeurs possibles."
                    .to_owned(),
                false,
            )],
        },
        Panel::Close => PanelContent {
            title: "👋 À bientôt",
            description: "Guide fermé. Relance `/start` quand tu veux.".to_owned(),
            colour: embeds::BRAND,
            fields: Vec::new(),
        },
    }
}

/// Every top-level command, with `/start` added as a fallback if the
/// registry's help table lags behind the command surface.
fn help_description() -> String {
    let mut entries = crate::commands::help_data();
    if !entries.iter().any(|(name, _)| *name == "start") {
        entries.push(("start", "Open the interactive CipherBot guide"));
    }
    entries.sort_by(|left, right| left.0.cmp(right.0));

    let mut text = String::from("**Toutes les commandes**\n");
    for (name, description) in entries {
        text.push_str(&format!("**/{name}** — {description}\n"));
    }
    text.push_str("\nAstuce : `/help command:mod` pour le détail d'une commande.");
    text
}

// -------------------------------------------------------------------- handlers --

pub fn commands() -> Vec<CreateCommand> {
    vec![CreateCommand::new("start").description("Open the interactive CipherBot guide")]
}

pub async fn handle(
    _state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
) -> BotResult<()> {
    let access =
        Access::from_permissions(cmd.member.as_ref().and_then(|member| member.permissions));

    cmd.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .embed(panel_embed(Panel::Home, access))
                .components(panel_components(Panel::Home))
                .allowed_mentions(CreateAllowedMentions::new())
                .ephemeral(true),
        ),
    )
    .await?;
    Ok(())
}

pub async fn handle_component(
    _state: &SharedState,
    ctx: &Context,
    ci: &ComponentInteraction,
) -> BotResult<bool> {
    let access = Access::from_permissions(ci.member.as_ref().and_then(|member| member.permissions));

    let Some(view) = panel_view(&ci.data.custom_id, access.staff, access.admin) else {
        return Ok(false);
    };

    if !view.allowed {
        ci.create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(denial_message(view.panel))
                    .allowed_mentions(CreateAllowedMentions::new())
                    .ephemeral(true),
            ),
        )
        .await?;
        return Ok(true);
    }

    if view.panel == Panel::Close {
        close(ctx, ci).await?;
        return Ok(true);
    }

    ci.create_response(
        &ctx.http,
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .embed(panel_embed(view.panel, access))
                .components(panel_components(view.panel))
                .allowed_mentions(CreateAllowedMentions::new()),
        ),
    )
    .await?;
    Ok(true)
}

fn denial_message(panel: Panel) -> &'static str {
    match panel {
        Panel::Config => "🔒 Accès réservé aux administrateurs (`Gérer le serveur`).",
        _ => "🔒 Accès réservé au staff (`Modérer les membres`).",
    }
}

/// Delete the ephemeral hub message; if Discord refuses, replace it with a
/// small farewell so the interaction is still acknowledged.
async fn close(ctx: &Context, ci: &ComponentInteraction) -> BotResult<()> {
    match ci.delete_response(&ctx.http).await {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::warn!("Failed to delete /start response: {error}");
            ci.create_response(
                &ctx.http,
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new()
                        .embed(embeds::info(
                            "👋 Guide fermé",
                            "Relance `/start` quand tu veux.",
                        ))
                        .components(Vec::new())
                        .allowed_mentions(CreateAllowedMentions::new()),
                ),
            )
            .await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twenty top-level commands as of this change; the help panel must list
    /// every one of them.
    const EXPECTED_COMMANDS: [&str; 20] = [
        "about",
        "config",
        "ctf",
        "help",
        "intel",
        "leaderboard",
        "learn",
        "links",
        "mod",
        "ping",
        "poll",
        "profile",
        "recon",
        "report",
        "start",
        "stats",
        "suggest",
        "tag",
        "ticket",
        "tool",
    ];

    #[test]
    fn panel_ids_round_trip() {
        for panel in Panel::ALL {
            let id = panel.custom_id();
            assert!(id.starts_with("start:"), "{id}");
            assert!(id.len() <= 100, "{id} is too long");
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ':' || c == '-'),
                "{id} has invalid characters"
            );
            let view = panel_view(id, true, true).expect("known id");
            assert_eq!(view.panel, *panel);
            assert!(view.allowed, "staff should see {id}");
        }
    }

    #[test]
    fn unknown_custom_ids_are_rejected() {
        for id in [
            "",
            "start",
            "start:",
            "start::",
            "start:home:extra",
            "start:unknown",
            "other:home",
            "start:START",
        ] {
            assert!(
                panel_view(id, true, true).is_none(),
                "{id} should be unknown"
            );
        }
    }

    #[test]
    fn layouts_are_compact_and_navigable() {
        for panel in Panel::ALL {
            let layout = panel_layout(*panel);
            assert!(!layout.is_empty(), "{panel:?} has no rows");
            assert!(layout.len() <= 5, "{panel:?} has too many rows");
            let total: usize = layout.iter().map(Vec::len).sum();
            assert!(total <= 20, "{panel:?} has {total} buttons");
            for row in &layout {
                assert!(!row.is_empty() && row.len() <= 5, "{panel:?} bad row");
            }

            if *panel != Panel::Home {
                let has = |target: Panel| {
                    layout
                        .iter()
                        .flatten()
                        .any(|nav| *nav == Nav::Panel(target))
                };
                assert!(has(Panel::Home), "{panel:?} lacks an Accueil button");
                assert!(has(Panel::Close), "{panel:?} lacks a Fermer button");
            }
        }
    }

    #[test]
    fn new_feature_panels_are_reachable() {
        let home = panel_layout(Panel::Home);
        for target in [Panel::Recon, Panel::Poll, Panel::Tag, Panel::Ticket] {
            assert!(
                home.iter().flatten().any(|nav| *nav == Nav::Panel(target)),
                "{target:?} is not reachable from the home panel"
            );
            let layout = panel_layout(target);
            let has = |wanted: Panel| {
                layout
                    .iter()
                    .flatten()
                    .any(|nav| *nav == Nav::Panel(wanted))
            };
            assert!(has(Panel::Home), "{target:?} lacks an Accueil button");
            assert!(has(Panel::Close), "{target:?} lacks a Fermer button");
            let view = panel_view(target.custom_id(), false, false).expect("new panel id");
            assert!(view.allowed, "{target:?} must be open to everyone");
        }
    }

    #[test]
    fn staff_panels_are_gated() {
        let home = panel_view("start:home", false, false).expect("home");
        assert!(home.allowed);

        let mod_panel = panel_view("start:mod", false, false).expect("mod");
        assert!(!mod_panel.allowed);
        assert!(panel_view("start:mod", true, false).expect("mod").allowed);

        let config = panel_view("start:config", true, false).expect("config");
        assert!(!config.allowed, "moderators must not see config");
        assert!(
            panel_view("start:config", false, true)
                .expect("config")
                .allowed
        );
    }

    #[test]
    fn content_stays_within_discord_limits() {
        let access = Access {
            staff: true,
            admin: true,
        };
        for panel in Panel::ALL {
            let content = panel_content(*panel, access);
            assert!(content.title.chars().count() <= 256, "{panel:?} title");
            assert!(!content.description.is_empty(), "{panel:?} description");
            assert!(
                content.description.chars().count() <= 1_500,
                "{panel:?} description is too long"
            );
            assert!(content.fields.len() <= 25, "{panel:?} has too many fields");
            for (name, value, _) in &content.fields {
                assert!(name.chars().count() <= 256, "{panel:?}/{name}");
                assert!(!value.is_empty(), "{panel:?}/{name} is empty");
                assert!(
                    value.chars().count() <= 1_024,
                    "{panel:?}/{name} is too long"
                );
            }
        }
    }

    #[test]
    fn help_lists_every_top_level_command() {
        let text = help_description();
        let registered: Vec<&str> = crate::commands::help_data()
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for name in EXPECTED_COMMANDS {
            assert!(
                registered.contains(&name),
                "/{name} is missing from the registry help table"
            );
            assert!(
                text.contains(&format!("**/{name}**")),
                "help panel is missing /{name}"
            );
        }
        assert!(
            text.chars().count() <= 1_500,
            "help description is too long"
        );
    }
}
