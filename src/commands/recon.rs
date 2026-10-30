//! `/recon` — passive OSINT: certificate transparency, Shodan InternetDB,
//! Wayback CDX and urlscan.io.
//!
//! Hard safety rule: this command never scans, connects to or fetches anything
//! from the target. Every call goes to a fixed, well-known passive API and the
//! user input only appears in that API's query string. IP lookups are limited
//! to globally routable addresses.
//!
//! Every subcommand acknowledges ephemerally, validates, queries the service
//! layer (which owns caching) and edits the placeholder with an embed.

use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};

use serenity::all::{
    CommandDataOption, CommandInteraction, CommandOptionType, Context, CreateCommand,
    CreateCommandOption, CreateEmbed, CreateEmbedFooter,
};

use crate::bot::{BotResult, SharedState};
use crate::services::recon;
use crate::util::{embeds, opts, respond, text, time};

const RECON_COOLDOWN_SECS: i64 = 5;
/// Display cap for the subdomain list; the report itself keeps everything.
const MAX_SUBDOMAINS: usize = 200;
const MAX_TRACKED_USERS: usize = 10_000;
const EMBED_DESCRIPTION_MAX: usize = 4_000;
const EMBED_FIELD_MAX: usize = 1_000;

/// Last accepted `/recon` call per user. `std::sync::Mutex`: the guard is never
/// held across an await point.
static LAST_RECON_CALL: LazyLock<Mutex<HashMap<u64, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("recon")
            .description("Passive recon: subdomains, InternetDB, Wayback, urlscan")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "subdomains",
                    "Subdomains from certificate transparency (crt.sh)",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "domain",
                        "Domain without scheme, e.g. example.com",
                    )
                    .required(true)
                    .max_length(253),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "ip",
                    "Shodan InternetDB ports, hostnames and CVEs",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "address",
                        "Public IPv4 or IPv6 address",
                    )
                    .required(true)
                    .max_length(45),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "wayback",
                    "Wayback Machine snapshots for a domain",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "domain",
                        "Domain without scheme, e.g. example.com",
                    )
                    .required(true)
                    .max_length(253),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "limit",
                        "How many snapshots to show (1-50, default 15)",
                    )
                    .min_int_value(1)
                    .max_int_value(50),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "urlscan",
                    "Recent public urlscan.io scans for a domain",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "domain",
                        "Domain without scheme, e.g. example.com",
                    )
                    .required(true)
                    .max_length(253),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "limit",
                        "How many scans to show (1-15, default 10)",
                    )
                    .min_int_value(1)
                    .max_int_value(15),
                ),
            ),
    ]
}

pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()> {
    let Some((name, args)) = opts::subcommand(&cmd.data) else {
        return respond::error(ctx, cmd, "Unknown `/recon` subcommand.").await;
    };

    if let Some(remaining) = check_cooldown(cmd.user.id.get()) {
        return respond::error(ctx, cmd, format!("Slow down — try again in {remaining}s.")).await;
    }

    match name {
        "subdomains" => subdomains(state, ctx, cmd, args).await,
        "ip" => ip(state, ctx, cmd, args).await,
        "wayback" => wayback(state, ctx, cmd, args).await,
        "urlscan" => urlscan(state, ctx, cmd, args).await,
        other => respond::error(ctx, cmd, format!("Unknown subcommand `{other}`.")).await,
    }
}

