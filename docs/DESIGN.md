# CipherBot v1.0 — Design Notes

> Historical design notes from the v1.0 build, kept for contributors who want the
> rationale behind the module boundaries, database schema and safety rules. The
> current command reference is [`USAGE.md`](USAGE.md); the architecture overview
> is in the root [`README.md`](../README.md). The verified serenity 0.12.5 API
> cheat sheet is [`SERENITY_API.md`](SERENITY_API.md).

## 0. Product

CipherBot is the operating system of a cybersecurity Discord community. One Rust
binary, one SQLite file, no external services required. Optional API keys unlock
more intel. It must feel like a finished product: consistent embeds, ephemeral
errors, clear permissions, audit trail, docs, tests, CI, Docker healthcheck.

Pillars: **curation** (links), **protection** (automod / anti-raid / cases),
**competition** (CTF), **intel** (CVE / ATT&CK / RDAP), **tradecraft** (toolbox),
**learning** (quiz / glossary / roadmaps), **community** (profiles / leaderboards /
reports / suggestions), **operations** (health / metrics / graceful shutdown).

Non-goals for v1: music, tickets, economy, i18n, dashboard, sharding.

## 1. Ground rules

- Rust 2024, `anyhow::Result` everywhere (`type BotResult<T> = anyhow::Result<T>`).
- No `unwrap()` / `expect()` / `panic!()` in request paths. `unwrap_or_else` with a
  sane fallback is fine. Log with `tracing`, never log secrets or raw submitted flags.
- **Acknowledge every interaction.** Any handler that does I/O responds first with a
  placeholder embed, then `edit_response`. No silent handlers, ever.
- Errors are ephemeral embeds (`util::respond::error`), never raw `Debug` output.
- All user input is untrusted: escape markdown, cap lengths, validate IDs, use SQL
  parameters. No dynamic SQL identifiers.
- Every external HTTP call: fixed host (no user-controlled URL fetch), 5 s timeout,
  JSON, cached in SQLite where useful. Never send private data to third parties.
- Files are owned by one lane. Do not edit another lane's files. `Cargo.toml`,
  `src/main.rs`, `src/handler.rs`, `src/bot.rs`, `src/commands/mod.rs`,
  `src/db/mod.rs` are frozen after Phase 0 — request changes from the integrator.
- Build with `scripts/ck.sh` (flock-serialized `cargo check`). Never run bare
  `cargo build` in parallel. Format your own files with `rustfmt`.
- Every feature ships with at least one unit test for its non-Discord logic.

## 2. Frozen architecture

```
src/main.rs              bootstrap, client, shutdown
src/config.rs            YAML + env, Secret newtype
src/bot.rs               AppState, SharedState, Services
src/handler.rs           EventHandler: routing only
src/commands/mod.rs      registry + dispatch + help data
src/commands/{moderation,ctf,intel,toolbox,learning,community,links}.rs
src/components/mod.rs    component router (buttons/selects)
src/db/mod.rs            Database, connection, migrations
src/db/{cases,ctf,users,links,intel,guild,community,evidence}.rs   impl blocks
src/security/{automod,raid,flagwatch,phishing}.rs
src/services/{nvd,rdap,ipinfo,vt,scheduler}.rs
src/ops/{health,metrics}.rs
src/util/{respond,embeds,opts,time,text,hash,permissions}
                     + {encodings,hashid,entropy,cvss}.rs
src/content/*.rs         static data (DONE)
```

### AppState (frozen — `src/bot.rs`)

```rust
pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
    pub http: reqwest::Client,          // external APIs only
    pub metrics: Metrics,               // atomics
    pub started_at: std::time::Instant,
    pub phishing: PhishingStore,        // RwLock<HashSet<String>> domain feed
    pub bot_user_id: OnceLock<UserId>,
}
pub type SharedState = Arc<AppState>;
```

Services are plain functions taking `&AppState` (or free functions) — no trait objects.
`AppState` is built in `main` and passed as `Arc` into `Handler`.

### Dispatch (frozen — `src/commands/mod.rs`)

```rust
pub fn all_commands() -> Vec<CreateCommand>;                  // concat of modules
pub async fn dispatch(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()>;

// each feature module exposes exactly:
pub fn commands() -> Vec<CreateCommand>;
pub async fn handle(state: &SharedState, ctx: &Context, cmd: &CommandInteraction) -> BotResult<()>;
```

`dispatch` matches on `cmd.data.name`; unknown → one-line ephemeral error. Every
branch increments `metrics.commands_total`. Register **guild commands** in `ready`
for every guild returned by `http.get_guilds`, and again (idempotently) on
`guild_create`.

Components (`src/components/mod.rs`):

