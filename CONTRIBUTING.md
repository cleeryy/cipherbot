# Contributing to CipherBot

Thanks for helping. CipherBot is a Rust 2024 / serenity 0.12 Discord bot with a SQLite backend; the whole product is one binary and one database file.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

---

## Development setup

Requirements:

- Rust **1.99+** with `rustfmt` and `clippy` (`rustup component add rustfmt clippy`).
- A Discord application if you want to run the bot live (not needed for tests).
- Docker only if you are working on the image or compose file.

rustup installs binaries in `~/.cargo/bin`, which is not on the default non-login `PATH`. Either open a new login shell or:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Clone and configure:

```bash
git clone https://github.com/cleeryy/cipherbot
cd cipherbot
cp .env.example .env          # DISCORD_TOKEN for a live run
cp config.example.yaml config.yaml   # optional: defaults work without it
cargo run
```

`scripts/dev.sh` runs the binary with `RUST_LOG=info,cipherbot=debug`. The binary loads `.env` itself, so the script never echoes secrets.

### Build discipline

Cargo takes an exclusive lock on `target/`. When several checks or CI jobs share the same checkout, use the serialized helpers:

```bash
./scripts/ck.sh      # flock-serialized cargo check
./scripts/test.sh    # flock-serialized cargo test
```

Avoid launching several `cargo build`/`cargo test` runs concurrently; they queue on the same lock and waste time.

---

## Further reading

- [`docs/USAGE.md`](docs/USAGE.md) — every command, option and permission.
- [`docs/DESIGN.md`](docs/DESIGN.md) — module boundaries, database schema and safety rules.
- [`docs/SERENITY_API.md`](docs/SERENITY_API.md) — verified serenity 0.12.5 API notes.

## Architecture map

| Path | Responsibility |
|---|---|
| `src/main.rs` | Bootstrap: config, database, `AppState`, client, intents, shutdown. |
| `src/config.rs` | YAML + env loading, `Secret` newtype (redacted `Debug`). |
| `src/bot.rs` | `AppState` / `SharedState`, HTTP client, metrics, phishing store. |
| `src/handler.rs` | Serenity `EventHandler`: routing, command registration, message pipeline. |
| `src/commands/mod.rs` | Command registry, dispatch, `/help` data, command lint test. |
| `src/commands/*.rs` | One module per feature: `commands()` + `handle()`. |
| `src/components/mod.rs` | Button/select router (`module:action[:args]`). |
| `src/db/` | SQLite access; one connection behind a mutex, one lock per public method. |
| `src/security/` | Automod, anti-raid, flag watch, phishing feed. |
| `src/services/` | NVD, RDAP, ipinfo, VirusTotal, scheduler. |
| `src/ops/` | Health server, Prometheus metrics. |
| `src/util/` | Respond/embed helpers, option parsing, time/text/hash/permissions. |
| `src/content/` | Static quiz/glossary/roadmap/resource data. |

Some files (`Cargo.toml`, `src/main.rs`, `src/handler.rs`, `src/bot.rs`, `src/commands/mod.rs`, `src/db/mod.rs`) are shared integration points. Discuss changes there in an issue first; feature work should stay inside the lane module.

---

## Adding a command module

1. Create `src/commands/foo.rs` with exactly this surface:

   ```rust
   use serenity::all::{CommandInteraction, Context, CreateCommand};
   use crate::bot::{BotResult, SharedState};

   pub fn commands() -> Vec<CreateCommand> {
       vec![CreateCommand::new("foo").description("One-line description")]
   }

   pub async fn handle(
       state: &SharedState,
       ctx: &Context,
       cmd: &CommandInteraction,
   ) -> BotResult<()> {
       // acknowledge first, then edit
       Ok(())
   }
   ```

2. Register it in `src/commands/mod.rs`:
   - `pub mod foo;`
   - extend `all_commands()`;
   - add the `"foo" => foo::handle(...)` dispatch arm;
   - add a `HELP_TABLE` entry.
3. Acknowledge every interaction before doing I/O (`respond::working`), then `respond::edit`. Errors must go through `respond::error` (ephemeral), never raw `Debug`.
4. Add a unit test for the non-Discord logic (parsing, formatting, hashing, SQL).
5. Run formatting, clippy and tests.

Naming rules (enforced by `command_definitions_pass_discord_lint` in `src/commands/mod.rs`): command and option names match `^[a-z0-9_-]{1,32}$`, descriptions are non-empty and ≤ 100 characters, options ≤ 25 per command, no duplicates.

### Components

Buttons and select menus use stateless `custom_id`s of the form `module:action[:args...]` (≤ 100 chars). Route them by prefix in `src/components/mod.rs`; unknown IDs must answer politely instead of panicking.

### Database changes

- New tables or columns go through the schema block in `src/db/mod.rs` and a new `PRAGMA user_version` migration step; migrations must be idempotent and forward-only.
- Timestamps are Unix seconds (`INTEGER`), except the legacy `tracked_messages.delete_at` text column.
- Every public `Database` method takes the mutex once, does not call another public method, and returns owned data.
- Use SQL parameters; never interpolate identifiers or user input into SQL.

---

## Tests

```bash
cargo test
```

- Unit tests live next to the code (`#[cfg(test)] mod tests`).
- Database tests use `tempfile::tempdir()` and a real SQLite file.
- **No live Discord and no network in tests.** Mock or test the pure logic; the CI has no token and no outbound calls to Discord.
- Every feature needs at least one test for its non-Discord logic.
- The command registry must stay valid: the lint test serializes every `CreateCommand` and validates names, descriptions and options.

---

## Style

- `anyhow::Result` everywhere; `type BotResult<T> = anyhow::Result<T>`.
- No `unwrap()`, `expect()` or `panic!()` in request paths. `unwrap_or_else` with a safe fallback is fine.
- All user input is untrusted: escape markdown, cap lengths, validate IDs, parameterize SQL.
- External HTTP: fixed host, 5-second timeout, JSON. Never fetch a user-supplied URL (no SSRF surface).
- Log with `tracing`; never log tokens, raw flags, passwords or message content from evidence paths.
- Keep handlers thin; business logic in modules, persistence in `db/`.
- No `#[allow(dead_code)]` additions without a comment explaining the lifetime of the code.

Formatting and lints (CI runs exactly this):

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

---

## Branches and commits

- Branch from `main`: `feat/short-name`, `fix/short-name`, `docs/short-name`, `chore/short-name`.
- Conventional Commits:

  ```text
  feat(ctf): add first-blood tracking
  fix(automod): ignore staff in duplicate-spam check
  docs: document the link curation pipeline
  refactor(db): extract case numbering
  chore(ci): cache cargo registry
  ```

- Keep commits focused and the tree buildable. Rebase on `main` before opening a PR; avoid merge commits in feature branches.

---

## Pull request checklist

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --locked` passes.
- [ ] New behaviour has a unit test; no live Discord or network calls.
- [ ] New commands are registered, documented in `docs/USAGE.md`, and pass the command lint test.
- [ ] No secret, token, real flag or personal data in the diff, logs or fixtures.
- [ ] Docs updated (`README.md`, `docs/USAGE.md`, `docs/DEPLOYMENT.md`) when behaviour or configuration changes.
- [ ] `Dockerfile`/compose still build if the change touches runtime assets or env vars.

---

## Reporting security issues

Do **not** open a public issue for a vulnerability. Follow [SECURITY.md](SECURITY.md): use a GitHub private security advisory.

Questions? Open a discussion or an issue.