async fn subdomains(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    respond::working(ctx, cmd, "Querying certificate transparency (crt.sh)…").await?;
    let domain = match domain_arg(args) {
        Ok(domain) => domain,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    match recon::subdomains(state, &domain).await {
        Ok(report) => respond::edit(ctx, cmd, subdomain_embed(&report)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn ip(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    respond::working(ctx, cmd, "Querying Shodan InternetDB…").await?;
    let address = match opts::req_str(args, "address")
        .map_err(|error| error.to_string())
        .and_then(validate_ip)
    {
        Ok(address) => address,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };

    match recon::internetdb(state, address).await {
        Ok(report) => respond::edit(ctx, cmd, host_embed(&report)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn wayback(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    respond::working(ctx, cmd, "Querying the Wayback Machine…").await?;
    let domain = match domain_arg(args) {
        Ok(domain) => domain,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };
    let limit = limit_arg(args, 15, 1, 50);

    match recon::wayback(state, &domain, limit).await {
        Ok(report) => respond::edit(ctx, cmd, wayback_embed(&report)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

async fn urlscan(
    state: &SharedState,
    ctx: &Context,
    cmd: &CommandInteraction,
    args: &[CommandDataOption],
) -> BotResult<()> {
    respond::working(ctx, cmd, "Querying urlscan.io…").await?;
    let domain = match domain_arg(args) {
        Ok(domain) => domain,
        Err(error) => return respond::error(ctx, cmd, error).await,
    };
    let limit = limit_arg(args, 10, 1, 15);

    match recon::urlscan(state, &domain, limit).await {
        Ok(report) => respond::edit(ctx, cmd, urlscan_embed(&report)).await,
        Err(error) => respond::error(ctx, cmd, error).await,
    }
}

fn domain_arg(args: &[CommandDataOption]) -> Result<String, String> {
    let raw = opts::req_str(args, "domain").map_err(|error| error.to_string())?;
    validate_domain(raw)
}

fn validate_domain(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if IpAddr::from_str(trimmed).is_ok() {
        return Err(format!(
            "`{}` is an IP address — use `/recon ip`.",
            text::truncate(trimmed, 80)
        ));
    }
    recon::normalize_domain(trimmed).ok_or_else(|| {
        format!(
            "`{}` is not a valid domain name — send a bare hostname without scheme, path or wildcard.",
            text::truncate(trimmed, 80)
        )
    })
}

fn validate_ip(raw: &str) -> Result<IpAddr, String> {
    let trimmed = raw.trim();
    let address = IpAddr::from_str(trimmed).map_err(|_| {
        format!(
            "`{}` is not a valid IP address.",
            text::truncate(trimmed, 80)
        )
    })?;
    if !recon::is_public_ip(&address) {
        return Err(format!(
            "`{address}` is not a public address — only globally routable IPs are queried."
        ));
    }
    Ok(address)
}

/// Read the optional `limit` option, clamped to the documented range.
fn limit_arg(args: &[CommandDataOption], default: i64, min: i64, max: i64) -> usize {
    let value = opts::opt_i64(args, "limit")
        .unwrap_or(default)
        .clamp(min, max);
    usize::try_from(value).unwrap_or(1)
}

/// `Some(remaining_seconds)` while the caller is still inside the cooldown.
fn cooldown_remaining(last: Option<i64>, now: i64) -> Option<i64> {
    last.and_then(|previous| {
        let elapsed = now.saturating_sub(previous).max(0);
        if elapsed < RECON_COOLDOWN_SECS {
            Some(RECON_COOLDOWN_SECS - elapsed)
        } else {
            None
        }
    })
}

fn check_cooldown(user_id: u64) -> Option<i64> {
    let now = time::unix_now();
    let mut calls = LAST_RECON_CALL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if calls.len() > MAX_TRACKED_USERS {
        calls.retain(|_, last| now.saturating_sub(*last) < RECON_COOLDOWN_SECS);
    }
    let remaining = cooldown_remaining(calls.get(&user_id).copied(), now);
    if remaining.is_none() {
        calls.insert(user_id, now);
    }
    remaining
}

fn source_footer(embed: CreateEmbed, source: &str) -> CreateEmbed {
    embed.footer(CreateEmbedFooter::new(format!(
        "Passive OSINT — no traffic to the target · {source} · CipherBot v{}",
        env!("CARGO_PKG_VERSION")
    )))
}

fn subdomain_lines(report: &recon::SubdomainReport) -> String {
    report
        .names
        .iter()
        .take(MAX_SUBDOMAINS)
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn subdomain_embed(report: &recon::SubdomainReport) -> CreateEmbed {
    let lines = subdomain_lines(report);
    let description = if report.names.is_empty() {
        "No subdomains were found in certificate transparency logs.".to_owned()
    } else {
        text::truncate(&lines, EMBED_DESCRIPTION_MAX)
    };

    let mut embed = CreateEmbed::new()
        .title(format!(
            "🔎 Subdomains — {}",
            text::truncate(&report.domain, 200)
        ))
        .description(description)
        .colour(embeds::brand())
        .field(
            "Apex",
            if report.apex_present {
                "seen in CT logs"
            } else {
                "not listed"
            },
            true,
        )
        .field("Names found", report.total.to_string(), true);

    if report.names.len() > MAX_SUBDOMAINS {
        embed = embed.field(
            "Note",
            format!(
                "Showing the first {MAX_SUBDOMAINS} of {} subdomains.",
                report.names.len()
            ),
            false,
        );
    }

    source_footer(embed, "crt.sh")
}

fn host_embed(report: &recon::HostReport) -> CreateEmbed {
    let ports = if report.ports.is_empty() {
        "None reported".to_owned()
    } else {
        report
            .ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let hostnames = if report.hostnames.is_empty() {
        "None reported".to_owned()
    } else {
        report
            .hostnames
            .iter()
            .take(10)
            .map(|hostname| format!("`{hostname}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let cves = if report.vulns.is_empty() {
        "None reported".to_owned()
    } else {
        report
            .vulns
            .iter()
            .take(15)
            .map(|cve| format!("`{cve}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tags = if report.tags.is_empty() {
        "None".to_owned()
    } else {
        report.tags.join(", ")
    };

    let embed = CreateEmbed::new()
        .title(format!("🔎 InternetDB — {}", report.ip))
        .url(format!("https://internetdb.shodan.io/{}", report.ip))
        .colour(if report.vulns.is_empty() {
            embeds::brand()
        } else {
            embeds::RED
        })
        .field("Ports", text::truncate(&ports, EMBED_FIELD_MAX), false)
        .field(
            "Hostnames",
            text::truncate(&hostnames, EMBED_FIELD_MAX),
            false,
        )
        .field("CVEs", report.vulns.len().to_string(), true)
        .field("CPEs", report.cpes.len().to_string(), true)
        .field("Top CVEs", text::truncate(&cves, EMBED_FIELD_MAX), false)
        .field("Tags", text::truncate(&tags, EMBED_FIELD_MAX), false);

    source_footer(embed, "Shodan InternetDB")
}

fn wayback_lines(report: &recon::WaybackReport) -> String {
    report
        .rows
        .iter()
        .take(report.limit.max(1))
        .map(|row| {
            let timestamp = if row.timestamp.is_empty() {
                "unknown".to_owned()
            } else {
                row.timestamp.clone()
            };
            let status = if row.status.is_empty() {
                "-----"
            } else {
                row.status.as_str()
            };
            let mimetype = if row.mimetype.is_empty() {
                "unknown"
            } else {
                row.mimetype.as_str()
            };
            format!(
                "`{timestamp}` **{status}** {mimetype} — {}",
                text::truncate(&row.original, 180)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn wayback_embed(report: &recon::WaybackReport) -> CreateEmbed {
    let lines = wayback_lines(report);
    let description = if report.rows.is_empty() {
        "No snapshots were found for this domain.".to_owned()
    } else {
        text::truncate(&lines, EMBED_DESCRIPTION_MAX)
    };

    let embed = CreateEmbed::new()
        .title(format!(
            "🔎 Wayback — {}",
            text::truncate(&report.domain, 200)
        ))
        .description(description)
        .colour(embeds::brand())
        .field("Snapshots", report.rows.len().to_string(), true)
        .field("Requested", report.limit.to_string(), true);

    source_footer(embed, "Wayback Machine CDX")
}

fn urlscan_lines(report: &recon::UrlscanReport) -> String {
    report
        .hits
        .iter()
        .take(report.limit.max(1))
        .map(|hit| {
            let flag = if hit.malicious > 0 {
                format!("⚠️ {} malicious", hit.malicious)
            } else {
                "clean".to_owned()
            };
            let stamp = if hit.time.is_empty() {
                "unknown".to_owned()
            } else {
                hit.time.clone()
            };
            let result = if hit.id.is_empty() {
                String::new()
            } else {
                format!(" — <https://urlscan.io/result/{}/>", hit.id)
            };
            format!(
                "`{stamp}` · {flag}\n<{}>{result}",
                text::truncate(&hit.url, 300)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn urlscan_embed(report: &recon::UrlscanReport) -> CreateEmbed {
    let lines = urlscan_lines(report);
    let malicious = report.hits.iter().filter(|hit| hit.malicious > 0).count();
    let description = if report.hits.is_empty() {
        "No public urlscan.io results for this domain.".to_owned()
    } else {
        text::truncate(&lines, EMBED_DESCRIPTION_MAX)
    };

    let embed = CreateEmbed::new()
        .title(format!(
            "🔎 urlscan — {}",
            text::truncate(&report.domain, 200)
        ))
        .url(format!("https://urlscan.io/domain/{}", report.domain))
        .description(description)
        .colour(if malicious > 0 {
            embeds::RED
        } else {
            embeds::brand()
        })
        .field("Results", report.hits.len().to_string(), true)
        .field("Malicious scans", malicious.to_string(), true);

    source_footer(embed, "urlscan.io")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::recon::{HostReport, SubdomainReport, UrlscanHit, WaybackRow};

    fn sample_subdomains(count: usize) -> SubdomainReport {
        SubdomainReport {
            domain: "example.com".to_owned(),
            names: (0..count)
                .map(|index| format!("host{index:03}.example.com"))
                .collect(),
            apex_present: true,
            total: count + 1,
        }
    }

    fn sample_wayback() -> recon::WaybackReport {
        recon::WaybackReport {
            domain: "example.com".to_owned(),
            limit: 15,
            rows: vec![
                WaybackRow {
                    original: "https://example.com/".to_owned(),
                    timestamp: "20240101000000".to_owned(),
                    status: "200".to_owned(),
                    mimetype: "text/html".to_owned(),
                },
                WaybackRow {
                    original: "https://example.com/login".to_owned(),
                    timestamp: String::new(),
                    status: String::new(),
                    mimetype: String::new(),
                },
            ],
        }
    }

    fn sample_urlscan() -> recon::UrlscanReport {
        recon::UrlscanReport {
            domain: "example.com".to_owned(),
            limit: 10,
            hits: vec![
                UrlscanHit {
                    url: "https://example.com/login".to_owned(),
                    time: "2026-10-01T12:00:00.000Z".to_owned(),
                    malicious: 2,
                    id: "abc-123".to_owned(),
                },
                UrlscanHit {
                    url: "https://example.com/".to_owned(),
                    time: "2026-09-30T09:30:00.000Z".to_owned(),
                    malicious: 0,
                    id: "def-456".to_owned(),
                },
            ],
        }
    }

    fn assert_embed_limits(embed: CreateEmbed) {
        let value = serde_json::to_value(embed).expect("embed serializes");
        let mut total = 0usize;

        if let Some(title) = value.get("title").and_then(|v| v.as_str()) {
            assert!(title.chars().count() <= 256, "title too long");
            total += title.chars().count();
        }
        if let Some(description) = value.get("description").and_then(|v| v.as_str()) {
            assert!(description.chars().count() <= 4096, "description too long");
            total += description.chars().count();
        }
        if let Some(footer) = value
            .get("footer")
            .and_then(|f| f.get("text"))
            .and_then(|v| v.as_str())
        {
            total += footer.chars().count();
        }

        let fields = value
            .get("fields")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(fields.len() <= 25, "too many fields");
        for field in fields {
            let name = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let content = field.get("value").and_then(|v| v.as_str()).unwrap_or("");
            assert!(!name.is_empty() && name.chars().count() <= 256);
            assert!(!content.is_empty() && content.chars().count() <= 1024);
            total += name.chars().count() + content.chars().count();
        }
        assert!(total <= 6000, "embed too large: {total}");
    }

    #[test]
    fn cooldown_window_is_five_seconds() {
        assert_eq!(cooldown_remaining(None, 1_000), None);
        assert_eq!(cooldown_remaining(Some(1_000), 1_000), Some(5));
        assert_eq!(cooldown_remaining(Some(1_000), 1_004), Some(1));
        assert_eq!(cooldown_remaining(Some(1_000), 1_005), None);
        assert_eq!(cooldown_remaining(Some(1_000), 9_000), None);
        // Clock skew: a future timestamp is treated as a fresh call.
        assert_eq!(cooldown_remaining(Some(2_000), 1_000), Some(5));
    }

    #[test]
    fn domains_are_validated_and_ips_redirected() {
        assert_eq!(
            validate_domain("Example.COM."),
            Ok("example.com".to_owned())
        );
        assert!(validate_domain("sub.example.org").is_ok());

        let ip_error = validate_domain("1.1.1.1").expect_err("IP");
        assert!(ip_error.contains("`/recon ip`"), "{ip_error}");

        for bad in [
            "",
            "localhost",
            "https://example.com",
            "example.com/x",
            "*.example.com",
        ] {
            assert!(validate_domain(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn ip_input_must_be_public() {
        assert_eq!(
            validate_ip("1.1.1.1"),
            Ok(IpAddr::from_str("1.1.1.1").expect("fixture"))
        );
        assert_eq!(
            validate_ip("2606:4700:4700::1111"),
            Ok(IpAddr::from_str("2606:4700:4700::1111").expect("fixture"))
        );

        for private in [
            "10.0.0.1",
            "192.168.1.1",
            "127.0.0.1",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            let error = validate_ip(private).expect_err("private");
            assert!(!error.is_empty());
        }
        assert!(validate_ip("not-an-ip").is_err());
    }

    #[test]
    fn limit_option_is_clamped() {
        let args: Vec<CommandDataOption> =
            serde_json::from_str(r#"[{"name":"limit","type":4,"value":500}]"#).expect("options");
        assert_eq!(limit_arg(&args, 15, 1, 50), 50);
        assert_eq!(limit_arg(&[], 15, 1, 50), 15);
    }

    #[test]
    fn subdomain_list_is_capped() {
        let report = sample_subdomains(250);
        let lines = subdomain_lines(&report);
        assert_eq!(lines.lines().count(), MAX_SUBDOMAINS);
        assert!(lines.contains("host000.example.com"));
        assert!(!lines.contains("host200.example.com"));

        let empty = subdomain_lines(&SubdomainReport::default());
        assert!(empty.is_empty());
    }

    #[test]
    fn embeds_stay_within_discord_limits() {
        assert_embed_limits(subdomain_embed(&sample_subdomains(250)));
        assert_embed_limits(subdomain_embed(&SubdomainReport::default()));
        assert_embed_limits(host_embed(&HostReport {
            ip: "93.184.216.34".to_owned(),
            ports: (1..=300).collect(),
            hostnames: (0..40).map(|i| format!("host{i}.example.com")).collect(),
            cpes: (0..20)
                .map(|i| format!("cpe:/a:vendor{i}:product"))
                .collect(),
            vulns: (0..40).map(|i| format!("CVE-2024-{i:04}")).collect(),
            tags: (0..40).map(|i| format!("tag{i}")).collect(),
        }));
        assert_embed_limits(host_embed(&HostReport::default()));
        assert_embed_limits(wayback_embed(&sample_wayback()));
        assert_embed_limits(wayback_embed(&recon::WaybackReport::default()));
        assert_embed_limits(urlscan_embed(&sample_urlscan()));
        assert_embed_limits(urlscan_embed(&recon::UrlscanReport::default()));
    }

    #[test]
    fn wayback_lines_show_metadata() {
        let lines = wayback_lines(&sample_wayback());
        assert!(lines.contains("20240101000000"));
        assert!(lines.contains("**200**"));
        assert!(lines.contains("text/html"));
        assert!(lines.contains("https://example.com/login"));
        assert!(lines.contains("unknown"));
    }

    #[test]
    fn urlscan_lines_flag_malicious_scans() {
        let lines = urlscan_lines(&sample_urlscan());
        assert!(lines.contains("2 malicious"));
        assert!(lines.contains("<https://urlscan.io/result/abc-123/>"));
        assert!(lines.contains("clean"));
        assert!(lines.contains("https://example.com/"));
    }
}