```rust
pub async fn dispatch(state: &SharedState, ctx: &Context, ci: &ComponentInteraction) -> BotResult<()>;
// custom_id format: "module:action[:args...]", <=100 chars, stateless.
// routes by prefix: "verify:" -> moderation, "quiz:" -> learning, "vote:" -> community, "ctf:" -> ctf.
```

Modals: not used in v1; log and politely reject.

### Message pipeline (frozen — `src/handler.rs`)

```rust
pub enum HookResult { Continue, Stop }

// called in this order; any Stop ends processing. Bots/webhooks ignored upstream.
security::automod::on_message(&state, &ctx, &msg).await   // may delete + evidence + modlog
commands::links::on_message(&state, &ctx, &msg).await     // may stop (thread created/forwarded)
db::users::on_message(&state, &ctx, &msg).await           // XP, never stops
```

`security::flagwatch::on_message` runs inside automod's pipeline (before links) and
returns `Stop` after deleting a leaked flag.

Cleanup loop lives in `commands::links::cleanup_loop(state)` (spawned in `main`),
plus `services::scheduler::run(state, ctx)` for feeds/digest/presence.

### Database (frozen — `src/db/mod.rs`)

`Database { conn: parking_lot::Mutex<Connection> }`. Open pragmas: `journal_mode=WAL`,
`busy_timeout=5000`, `foreign_keys=ON`, `synchronous=NORMAL`. **One lock per public
method**, no public method calls another. Return owned data. Timestamps are `INTEGER`
unix seconds (`Utc::now().timestamp()`). Migrations use `PRAGMA user_version`.

Schema v1 (all created in Phase 0):

```sql
guild_config(guild_id INTEGER PRIMARY KEY, settings_json TEXT NOT NULL, updated_at INTEGER NOT NULL)
cases(id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, case_number INTEGER NOT NULL,
      kind TEXT NOT NULL, target_id INTEGER NOT NULL, moderator_id INTEGER NOT NULL,
      reason TEXT NOT NULL, evidence TEXT, created_at INTEGER NOT NULL, expires_at INTEGER,
      active INTEGER NOT NULL DEFAULT 1, UNIQUE(guild_id, case_number))
ctf_challenges(id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, name TEXT NOT NULL,
      category TEXT NOT NULL, points INTEGER NOT NULL, flag_hash TEXT NOT NULL, channel_id INTEGER,
      description TEXT, created_by INTEGER NOT NULL, created_at INTEGER NOT NULL,
      first_blood_id INTEGER, first_blood_at INTEGER, solved_count INTEGER NOT NULL DEFAULT 0,
      UNIQUE(guild_id, name))
ctf_solves(challenge_id INTEGER NOT NULL, guild_id INTEGER NOT NULL, user_id INTEGER NOT NULL,
      points INTEGER NOT NULL, solved_at INTEGER NOT NULL, PRIMARY KEY(challenge_id, user_id))
users(guild_id INTEGER NOT NULL, user_id INTEGER NOT NULL, xp INTEGER NOT NULL DEFAULT 0,
      messages INTEGER NOT NULL DEFAULT 0, quiz_correct INTEGER NOT NULL DEFAULT 0,
      quiz_total INTEGER NOT NULL DEFAULT 0, streak INTEGER NOT NULL DEFAULT 0,
      last_quiz_day TEXT,
      first_seen INTEGER NOT NULL, PRIMARY KEY(guild_id, user_id))
links(id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, channel_id INTEGER NOT NULL,
      message_id INTEGER NOT NULL, thread_id INTEGER, author_id INTEGER NOT NULL, url TEXT NOT NULL,
      domain TEXT NOT NULL, posted_at INTEGER NOT NULL)
intel_cache(cache_key TEXT PRIMARY KEY, payload TEXT NOT NULL, fetched_at INTEGER NOT NULL,
      expires_at INTEGER NOT NULL)
feed_state(name TEXT PRIMARY KEY, last_checked INTEGER, entries INTEGER)
reports(id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, reporter_id INTEGER NOT NULL,
      target_id INTEGER NOT NULL, message_id INTEGER, reason TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'open',
      created_at INTEGER NOT NULL, handled_by INTEGER, handled_at INTEGER)
suggestions(id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, author_id INTEGER NOT NULL,
      content TEXT NOT NULL, message_id INTEGER, upvotes INTEGER NOT NULL DEFAULT 0,
      downvotes INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL DEFAULT 'open', created_at INTEGER NOT NULL)
evidence(id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, channel_id INTEGER NOT NULL,
      message_id INTEGER NOT NULL, author_id INTEGER NOT NULL, content TEXT NOT NULL,
      reason TEXT NOT NULL, created_at INTEGER NOT NULL)
-- legacy, keep: tracked_messages, channel_threads
```

