<div align="center">

<img src="https://img.shields.io/badge/Rust-1.99%2B-de8b3e?style=for-the-badge&logo=rust&logoColor=white" alt="Rust 1.99+">
<img src="https://img.shields.io/badge/serenity-0.12.5-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="serenity 0.12.5">
<img src="https://img.shields.io/badge/license-MIT-blue?style=for-the-badge" alt="MIT license">
<img src="https://img.shields.io/badge/docker-ready-2496ED?style=for-the-badge&logo=docker&logoColor=white" alt="Docker ready">
<br>
<a href="https://github.com/cleeryy/cipherbot/actions/workflows/ci.yml"><img src="https://github.com/cleeryy/cipherbot/actions/workflows/ci.yml/badge.svg" alt="CI"></a>

</div>

# CipherBot

> The operating system of a cybersecurity Discord community: one Rust binary, one SQLite file, no external database or queue.

CipherBot bundles moderation and anti-raid, a CTF engine, threat-intel lookups, a blue-team toolbox, passive recon, button polls, FAQ tags, private support tickets, a learning track and a curated link archive behind a consistent slash-command surface. Optional API keys unlock richer intel. Moderation, CTF, tags, polls, tickets and the learning content work offline; intel, DNS, recon and the breach check call fixed public APIs.

---

## Table of contents

