//! `/intel` — CVE, ATT&CK, IP, domain and hash intel.
//!
//! Every subcommand acknowledges with an ephemeral placeholder, validates input
//! before any network call, then edits the placeholder with an embed. Errors are
//! always ephemeral and never carry raw API responses.

use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};

use anyhow::{Result as AnyResult, bail};
use regex::Regex;
use serenity::all::{
    CommandDataOption, CommandInteraction, CommandOptionType, Context, CreateCommand,
    CreateCommandOption, CreateEmbed,
};

use crate::bot::{BotResult, SharedState};
use crate::content;
use crate::services::{ipinfo, nvd, rdap, vt};
use crate::util::{embeds, opts, respond, text, time};

const INTEL_COOLDOWN_SECS: i64 = 4;

/// Last accepted call per user. `std::sync::Mutex`: the guard is never held
/// across an await point.
static LAST_INTEL_CALL: LazyLock<Mutex<HashMap<u64, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static CVE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(?i)CVE-\d{4}-\d{4,7}$").ok());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashKind {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl HashKind {
    const fn label(self) -> &'static str {
        match self {
            HashKind::Md5 => "MD5 / NTLM",
            HashKind::Sha1 => "SHA-1",
            HashKind::Sha224 => "SHA-224",
            HashKind::Sha256 => "SHA-256",
            HashKind::Sha384 => "SHA-384",
            HashKind::Sha512 => "SHA-512",
        }
    }
}