`GuildSettings` (serde, in `src/db/guild.rs`, defaults for everything):
`modlog_channel`, `quarantine_role`, `verify_role`, `ctf_channel`, `digest_channel`,
`locale` ("en"), `automod: { invites, phishing, mass_mentions, duplicate_spam,
caps, token_leaks }` all bool, `raid: { enabled, min_account_age_hours, require_avatar }`,
`links: { auto_thread, ttl_hours }`.

### Config (frozen — `src/config.rs`)

YAML + env overrides. `discord_token` is `Secret` with redacted `Debug`. Env:
`DISCORD_TOKEN`, `DATABASE_PATH`, `MONITORED_CATEGORIES` (optional now),
`HEALTH_ADDR` (default `0.0.0.0:8080`, empty disables), `NVD_API_KEY`, `VT_API_KEY`,
`IPINFO_TOKEN`, `ENABLE_MEMBER_INTENT` (default false; enables anti-raid join events),
`RUST_LOG`. Categories remain the legacy link-monitor config.

### Utilities (frozen signatures)

```rust
// util/respond.rs
pub async fn working(ctx, cmd, text) -> BotResult<()>;            // ephemeral placeholder
pub async fn public(ctx, cmd, embed) -> BotResult<()>;
pub async fn ephemeral(ctx, cmd, embed) -> BotResult<()>;
pub async fn edit(ctx, cmd, embed) -> BotResult<()>;
pub async fn error(ctx, cmd, err: impl Display) -> BotResult<()>; // always ephemeral
pub async fn text(ctx, cmd, s, ephemeral) -> BotResult<()>;
// util/embeds.rs
pub fn ok(title, desc) -> CreateEmbed; pub fn err(desc) -> CreateEmbed;
pub fn warn(title, desc) -> CreateEmbed; pub fn info(title, desc) -> CreateEmbed;
pub fn brand() -> Colour; // 0x00E5A0 cyber green
pub fn footer(embed) -> CreateEmbed; // "CipherBot v…"
// util/opts.rs
pub fn subcommand(data: &CommandData) -> Option<(&str, &[CommandDataOption])>;
pub fn opt_str / opt_i64 / opt_bool / opt_user / opt_channel / opt_role;
pub fn req_str(opts, name) -> BotResult<&str>; // ephemeral-friendly error text
pub fn duration(opts, name) -> BotResult<i64>; // seconds, "10m"/"2h"/"7d"
// util/time.rs: parse_duration(&str) -> Option<i64>, human_duration(i64) -> String,
//               unix_now() -> i64, human_date(i64) -> String
// util/text.rs: truncate(&str, usize) -> String, escape_md(&str) -> String,
//               domain_of(&str) -> Option<String>, is_url(&str) -> bool, sha256_hex(&str)
// util/permissions.rs: require(cmd, Permissions) -> BotResult<()> (admin bypass),
//                      require_bot(cmd, Permissions) -> BotResult<()>
```

## 3. Command surface

The authoritative list of commands, options and permissions is [`USAGE.md`](USAGE.md).
Descriptions are ≤100 chars, names are lowercase, options are snake_case, and
permissions are re-checked in code (`default_member_permissions` is cosmetic).

- `/mod` — `warn`, `warnings`, `timeout`, `untimeout`, `kick`, `ban`, `unban`,
  `purge`, `slowmode`, `lock`, `unlock`, `case view|list|close`, `raidmode`,
  `quarantine`, `unquarantine`, `verify-panel`, `reports`, `report-close`,
  `evidence`. Punitive actions record a case and post a modlog embed.
- `/ctf` — `create`, `list`, `info`, `solve`, `scoreboard`, `delete`.
- `/intel` — `cve`, `cve-latest`, `cve-search`, `mitre`, `ip`, `domain`, `hash`, `vt`.
- `/tool` — `encode`, `decode`, `hashid`, `entropy`, `cvss`, `password`, `dork`, `jwt`.
- `/learn` — `quiz`, `glossary`, `roadmap`, `resources`, `tip`.
- `/links` — `search`, `recent`, `stats`.
- `/profile [user]`, `/leaderboard [metric]`, `/report`, `/suggest`.
- `/config` — `view`, `set`, `automod`, `modlog`, `ctf-channel`, `verify`, `links`.
- `/help [command]`, `/stats`, `/ping`, `/about`.

Component IDs: `verify:grant`, `quiz:<question_id>:<choice_index>`,
`vote:<up|down>:<suggestion_id>`, `ctf:hint:<challenge_id>` (reserved).

## 4. Quality gates

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test` (unit tests only; no live Discord token, no network calls)
- `cargo build --release --locked`
- `docker build` (multi-stage, non-root runtime, `/healthz` probe)
- `scripts/ck.sh` / `scripts/test.sh` for serialized local runs