- [Features](#features)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [Commands](#commands)
- [Architecture](#architecture)
- [Data model](#data-model)
- [Security model](#security-model)
- [Operations](#operations)
- [Development](#development)
- [Deployment](#deployment)
- [FAQ and troubleshooting](#faq-and-troubleshooting)
- [License](#license)

---

## Features

### Guide — `/start`

- Ephemeral interactive hub: one message, buttons, 14 panels besides Home (mod, CTF, intel, recon, tools, learn, links, profile, poll, tag, ticket, config, help, close).
- Navigation is stateless (`start:<panel>` custom ids), so buttons keep working across restarts.
- Staff panels re-check the clicker's permissions on every click: **Moderation** needs `Moderate Members`, **Configuration** needs `Manage Guild` (administrators bypass both).

### Protection — `/mod`

- Warning history, timeouts, kicks, bans, unban by ID, bulk purge, slowmode, channel lock/unlock.
- Case management with per-guild numbering, `case view|list|close` and modlog embeds for moderation actions.
- Staff inbox: `/report` feeds a persisted queue read with `/mod reports`, and `/mod evidence` reviews captured automod/moderation evidence.
- Automod: invite links, phishing domains, mass mentions, duplicate spam, excessive caps, leaked tokens (all on by default).
- Anti-raid join gate (account age / avatar requirements) with optional quarantine role.
- Verify panel: one button grants the configured verify role.

Commands: `/mod warn`, `warnings`, `timeout`, `untimeout`, `kick`, `ban`, `unban`, `purge`, `slowmode`, `lock`, `unlock`, `case view|list|close`, `raidmode`, `quarantine`, `unquarantine`, `verify-panel`, `reports`, `report-close`, `evidence`.

### Competition — `/ctf`

- Create challenges with points, category and a hashed flag.
- First-blood tracking, solve counts and a per-guild scoreboard.
- Flags are stored as guild-scoped SHA-256 hashes and never echoed in logs. `/ctf solve` replies **ephemerally** so flags and results are never broadcast.

Commands: `/ctf create`, `list`, `info`, `solve`, `scoreboard`, `delete`.

### Intel — `/intel`

- CVE lookup, latest CVEs by severity, keyword CVE search (NVD), MITRE ATT&CK techniques, IP/domain registration data (RDAP, optional ipinfo), DNS records over Cloudflare DoH, hash and URL reputation (VirusTotal, optional key).
- Results cached in SQLite to stay inside free-tier rate limits. The optional daily digest posts critical CVEs to a configured channel at 09:00 UTC.
- Every `/intel` reply is **ephemeral** — the caller sees the lookup, the channel does not.

Commands: `/intel cve`, `cve-latest`, `cve-search`, `mitre`, `ip`, `dns`, `domain`, `hash`, `vt`.

### Tradecraft — `/tool`

- Encode/decode (Base64, hex, URL), hash identification, MD5/SHA digests, Shannon entropy, CVSS v3.1 scoring, password exposure check, search dorks, JWT inspection, CIDR analysis and timestamp conversion.
- `hash`, `ipcalc`, `timestamp` reply ephemerally like `encode`/`decode`; only `hashid`, `entropy`, `cvss` and `dork` are public.

Commands: `/tool encode`, `decode`, `hashid`, `entropy`, `cvss`, `password`, `dork`, `jwt`, `hash`, `ipcalc`, `timestamp`.

### Learning — `/learn`

- Multiple-choice quizzes with streaks and accuracy tracking, a glossary, roadmaps and curated resources.
- **Every reply is ephemeral** — quiz, glossary, roadmap, resources and tip are visible only to the caller, so `/learn` never spams the channel. The quiz buttons still work and the stats/streak still update.

Commands: `/learn quiz`, `glossary`, `roadmap`, `resources`, `tip`.

### Polls — `/poll`

- `/poll create question:… option1:… option2:…` posts a **public** message with up to five options, one vote button per option and a close button.
- Votes edit the message in place: live ten-cell bars, counts, percentages and a total in the footer. No extra message is posted per vote.
- One vote per member, changeable at any time; the poll author or a member with `Manage Messages` can close it. `/poll close id:` and `/poll results id:` work by poll number. There is no duration or auto-close.

Commands: `/poll create`, `close`, `results`.

### Tags — `/tag`

- Reusable snippets and FAQ answers stored per guild: `/tag get name:faq` and `/tag list` are public; `/tag add` and `/tag remove` require `Manage Messages`.
- Tag names are normalized to `[a-z0-9_-]`, 1–32 characters; content is capped at 1,500 characters and markdown-escaped at render time.
- `/tag list` shows up to 50 names; `/tag get` is the only way to read a full body.

Commands: `/tag get`, `list`, `add`, `remove`.

### Support — `/ticket`

- `/ticket panel channel:#support role:@Staff` posts a public panel with an **Ouvrir un ticket** button; the button creates a **private thread** in that channel and adds the opener.
- Staff see the thread through the panel channel's permissions: a role needs `Manage Threads` to view private threads it was not added to (Discord rule), or staff must be added manually. The configured ticket role is pinged in a short panel-channel notice when a ticket opens; it is not added to the private thread.
- One open ticket per member; clicking again while one is open links to the existing thread. `/ticket close` (inside the thread, opener or staff) archives and locks it; `/ticket list` shows the queue to `Moderate Members` or the ticket role.
- Ticket staff role: `/config set key:ticket_role value:<role ID>` or the `role` option of `/ticket panel`.

Commands: `/ticket panel`, `close`, `list`.

### Passive recon — `/recon`

- Subdomains from certificate transparency (crt.sh), Shodan InternetDB ports/hostnames/CVEs, Wayback Machine snapshots and public urlscan.io results.
- **Passive only:** every request goes to a fixed third-party API. No packet, connection or HTTP request is ever sent to the target.
- IP lookups accept **public addresses only** — private, loopback, link-local, CGNAT and documentation ranges are refused. Domains must be bare hostnames (no scheme, path or wildcard).
- Results cached in SQLite (crt.sh 6 h, InternetDB 1 h, Wayback 6 h, urlscan 1 h); a 5-second per-user cooldown limits abuse.
- Every `/recon` reply is **ephemeral** (`respond::working` then an in-place edit), so results are not broadcast to the channel.

Commands: `/recon subdomains`, `ip`, `wayback`, `urlscan`.

### Curation — `/links`

- In monitored categories, a message containing a link automatically gets a thread (unless auto-threading is disabled); a duplicate URL is answered with a link to the existing thread and the new message is removed.
- Messages without a link are forwarded into the channel's latest thread (when one exists) and removed from the parent channel; only forwarded copies expire after the configured TTL.
- Searchable archive with contributor counts and top-domain statistics.

Commands: `/links search`, `recent`, `stats`. The guild-level `/config links` overrides per-category YAML once used.

### Community — `/profile`, `/leaderboard`, `/report`, `/suggest`, `/config`

- Member profiles (level, XP, messages, quiz accuracy, streak, CTF points, warnings) and leaderboards.
- Staff reports and up/down-voted suggestions.
- Per-guild configuration: modlog, CTF channel, digest channel, quarantine/verify roles, ticket staff role, automod toggles, raid thresholds and the link pipeline (`/config` has `view`, `modlog`, `ctf-channel`, `automod`, `verify`, `set`, `links`).

### Operations

- `GET /healthz` JSON probe, `GET /metrics` Prometheus text.
- Structured `tracing` logs, graceful shutdown, SQLite WAL storage.
- `/help`, `/stats`, `/ping`, `/about`.

**Non-goals for v1:** music, economy, web dashboard, sharding.

---

## Quick start

### 1. Create the Discord application

1. Open the [Discord Developer Portal](https://discord.com/developers/applications) and create an application.
2. **Bot** tab: reset/copy the token.
3. **Privileged Gateway Intents**: enable **Message Content Intent**. Enable **Server Members Intent** only if you plan to use the anti-raid join gate (`ENABLE_MEMBER_INTENT=true`).
4. **OAuth2 → URL Generator**: scopes `bot` and `applications.commands`, permissions listed below, then open the generated URL.

Required permissions (integer `1497064631318`):

| Permission | Why |
|---|---|
| View Channels, Send Messages, Embed Links, Read Message History | Replies, embeds, purge/cleanup |
| Manage Messages | Purge, automod deletion, `/tag add` / `/tag remove` |
| Manage Threads, Create Public Threads | Link threads and forwarding |
| Create Private Threads | Private support-ticket threads |
| Send Messages in Threads | Replies inside link and ticket threads |
| Moderate Members | Timeouts, warnings, cases, report/evidence review, `/ticket list` |
| Kick Members, Ban Members | Kick/ban/unban |
| Manage Channels | Lock/unlock, slowmode |
| Manage Roles | Quarantine and verify roles |
| Use Application Commands | Slash commands |

Invite URL template:

```text
https://discord.com/oauth2/authorize?client_id=YOUR_APP_ID&permissions=1497064631318&scope=bot%20applications.commands
```

> `/config`, `/ticket panel`, `/mod raidmode` and `/mod verify-panel` additionally require the **invoking moderator** to hold `Manage Guild`; that permission is not part of the invite integer, so grant it explicitly or use an administrator for those actions.

> **Role hierarchy:** `Manage Roles` only works on roles below the bot's highest role. Move the CipherBot role above the quarantine and verify roles, otherwise those actions fail with `Missing Permissions`.

### 2. Configure

```bash
git clone https://github.com/cleeryy/cipherbot
cd cipherbot
cp .env.example .env
$EDITOR .env          # at minimum: DISCORD_TOKEN=...
```

The binary loads `.env` automatically (`dotenvy`) and also accepts a YAML file (`CONFIG_PATH`, default `config.yaml`). Environment variables win over YAML.

### 3. Run

Docker (recommended):

```bash
mkdir -p data && sudo chown 10001:10001 data   # container runs as uid 10001
docker compose up -d --build
docker compose logs -f cipherbot
```

From source:

```bash
cargo run --release
```

Guild commands are registered on startup for every guild the bot is in, and again when it joins a new guild. Run `/help` to confirm.

---

## Configuration

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `DISCORD_TOKEN` | — (required) | Bot token. Never commit it. |
| `DATABASE_PATH` | `cipherbot.db` | SQLite file path. Docker uses `/data/cipherbot.db`. |
| `MONITORED_CATEGORIES` | empty | Comma-separated category IDs for link curation. Empty disables the pipeline. |
| `HEALTH_ADDR` | `0.0.0.0:8080` | Health/metrics listener. An empty value disables the HTTP server. |
| `NVD_API_KEY` | unset | NVD API key (higher CVE rate limit). |
| `VT_API_KEY` | unset | VirusTotal API key (`/intel hash`, `/intel vt`). |
| `IPINFO_TOKEN` | unset | ipinfo.io token (enriches `/intel ip`). |
| `ENABLE_MEMBER_INTENT` | `false` | Enables the privileged `GUILD_MEMBERS` intent for the anti-raid join gate. |
| `RUST_LOG` | `info` | Tracing filter, e.g. `info,cipherbot=debug`. |
| `CONFIG_PATH` | `config.yaml` | YAML config file; a missing file is not an error. |

Boolean env values accept `1/true/yes/on` (case-insensitive).

The polls, tags, tickets, `/start` and `/recon` features add **no environment variables**. The optional intel keys are unchanged, and `/recon`, `/intel dns` and `/tool` need no API key at all — only outbound HTTPS.

### YAML

```yaml
discord_token: ""            # prefer DISCORD_TOKEN

database:
  path: "cipherbot.db"

health:
  addr: "0.0.0.0:8080"
  enabled: true

categories:                  # optional link curation
  - id: 123456789012345678
    auto_thread_links: true
    message_ttl_hours: 24

intel: {}                    # nvd_api_key / vt_api_key / ipinfo_token

features:
  member_intent: false       # requires the Server Members Intent
```

`config.example.yaml` is the commented reference. Category IDs: enable Developer Mode in Discord, right-click the category → **Copy ID**.

---

## Commands

All 20 top-level commands, all guild-scoped. The full reference — options, permissions, examples — is in **[docs/USAGE.md](docs/USAGE.md)**.

| Command | Description |
|---|---|
| `/start` | Interactive button hub covering every feature. |
| `/mod` | Warn, warnings, timeout, untimeout, kick, ban, unban, purge, slowmode, lock, unlock, case view/list/close, raidmode, quarantine, unquarantine, verify-panel, reports, report-close, evidence. |
| `/ctf` | Create, list, info, solve, scoreboard and delete CTF challenges. |
| `/intel` | CVE lookup, cve-latest, cve-search, ATT&CK, IP, DNS, domain, hash and VirusTotal lookups. |
| `/tool` | Encode/decode, hashid, hash, entropy, CVSS, password check, dorks, JWT, ipcalc, timestamp. |
| `/learn` | Quiz, glossary, roadmap, resources, tip — all ephemeral. |
| `/poll` | Create, close and inspect button polls with live bars. |
| `/tag` | Reusable snippets and FAQ answers. |
| `/ticket` | Support panel, private ticket threads and the staff queue. |
| `/recon` | Passive OSINT: subdomains, InternetDB, Wayback, urlscan. |
| `/links` | Search, recent and stats for the curated link archive. |
| `/profile` | Member profile: level, XP, messages, quiz stats, CTF points, warnings (self/staff). |
| `/leaderboard` | Server rankings by XP, quiz accuracy or CTF points. |
| `/report` | Report a member or message to staff. |
| `/suggest` | Submit a suggestion with vote buttons. |
| `/config` | View settings; modlog, ctf-channel, automod, verify, ticket role, set and links. |
| `/help` | List commands or show details for one. |
| `/stats` | Bot statistics. |
| `/ping` | Liveness and uptime. |
| `/about` | Version and source link. |

---

## Architecture

```text
                         Discord Gateway / REST
                                   │
┌──────────────────────────────────▼───────────────────────────────────┐
│ src/main.rs        config → SQLite → AppState → serenity Client      │
│                    intents: GUILDS, GUILD_MESSAGES, MESSAGE_CONTENT, │
│                    GUILD_MODERATION (+ GUILD_MEMBERS if enabled)     │
└───────────────┬──────────────────────────────────────┬───────────────┘
                │                                      │
     src/handler.rs (EventHandler)              src/ops/health.rs
       ready        → register guild commands     GET /healthz
       guild_create → re-register                 GET /metrics
       message      → pipeline
       interaction  → commands / components
                │
                ▼
  security/automod → security/flagwatch → db::users → commands::links::on_message
                │
       interaction_create
                ▼
  src/commands/mod.rs  dispatch (metrics, unknown-command guard)
    ├─ moderation  ctf  intel  toolbox  learning  community  links
    └─ start  poll  tag  ticket  recon  help / stats / ping / about (core)
                │
  src/components/mod.rs  verify: · quiz: · vote: · ctf: · start: · poll: · ticket:
                │
                ▼
  src/db/*  (SQLite, WAL)      src/services/*  (nvd, rdap, ipinfo, vt, dns, recon, scheduler)
```

| Module | Responsibility |
|---|---|
| `src/config.rs` | YAML + env loading, `Secret` newtype with redacted `Debug`. |
| `src/bot.rs` | `AppState` / `SharedState`, HTTP client, phishing feed, metrics. |
| `src/handler.rs` | Serenity event routing, command registration, presence rotation. |
| `src/commands/mod.rs` | Registry (`all_commands`), dispatch, help table. |
| `src/commands/start.rs` | `/start` hub: 14 stateless panels, per-click staff gating. |
| `src/commands/poll.rs` | `/poll` create/close/results, vote and close buttons, live bars. |
| `src/commands/tag.rs` | `/tag` get/list/add/remove, per-guild FAQ snippets. |
| `src/commands/ticket.rs` | `/ticket` panel/close/list, private threads, archive on close. |
| `src/commands/recon.rs` | `/recon` passive OSINT embeds, domain/IP validation, cooldown. |
| `src/components/mod.rs` | Button/select router (`module:action[:args]`, stateless). |
| `src/db/` | One SQLite connection behind a mutex; migrations via `PRAGMA user_version`. |
| `src/security/` | Automod, anti-raid, flag watch, phishing feed. |
| `src/services/` | NVD, RDAP, ipinfo, VirusTotal, Cloudflare DoH, passive recon, scheduler. |
| `src/ops/` | Health server and Prometheus metrics. |
| `src/util/` | Response helpers, embeds, option parsing, time/text/hash/permission helpers, `ipcalc`, `timestamp`. |
| `src/content/` | Static glossaries, roadmaps, quiz banks, resources. |

### Message pipeline

1. Bots and webhooks are ignored upstream.
2. `security::automod::on_message` — may delete, store evidence and write the modlog.
3. `security::flagwatch` — deletes leaked flags (outside the configured CTF channel) and stops the pipeline.
4. `db::users` — awards XP and message counters; never stops processing.
5. `commands::links::on_message` — creates link threads / forwards non-link messages.

The 60-second cleanup loop (`commands::links::cleanup_loop`) deletes TTL-expired forwarded messages; `services::scheduler` refreshes the phishing feed (15 min), the guild gauge (60 min), the intel cache GC (10 min) and checks for the 09:00 UTC critical-CVE digest every 15 minutes (at most one digest per UTC day).

---

## Data model

One SQLite file, WAL mode, `busy_timeout=5000`, `synchronous=NORMAL`, foreign keys on. Timestamps are Unix seconds except the legacy `tracked_messages.delete_at` text column.

Schema v2 adds four tables on top of v1 (migration runs automatically at startup and is keyed on `PRAGMA user_version`):

| Table | Contents |
|---|---|
| `guild_config` | Per-guild JSON settings (`GuildSettings`, including `ticket_role`). |
| `cases` | Moderation cases, numbered per guild, with expiry and active flag. |
| `ctf_challenges` / `ctf_solves` | Challenges (flag = SHA-256 hash) and solves. |
| `users` | XP, message count, quiz accuracy, streak and first-seen date. |
| `links` | Curated link archive (URL, domain, channel, thread, author). |
| `intel_cache` | Cached external API payloads with expiry (also caches `/recon` and DNS results). |
| `feed_state` | Feed checkpoints. |
| `reports` / `suggestions` / `suggestion_votes` | Staff reports, community suggestions and per-user votes. |
| `evidence` | Automod evidence (content, author, channel, reason) — local only. |
| `tracked_messages` / `channel_threads` | TTL queue for forwarded link-thread copies and the last thread per monitored channel. |
| `tags` | Per-guild FAQ snippets: `(guild_id, name)` primary key, content, author, use counter. |
| `polls` | Poll question, `options_json`, channel/message ids, author, `closed_at`. |
| `poll_votes` | One row per `(poll_id, user_id)`, holding the chosen option index. |
| `tickets` | Ticket threads: `thread_id` primary key, guild, panel channel, opener, status, close metadata. |

---

## Security model

- **Token handling** — the token is read from `DISCORD_TOKEN` (or YAML) and wrapped in `Secret`; `Debug` prints `Secret(***)`. It is never logged, stored in SQLite, or echoed in errors.
- **CTF flags** — stored as guild-scoped SHA-256 hashes. Submitted flags are compared in memory and never logged.
- **Password checks** — `/tool password` hashes locally with SHA-1 and sends only the first 5 hex characters to the Have I Been Pwned range API (k-anonymity). The password itself never leaves the process.
- **Passive recon** — `/recon` is **passive only**: it queries fixed third-party APIs (crt.sh, Shodan InternetDB, Wayback CDX, urlscan.io) and never sends a packet to the target. Domains are validated as bare hostnames and IPs must be globally routable, so the command cannot be pointed at a LAN, loopback or CGNAT address.
- **Support tickets** — ticket threads are private threads. Discord only lets roles with `Manage Threads` see private threads they were not added to; grant that permission to staff, or add staff manually. The bot never posts ticket content outside the thread.
- **Ephemeral learning** — every `/learn` reply (quiz, glossary, roadmap, resources, tip) is ephemeral; no channel spam and no public quiz answers.
- **External calls** — fixed hosts only, no user-controlled URL fetching. Most calls use the shared 5-second client; passive recon uses a dedicated 20-second client because crt.sh is slow. If you find an SSRF vector, it is a bug: report it privately.
- **Automod evidence** — message content is stored in your local database so moderators can review it; credential-shaped content is redacted first. It never leaves the host.
- **Health endpoints** — `/healthz` and `/metrics` are unauthenticated. Bind to a private network or Docker network; do not expose `/metrics` publicly.
- **Permissions** — re-checked in code for every sensitive action; `default_member_permissions` on the command is cosmetic. See [SECURITY.md](SECURITY.md) for the full threat model and disclosure process.

---

## Operations

### Health and metrics

```bash
curl -s localhost:8080/healthz
# {"status":"ok","version":"1.0.0","uptime_s":42,"guilds":3,"db_ok":true}

curl -s localhost:8080/metrics | grep cipherbot_
```

`status` becomes `degraded` and `db_ok` turns `false` when the SQLite probe fails; the HTTP status stays `200` so the process is not restarted for a recoverable database issue.

Exported metrics: `cipherbot_commands_total`, `cipherbot_messages_total`, `cipherbot_automod_actions_total`, `cipherbot_links_threaded_total`, `cipherbot_errors_total`, `cipherbot_guilds`, `cipherbot_uptime_seconds`, `cipherbot_db_ok`, `cipherbot_build_info`.

### Logs

Set `RUST_LOG` (e.g. `info,cipherbot=debug`, or `warn,serenity=info`). Logs go to stdout as structured `tracing` lines; run under Docker, systemd or your supervisor and rotate at the collector.

### Backups

SQLite WAL means a naive file copy can miss the `-wal` contents. Use the online backup API:

```bash
# Run on the host — the runtime image does not ship the sqlite3 CLI.
sqlite3 data/cipherbot.db ".backup 'backup-$(date +%F).db'"
```

or stop the container and copy `cipherbot.db` together with `cipherbot.db-wal` and `cipherbot.db-shm`. Restore = replace the file(s) while the bot is stopped.

### Upgrades

1. Back up the database.
2. Pull the new image (`docker compose pull`) or rebuild.
3. `docker compose up -d`.
4. Watch `/healthz` and the logs. Schema migrations run at startup and are keyed on `PRAGMA user_version`; they are idempotent.

### Resource usage

Budget 256 MB of RAM and 0.5 CPU for a small community; 512 MB / 1 CPU covers busy multi-guild deployments. The shipped `docker-compose.yml` uses 512 MB / 1.0 CPU. Memory grows with Serenity's guild/member cache, especially with `GUILD_MEMBERS` enabled.

---

## Development

Requirements: Rust 1.99+ (2024 edition), and the `rustfmt`/`clippy` components.

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # rustup installs outside the default PATH
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The build is CPU-bound; when several checks share one checkout, use the serialized helpers:

```bash
./scripts/ck.sh      # flock-serialized cargo check
./scripts/test.sh    # flock-serialized cargo test
./scripts/dev.sh     # cargo run with RUST_LOG=info,cipherbot=debug
```

### Further reading

- [Command reference](docs/USAGE.md)
- [Design notes](docs/DESIGN.md)
- [Verified serenity 0.12.5 API notes](docs/SERENITY_API.md)

### Adding a command module

1. Create `src/commands/foo.rs` exposing exactly:

   ```rust
   pub fn commands() -> Vec<CreateCommand>;
   pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()>;
   ```

2. Register it in `src/commands/mod.rs`: `pub mod foo;`, extend `all_commands()`, add the dispatch arm, and add a `HELP_TABLE` entry.
3. Acknowledge first (`respond::working`), then `edit_response`. Errors must be ephemeral (`respond::error`).
4. Add a unit test for the non-Discord logic. The command lint test (`command_definitions_pass_discord_lint`) checks names, description lengths, duplicates and option limits.
5. Run `cargo fmt`, clippy and tests before opening a PR.

See [CONTRIBUTING.md](CONTRIBUTING.md) for branch/commit conventions and the PR checklist.

---

## Deployment

Full production checklist: **[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)**.

### Docker

```bash
docker build -t cipherbot .
docker run -d --name cipherbot \
  --env-file .env \
  -e DATABASE_PATH=/data/cipherbot.db \
  -p 127.0.0.1:8080:8080 \
  -v cipherbot-data:/data \
  --restart unless-stopped \
  cipherbot
```

### Compose

`docker-compose.yml` runs the image read-only, drops all capabilities, sets `no-new-privileges`, caps memory at 512 MB / 1 CPU and stores data in `./data`.

### Dockploy

Create an application from the image (or the Git repository with the Dockerfile), mount a persistent volume at `/data`, set `DISCORD_TOKEN` and friends as environment variables, and expose port `8080` only on the internal network.

### systemd

```ini
[Unit]
Description=CipherBot
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=cipher
Group=cipher
WorkingDirectory=/opt/cipherbot
EnvironmentFile=/opt/cipherbot/.env
Environment=DATABASE_PATH=/var/lib/cipherbot/cipherbot.db
ExecStart=/usr/local/bin/cipherbot
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/var/lib/cipherbot

[Install]
WantedBy=multi-user.target
```

`systemctl stop` sends SIGTERM; the bot drains shards on SIGINT, so use `KillSignal=SIGINT` if you want the graceful path in systemd too.

---

## FAQ and troubleshooting

**`4014 Disallowed intent(s)` on startup.** A privileged intent is missing in the Developer Portal. Enable **Message Content Intent**; if `ENABLE_MEMBER_INTENT=true`, also enable **Server Members Intent**. Intents cannot be enabled by code.

**Slash commands do not appear.** Commands are guild-scoped and registered when the bot connects (and on `guild_create`). Check that the bot was invited with the `applications.commands` scope, that it is online, then refresh the Discord client (`Ctrl+R`). A newly invited guild receives commands on the next `guild_create`/restart. After an upgrade that adds commands, a client refresh is usually enough; if not, restart the bot.

**`/ticket` staff cannot see the new thread.** Private threads are only visible to roles holding `Manage Threads` (or to members explicitly added to the thread). Grant `Manage Threads` to the staff role, or add staff manually inside the thread. The ticket role is pinged in the intro but not added to the thread.

**`/recon` refuses an IP as private.** Only globally routable IPs are queried. Private, loopback, link-local, CGNAT and documentation ranges fail by design; resolve the hostname with `/intel dns` instead.

**`Missing Permissions`.** Compare the bot's role permissions with the table in [Quick start](#quick-start). Quarantine/verify need `Manage Roles` **and** a bot role above the target role. Timeouts need `Moderate Members`.

**`Missing Access` when creating threads.** The bot needs `Create Public Threads`, `Send Messages in Threads` and `Read Message History` in that channel/category.

**Health check fails but the bot is online.** `HEALTH_ADDR` may have been changed or emptied (empty disables the server), or the port is already bound. The server only logs a warning and the bot keeps running, so check `docker logs cipherbot` for `Health server could not bind`.

**`database is locked`.** Run exactly one instance per SQLite file. WAL allows concurrent readers but not multiple writers across processes; do not scale the container horizontally on the same volume.

**`No link-monitor categories configured` warning.** Expected unless `MONITORED_CATEGORIES` (or `categories` in YAML) is set. The rest of the bot works without it.

**Rate limits.** Serenity handles 429s with backoff. Intel calls are cached in SQLite; without `NVD_API_KEY` the NVD limit is low, and VirusTotal free keys allow only a few requests per minute. Add keys or wait.

---

## License

MIT — see [LICENSE](LICENSE).

Built with [serenity](https://github.com/serenity-rs/serenity), [rusqlite](https://github.com/rusqlite/rusqlite), [tokio](https://tokio.rs) and the Rust community's crates. Thanks to the maintainers of NVD, MITRE ATT&CK, RDAP, ipinfo and Have I Been Pwned for their public APIs.
