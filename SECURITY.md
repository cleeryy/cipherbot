# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 1.0.x | ✅ |
| < 0.1 | ❌ |

Only the latest `1.0.x` release (and `main`) is supported. Fixes are shipped as patch releases; there are no long-lived maintenance branches.

## Reporting a vulnerability

Use a **GitHub private security advisory**:

<https://github.com/cleeryy/cipherbot/security/advisories/new>

Do not open a public issue, and do not post details, tokens or exploit code in Discord.

Include:

- affected version/commit and deployment mode (Docker, binary, platform);
- a description of the issue and the impact (what an attacker gains);
- minimal reproduction steps or a proof of concept;
- any mitigating configuration already in place;
- a suggested fix if you have one.

What to expect:

- acknowledgement within **48 hours**;
- an initial severity assessment within **5 days**;
- a fix or mitigation plan for confirmed issues, and credit in the advisory if desired;
- coordinated disclosure: please allow at least **30 days** for medium/high issues before publishing.

## Threat model

### Assets

- The Discord bot token (full control of the bot).
- Guild data in SQLite: moderation cases, warnings, automod evidence, reports, suggestions, link history, hashed CTF flags.
- Optional API keys (NVD, VirusTotal, ipinfo).
- Integrity of moderation and verification actions.

### Trust boundaries

- **Discord → bot**: every interaction and message is untrusted input, including markdown, IDs, flags and added components.
- **Bot → external APIs**: fixed-host calls only. User input is limited to the indicator being looked up; no user-controlled URLs are fetched.
- **Operator → process**: environment variables and the config file are trusted; the host filesystem and Docker daemon are trusted.

### Scenarios and mitigations

| Scenario | Mitigation |
|---|---|
| Token leak through logs, crash dumps or config commits | Token lives in `Secret`, whose `Debug` prints `Secret(***)`; env-only recommended; `.env`/`config.yaml` git-ignored; logs never include the token. Rotate immediately in the Developer Portal if leaked (restart required). |
| Privileged intent abuse | `MESSAGE_CONTENT` is required for the product; `GUILD_MEMBERS` is opt-in via `ENABLE_MEMBER_INTENT` and documented as such. The bot only reads what it needs. |
| SSRF via intel/tool commands | **Out of scope by design**: every outbound call targets a fixed host with a fixed path template; users supply an indicator, never a URL to fetch. A regression here is a valid vulnerability — report it. |
| Command permission bypass | Sensitive actions re-check permissions in code (`util::permissions::require`); `default_member_permissions` is cosmetic. Administrators bypass checks intentionally. Role hierarchy is enforced by Discord. |
| Automod evasion | Automod is heuristic (invite patterns, phishing feed, mention/duplicate/caps thresholds). It is a speed bump, not a guarantee; evidence is stored locally for human review. |
| CTF flag disclosure | Flags are stored as guild-scoped SHA-256 hashes, never plaintext, never logged. A database reader can brute-force low-entropy flags offline — use random flags for real events. |
| Leaked-flag watch | `flagwatch` deletes messages that match known flag formats and stops the message pipeline; best-effort, not a secrecy guarantee. |
| Password check privacy | `/tool password` hashes locally and sends only the first 5 SHA-1 hex characters to HIBP (k-anonymity). The password still transits Discord when typed into the command. |
| Database at rest | SQLite is unencrypted; protect the volume with host disk encryption, `0700` permissions and encrypted backups. Anyone with the file can read all retained evidence. |
| Health/metrics exposure | `/healthz` and `/metrics` are unauthenticated. They expose version, uptime, guild count and counters. Bind to loopback/private network; never publish `/metrics`. |
| Duplicate instances | Two processes on one SQLite file can corrupt state or lose writes. Run exactly one replica per database. |
| Supply chain | `Cargo.lock` is committed and CI/Docker build with `--locked`. Review dependency updates; the attack surface is serenity, reqwest/rustls, rusqlite and the parsing crates. |

### Explicitly out of scope

- Vulnerabilities in Discord's API or client.
- Host, Docker daemon, orchestrator or reverse-proxy compromise.
- Rust toolchain and upstream crate vulnerabilities (report upstream; we will bump).
- Social engineering of server staff.
- Denial of service through Discord rate limits or gateway disconnects.
- Physical access to the host or database file.

## Secret handling

- Provide `DISCORD_TOKEN` (and optional API keys) through environment variables or an orchestrator secret store. `config.yaml` is supported but discouraged for secrets.
- Never commit `.env`, `config.yaml`, database files or backups. `.gitignore` covers `.env`, `config.yaml`, `*.db`, `*.db-wal`, `*.db-shm` and `/data`.
- `.env` should be mode `0600`, owned by the runtime user.
- Tokens are never written to SQLite, logs or error messages. Do not add logging that prints `Secret::expose()`.
- Rotate the token after any suspected exposure and restart the process.

## Data retention

| Data | Where | Retention |
|---|---|---|
| Moderation cases, warnings | `cases` | Indefinite (audit trail). |
| Automod evidence (message content, author, channel) | `evidence` | Indefinite; delete rows or the database to purge. |
| Reports and suggestions | `reports`, `suggestions` | Indefinite. |
| Link archive | `links` | Indefinite. |
| Forwarded-message tracking | `tracked_messages` | Deleted after `message_ttl_hours`. |
| Intel API cache | `intel_cache` | Until expiry, then garbage-collected. |
| CTF flags | `ctf_challenges.flag_hash` | Until the challenge is deleted; hashes only. |

There is no automatic retention policy for evidence and reports: operators are responsible for complying with their community's privacy rules. Backups inherit the retention of the data they contain.

## Hardening checklist

- [ ] Run as a non-root user (the image uses UID 10001), read-only root filesystem, `cap_drop: ALL`, `no-new-privileges`.
- [ ] Keep `/metrics` on a private network; bind the HTTP server to loopback when possible.
- [ ] Grant only the documented permissions; never `Administrator`.
- [ ] Keep the bot role above quarantine/verify roles; audit role changes.
- [ ] `RUST_LOG=info` in production; ship logs to a collector with retention limits.
- [ ] Back up with `sqlite3 .backup`, encrypt off-host copies, and test restores.
- [ ] Pin image tags/digests and rebuild on dependency advisories.
- [ ] Enable `ENABLE_MEMBER_INTENT` only when the anti-raid gate is in use.
- [ ] Restrict who can run `/config` and `/mod`.
- [ ] Review the [deployment checklist](docs/DEPLOYMENT.md) before going live.