pub fn commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("intel")
            .description("CVE, ATT&CK, IP, domain and hash intel")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "cve",
                    "Look up a CVE by its NVD identifier",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "id",
                        "CVE ID, e.g. CVE-2024-3094",
                    )
                    .required(true)
                    .min_length(12)
                    .max_length(20),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "cve-latest",
                    "List the most recently published CVEs",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "severity",
                        "Only show this CVSS severity (default: any)",
                    )
                    .add_string_choice("critical", "critical")
                    .add_string_choice("high", "high")
                    .add_string_choice("medium", "medium")
                    .add_string_choice("low", "low")
                    .add_string_choice("any", "any"),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "limit",
                        "How many CVEs to show (1-10, default 5)",
                    )
                    .min_int_value(1)
                    .max_int_value(10),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "cve-search",
                    "Search CVE descriptions by keyword",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "query",
                        "Keyword, e.g. log4j or exchange",
                    )
                    .required(true)
                    .min_length(2)
                    .max_length(120),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "mitre",
                    "Search the embedded MITRE ATT&CK techniques",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "query",
                        "Technique ID or keyword, e.g. T1059 or phishing",
                    )
                    .required(true)
                    .max_length(100),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "ip",
                    "IP geolocation, ASN and network ownership",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "address",
                        "IPv4 or IPv6 address, e.g. 1.1.1.1",
                    )
                    .required(true)
                    .max_length(45),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "domain",
                    "RDAP registration data for a domain",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "name",
                        "Domain without scheme, e.g. example.com",
                    )
                    .required(true)
                    .max_length(253),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "hash",
                    "Identify a file hash and check VirusTotal",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "value",
                        "MD5, SHA-1, SHA-256 or SHA-512 hash",
                    )
                    .required(true)
                    .min_length(32)
                    .max_length(128),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "vt",
                    "Query VirusTotal for a file, URL, domain or IP",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "query",
                        "Hash, URL, domain or IP address",
                    )
                    .required(true)
                    .max_length(2048),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "type",
                        "What the query is (required)",
                    )
                    .required(true)
                    .add_string_choice("file", "file")
                    .add_string_choice("url", "url")
                    .add_string_choice("domain", "domain")
                    .add_string_choice("ip", "ip"),
                ),
            ),
    ]
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some((name, args)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/intel` subcommand.").await;
    };

    if matches!(
        name,
        "cve" | "cve-latest" | "cve-search" | "ip" | "domain" | "vt"
    ) && let Some(remaining) = check_cooldown(cmd.user.id.get())
    {
        return respond::error(ctx, cmd, format!("Slow down — try again in {remaining}s.")).await;
    }

    match name {
        "cve" => cve(state, ctx, cmd, args).await,
        "cve-latest" => cve_latest(state, ctx, cmd, args).await,
        "cve-search" => cve_search(state, ctx, cmd, args).await,
        "mitre" => mitre(ctx, cmd, args).await,
        "ip" => ip_intel(state, ctx, cmd, args).await,
        "domain" => domain_intel(state, ctx, cmd, args).await,
        "hash" => hash_intel(state, ctx, cmd, args).await,
        "vt" => vt_intel(state, ctx, cmd, args).await,
        other => respond::error(ctx, cmd, format!("Unknown subcommand `{other}`.")).await,
    }
}

async fn cve(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let id = match opts::req_str(args, "id").and_then(normalize_cve) {
        Ok(id) => id,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    respond::working(ctx, cmd, format!("Looking up {id}…")).await?;
    match nvd::cve_by_id(state, &id).await {
        Ok(cve) => respond::edit(ctx, cmd, cve_embed(&cve)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn cve_latest(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let severity = opts::opt_str(args, "severity").unwrap_or("any").to_owned();
    let limit = opts::opt_i64(args, "limit").unwrap_or(5).clamp(1, 10);
    let limit = usize::try_from(limit).unwrap_or(5);

    respond::working(ctx, cmd, "Fetching the latest CVEs…").await?;
    match nvd::latest(state, Some(&severity), limit).await {
        Ok(cves) => respond::edit(ctx, cmd, latest_embed(&cves, &severity)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn cve_search(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let query = match opts::req_str(args, "query") {
        Ok(query) => query.trim().to_owned(),
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    respond::working(
        ctx,
        cmd,
        format!("Searching CVEs for `{}`…", text::truncate(&query, 80)),
    )
    .await?;
    match nvd::search(state, &query, 8).await {
        Ok(cves) => respond::edit(ctx, cmd, search_embed(&cves, &query)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn mitre(
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let query = match opts::req_str(args, "query") {
        Ok(query) => query.to_owned(),
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    respond::working(
        ctx,
        cmd,
        format!("Searching ATT&CK for `{}`…", text::truncate(&query, 60)),
    )
    .await?;
    match mitre_embed(&query) {
        Ok(embed) => respond::edit(ctx, cmd, embed).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn ip_intel(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let address = match opts::req_str(args, "address") {
        Ok(raw) => match IpAddr::from_str(raw.trim()) {
            Ok(address) => address.to_string(),
            Err(_) => {
                return respond::error(
                    ctx,
                    cmd,
                    format!(
                        "`{}` is not a valid IP address.",
                        text::truncate(raw.trim(), 80)
                    ),
                )
                .await;
            }
        },
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    respond::working(ctx, cmd, format!("Looking up {address}…")).await?;
    let (ipinfo_result, rdap_result) =
        tokio::join!(ipinfo::lookup(state, &address), rdap::ip(state, &address));

    let info = match ipinfo_result {
        Ok(info) => Some(info),
        Err(error) => {
            tracing::debug!("IPinfo lookup failed: {error}");
            None
        }
    };
    let record = match rdap_result {
        Ok(record) => Some(record),
        Err(error) => {
            tracing::debug!("RDAP IP lookup failed: {error}");
            None
        }
    };

    if info.is_none() && record.is_none() {
        return respond::error(
            ctx,
            cmd,
            format!("No intel found for {address} — IPinfo and RDAP both failed."),
        )
        .await;
    }

    respond::edit(ctx, cmd, ip_embed(&address, info.as_ref(), record.as_ref())).await
}

async fn domain_intel(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let name = match opts::req_str(args, "name") {
        Ok(raw) => match rdap::normalize_domain(raw) {
            Some(domain) => domain,
            None => {
                return respond::error(
                    ctx,
                    cmd,
                    format!(
                        "`{}` is not a valid domain name.",
                        text::truncate(raw.trim(), 80)
                    ),
                )
                .await;
            }
        },
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    respond::working(ctx, cmd, format!("Looking up {name}…")).await?;
    match rdap::domain(state, &name).await {
        Ok(record) => respond::edit(ctx, cmd, domain_embed(&record)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn hash_intel(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let value = match opts::req_str(args, "value") {
        Ok(raw) => raw.trim().to_ascii_lowercase(),
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    let Some(kind) = detect_hash(&value) else {
        return respond::error(
            ctx,
            cmd,
            "That does not look like an MD5, SHA-1, SHA-256 or SHA-512 hex digest.",
        )
        .await;
    };

    respond::working(ctx, cmd, "Identifying hash…").await?;

    let embed = if vt::configured(state) {
        match vt::file(state, &value).await {
            Ok(report) => hash_embed(&value, kind, Some(&report), None),
            Err(error) => {
                tracing::debug!("VirusTotal hash lookup failed: {error}");
                hash_embed(&value, kind, None, Some(&error.to_string()))
            }
        }
    } else {
        hash_embed(
            &value,
            kind,
            None,
            Some(
                "VirusTotal is not configured on this instance (VT_API_KEY), \
                 so no reputation data is shown.",
            ),
        )
    };

    respond::edit(ctx, cmd, embed).await
}

async fn vt_intel(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    let (query, kind) = match (opts::req_str(args, "query"), opts::req_str(args, "type")) {
        (Ok(query), Ok(kind)) => (query.trim().to_owned(), kind.to_owned()),
        (Err(error), _) | (_, Err(error)) => return respond::error(ctx, cmd, error).await,
    };

    if !vt::configured(state) {
        return respond::error(
            ctx,
            cmd,
            "VirusTotal is not configured on this instance (set VT_API_KEY).",
        )
        .await;
    }

    let target = match kind.as_str() {
        "file" => match detect_hash(&query) {
            Some(_) => query.to_ascii_lowercase(),
            None => {
                return respond::error(ctx, cmd, "`type: file` expects a hex file hash.").await;
            }
        },
        "url" => {
            if query.chars().count() > 2048 {
                return respond::error(ctx, cmd, "That URL is too long (max 2048 characters).")
                    .await;
            }
            if !text::is_url(&query) {
                return respond::error(ctx, cmd, "That is not a valid http(s) URL.").await;
            }
            query
        }
        "domain" => match rdap::normalize_domain(&query) {
            Some(domain) => domain,
            None => return respond::error(ctx, cmd, "That is not a valid domain name.").await,
        },
        "ip" => match IpAddr::from_str(&query) {
            Ok(address) => address.to_string(),
            Err(_) => return respond::error(ctx, cmd, "That is not a valid IP address.").await,
        },
        other => {
            return respond::error(ctx, cmd, format!("Unknown VirusTotal type `{other}`.")).await;
        }
    };

    respond::working(
        ctx,
        cmd,
        format!("Querying VirusTotal for `{}`…", text::truncate(&target, 80)),
    )
    .await?;

    let result = match kind.as_str() {
        "file" => vt::file(state, &target).await,
        "url" => vt::url(state, &target).await,
        "domain" => vt::domain(state, &target).await,
        _ => vt::ip(state, &target).await,
    };

    match result {
        Ok(report) => respond::edit(ctx, cmd, vt_embed(&report)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

/// Case-insensitive CVE identifier check; returns the canonical upper-case form.
fn normalize_cve(raw: &str) -> AnyResult<String> {
    let candidate = raw.trim().to_ascii_uppercase();
    if CVE_RE.as_ref().is_some_and(|re| re.is_match(&candidate)) {
        Ok(candidate)
    } else {
        bail!(
            "`{}` is not a valid CVE ID (expected CVE-YYYY-NNNN).",
            text::truncate(raw.trim(), 40)
        );
    }
}

/// Identify a hex digest by length. Non-hex or unsupported lengths return `None`.
fn detect_hash(value: &str) -> Option<HashKind> {
    let candidate = value.trim();
    if candidate.is_empty() || !candidate.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match candidate.len() {
        32 => Some(HashKind::Md5),
        40 => Some(HashKind::Sha1),
        56 => Some(HashKind::Sha224),
        64 => Some(HashKind::Sha256),
        96 => Some(HashKind::Sha384),
        128 => Some(HashKind::Sha512),
        _ => None,
    }
}

/// `Some(remaining_seconds)` when the caller is still inside the cooldown.
fn cooldown_remaining(last: Option<i64>, now: i64) -> Option<i64> {
    last.and_then(|previous| {
        let elapsed = now.saturating_sub(previous).max(0);
        if elapsed < INTEL_COOLDOWN_SECS {
            Some(INTEL_COOLDOWN_SECS - elapsed)
        } else {
            None
        }
    })
}

fn check_cooldown(user_id: u64) -> Option<i64> {
    let now = time::unix_now();
    let mut calls = LAST_INTEL_CALL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let remaining = cooldown_remaining(calls.get(&user_id).copied(), now);
    if remaining.is_none() {
        calls.insert(user_id, now);
    }
    remaining
}

fn cve_embed(cve: &nvd::Cve) -> CreateEmbed {
    let score = cve
        .score
        .map(|score| format!("{score:.1}"))
        .unwrap_or_else(|| "unknown".to_owned());
    let severity = cve.severity.clone().unwrap_or_else(|| "unknown".to_owned());
    let vector = cve.vector.clone().unwrap_or_else(|| "unknown".to_owned());

    let mut embed = CreateEmbed::new()
        .title(cve.id.clone())
        .url(format!("https://nvd.nist.gov/vuln/detail/{}", cve.id))
        .description(text::truncate(&cve.description, 1000))
        .colour(severity_colour(cve.severity.as_deref()))
        .field("CVSS", format!("**{score}** ({severity})"), true)
        .field("Vector", text::truncate(&vector, 200), false)
        .field("Published", text::truncate(&cve.published, 100), true)
        .field(
            "Last modified",
            text::truncate(&cve.last_modified, 100),
            true,
        );

    if !cve.weaknesses.is_empty() {
        let weaknesses = cve
            .weaknesses
            .iter()
            .take(5)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        embed = embed.field("Weaknesses", text::truncate(&weaknesses, 1000), false);
    }
    if !cve.references.is_empty() {
        let references = cve
            .references
            .iter()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        embed = embed.field("References", text::truncate(&references, 1000), false);
    }

    embeds::footer(embed)
}

fn latest_embed(cves: &[nvd::Cve], severity: &str) -> CreateEmbed {
    let title = if severity.eq_ignore_ascii_case("any") || severity.is_empty() {
        "Latest CVEs".to_owned()
    } else {
        format!("Latest {} CVEs", severity.to_ascii_uppercase())
    };

    let mut embed = CreateEmbed::new().title(title).colour(embeds::brand());
    for cve in cves.iter().take(10) {
        let severity = cve.severity.as_deref().unwrap_or("UNKNOWN");
        let score = cve
            .score
            .map(|score| format!("{score:.1}"))
            .unwrap_or_else(|| "?".to_owned());
        let value = format!(
            "**{severity}** · {score}\n{}",
            text::truncate(&cve.description, 200)
        );
        embed = embed.field(cve.id.clone(), value, false);
    }

    embeds::footer(embed)
}

fn search_embed(cves: &[nvd::Cve], query: &str) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title(format!("CVE search: {}", text::truncate(query, 80)))
        .colour(embeds::brand());

    for cve in cves.iter().take(10) {
        let severity = cve.severity.as_deref().unwrap_or("UNKNOWN");
        let score = cve
            .score
            .map(|score| format!("{score:.1}"))
            .unwrap_or_else(|| "?".to_owned());
        let value = format!(
            "**{severity}** · {score}\n{}",
            text::truncate(&cve.description, 200)
        );
        embed = embed.field(
            format!("[{}](https://nvd.nist.gov/vuln/detail/{})", cve.id, cve.id),
            value,
            false,
        );
    }

    embeds::footer(embed)
}

fn mitre_embed(query: &str) -> AnyResult<CreateEmbed> {
    let matches = content::techniques_matching(query, 5);
    let Some(best) = matches.first() else {
        bail!(
            "No ATT&CK technique matched `{}`.",
            text::truncate(query.trim(), 60)
        );
    };

    let mut embed = CreateEmbed::new()
        .title(format!("{} — {}", best.id, best.name))
        .url(format!("https://attack.mitre.org/techniques/{}/", best.id))
        .description(text::truncate(best.description, 1000))
        .colour(embeds::brand())
        .field("Tactic", best.tactic, true)
        .field("Detection", text::truncate(best.detection, 1000), false);

    if matches.len() > 1 {
        let others = matches
            .iter()
            .skip(1)
            .map(|technique| format!("{} {}", technique.id, technique.name))
            .collect::<Vec<_>>()
            .join(", ");
        embed = embed.field("Other matches", text::truncate(&others, 1000), false);
    }

    Ok(embeds::footer(embed))
}

fn ip_embed(
    address: &str,
    info: Option<&ipinfo::IpInfo>,
    record: Option<&rdap::RdapIp>,
) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title(format!("IP intel — {address}"))
        .colour(embeds::brand());
    let mut notes: Vec<&str> = Vec::new();

    if let Some(info) = info {
        let location = [
            info.city.as_deref(),
            info.region.as_deref(),
            info.country.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        embed = embed.field(
            "Location",
            if location.is_empty() {
                "unknown".to_owned()
            } else {
                location
            },
            true,
        );
        embed = embed.field(
            "Organisation",
            info.org.clone().unwrap_or_else(|| "unknown".to_owned()),
            true,
        );
        if let Some(hostname) = &info.hostname {
            embed = embed.field("Hostname", hostname.clone(), true);
        }
        if let Some(timezone) = &info.timezone {
            embed = embed.field("Timezone", timezone.clone(), true);
        }
    } else {
        notes.push("IPinfo unavailable — showing RDAP data only.");
    }

    if let Some(record) = record {
        let mut network: Vec<String> = Vec::new();
        if let (Some(start), Some(end)) = (&record.start, &record.end) {
            network.push(format!("{start} – {end}"));
        }
        if !record.cidr.is_empty() {
            network.push(record.cidr.join(", "));
        }
        if let Some(name) = &record.name {
            network.push(format!("Name: {name}"));
        }
        if let Some(handle) = &record.handle {
            network.push(format!("Handle: {handle}"));
        }
        if let Some(organization) = &record.organization {
            network.push(format!("Org: {organization}"));
        }
        if let Some(country) = &record.country {
            network.push(format!("Country: {country}"));
        }
        if !network.is_empty() {
            embed = embed.field("RDAP", text::truncate(&network.join("\n"), 1000), false);
        }
    } else {
        notes.push("RDAP unavailable.");
    }

    if !notes.is_empty() {
        embed = embed.field("Note", notes.join(" "), false);
    }

    embeds::footer(embed)
}

fn domain_embed(record: &rdap::RdapDomain) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title(format!("Domain intel — {}", record.domain))
        .colour(embeds::brand())
        .field(
            "Registrar",
            record
                .registrar
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            true,
        )
        .field(
            "Created",
            record
                .created
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            true,
        )
        .field(
            "Expires",
            record
                .expires
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            true,
        );

    if let Some(updated) = &record.updated {
        embed = embed.field("Updated", text::truncate(updated, 100), true);
    }
    if !record.statuses.is_empty() {
        embed = embed.field(
            "Statuses",
            text::truncate(&record.statuses.join(", "), 1000),
            false,
        );
    }
    if !record.nameservers.is_empty() {
        embed = embed.field(
            "Nameservers",
            text::truncate(&record.nameservers.join(", "), 1000),
            false,
        );
    }

    embeds::footer(embed)
}

fn hash_embed(
    value: &str,
    kind: HashKind,
    report: Option<&vt::VtReport>,
    note: Option<&str>,
) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title("Hash intel")
        .colour(embeds::brand())
        .field("Type", kind.label(), true)
        .field("Value", format!("`{value}`"), false);

    if let Some(report) = report {
        embed = add_vt_stats(embed, report);
    }
    if let Some(note) = note {
        embed = embed.field("VirusTotal", text::truncate(note, 1000), false);
    }

    embeds::footer(embed)
}

fn vt_embed(report: &vt::VtReport) -> CreateEmbed {
    let detected = report.malicious + report.suspicious;
    let colour = if detected > 0 {
        embeds::RED
    } else {
        embeds::brand()
    };
    let embed = CreateEmbed::new()
        .title(format!(
            "VirusTotal — {}",
            text::truncate(&report.target, 200)
        ))
        .url(report.permalink.clone())
        .colour(colour);

    embeds::footer(add_vt_stats(embed, report))
}

fn add_vt_stats(embed: CreateEmbed, report: &vt::VtReport) -> CreateEmbed {
    let detected = report.malicious + report.suspicious;
    let total = detected + report.harmless + report.undetected + report.timeout;

    let mut embed = embed
        .field("Detections", format!("{detected}/{total}"), true)
        .field("Malicious", report.malicious.to_string(), true)
        .field("Suspicious", report.suspicious.to_string(), true)
        .field("Harmless", report.harmless.to_string(), true)
        .field("Undetected", report.undetected.to_string(), true);

    if let Some(timestamp) = report.last_analysis_date {
        embed = embed.field("Last analysis", time::human_date(timestamp), true);
    }
    if !report.names.is_empty() {
        embed = embed.field(
            "Names",
            text::truncate(&report.names.join("\n"), 1000),
            false,
        );
    }

    embed
}

fn severity_colour(severity: Option<&str>) -> serenity::all::Colour {
    match severity.map(str::to_ascii_lowercase).as_deref() {
        Some("critical") | Some("high") => embeds::RED,
        Some("medium") => embeds::AMBER,
        _ => embeds::BLUE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cve_ids_are_normalized_and_validated() {
        assert_eq!(
            normalize_cve("cve-2024-3094").expect("valid"),
            "CVE-2024-3094"
        );
        assert_eq!(
            normalize_cve("  CVE-2026-1234567 ").expect("valid"),
            "CVE-2026-1234567"
        );
        for bad in [
            "",
            "CVE-24-3094",
            "CVE-2024-123",
            "CVE-2024-12345678",
            "2024-3094",
            "CVE-2024-ABCD",
        ] {
            assert!(normalize_cve(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn detects_hash_types_by_length() {
        assert_eq!(detect_hash(&"a".repeat(32)), Some(HashKind::Md5));
        assert_eq!(detect_hash(&"b".repeat(40)), Some(HashKind::Sha1));
        assert_eq!(detect_hash(&"c".repeat(56)), Some(HashKind::Sha224));
        assert_eq!(detect_hash(&"d".repeat(64)), Some(HashKind::Sha256));
        assert_eq!(detect_hash(&"e".repeat(96)), Some(HashKind::Sha384));
        assert_eq!(detect_hash(&"f".repeat(128)), Some(HashKind::Sha512));
        assert_eq!(detect_hash(&"A".repeat(64)), Some(HashKind::Sha256));

        assert_eq!(detect_hash(""), None);
        assert_eq!(detect_hash(&"z".repeat(64)), None);
        assert_eq!(detect_hash(&"a".repeat(33)), None);
        assert_eq!(detect_hash("not-a-hash"), None);
    }

    #[test]
    fn cooldown_window_is_four_seconds() {
        assert_eq!(cooldown_remaining(None, 1_000), None);
        assert_eq!(cooldown_remaining(Some(1_000), 1_000), Some(4));
        assert_eq!(cooldown_remaining(Some(1_000), 1_002), Some(2));
        assert_eq!(cooldown_remaining(Some(1_000), 1_003), Some(1));
        assert_eq!(cooldown_remaining(Some(1_000), 1_004), None);
        assert_eq!(cooldown_remaining(Some(1_000), 5_000), None);
        // Clock skew: a future timestamp is treated as a fresh call.
        assert_eq!(cooldown_remaining(Some(2_000), 1_000), Some(4));
    }

    #[test]
    fn mitre_lookup_finds_technique() {
        let embed = mitre_embed("T1059").expect("known technique");
        let _ = embed;
        assert!(mitre_embed("definitely-not-a-technique").is_err());
    }
}
