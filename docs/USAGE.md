# CipherBot — Command Reference

Complete reference for every slash command and subcommand. For installation, configuration and operations, see the [README](../README.md).

- All commands are **guild-scoped** and re-registered on startup and on guild join.
- Option tables list the registered signature. Discord always shows the authoritative option list (required vs optional) in the client.
- Output visibility is called out per command. Staff-only output (warnings, cases, reports, evidence, config, ticket queue), CTF create/delete/solve replies, `/intel` replies, `/tool encode|decode|hash|ipcalc|timestamp|password|jwt`, `/recon`, `/start` and **every** `/learn` reply are **ephemeral**. `/poll` posts one public message whose embeds are edited in place on each vote (no extra messages). Moderation confirmations such as warn/kick/ban, `/tag get|list`, `/links`, profiles and `/poll results` reply publicly.
- Permissions are re-checked in code for every sensitive action. Server administrators bypass the checks; `default_member_permissions` on the command is cosmetic.

---

## Conventions

### Permissions used

| Permission | Commands |
|---|---|
| `Moderate Members` | `/mod warn`, `/mod warnings`, `/mod timeout`, `/mod untimeout`, `/mod case`, `/mod reports`, `/mod report-close`, `/mod evidence`, `/ticket list` |
| `Kick Members` | `/mod kick` |
| `Ban Members` | `/mod ban`, `/mod unban` |
| `Manage Messages` | `/mod purge`, `/tag add`, `/tag remove`, `/poll close` (or poll author), `/ticket close` (or opener) |
| `Manage Channels` | `/mod slowmode`, `/mod lock`, `/mod unlock` |
| `Manage Roles` | `/mod quarantine`, `/mod unquarantine` |
| `Manage Guild` | `/mod raidmode`, `/mod verify-panel`, `/config`, `/ticket panel` |
| `Manage Guild` or `Manage Messages` | `/ctf create`, `/ctf delete` |
| Ticket staff role | `/ticket list`, `/ticket close` (or opener) |
| none | `/start`, `/poll create`, `/poll results`, `/tag get`, `/tag list`, `/recon`, `/intel`, `/tool`, `/learn`, `/links`, `/profile`, `/leaderboard`, `/report`, `/suggest`, `/help`, `/stats`, `/ping`, `/about` |

Administrators bypass every check. The bot also needs the same permission for actions it performs on Discord (kick, ban, manage channels/roles, moderate members); the error names the missing bot permission.

`/mod` actions post a modlog embed when a modlog channel is configured. Punitive actions create a case (see each subcommand).

### Durations

Timeout accepts compact durations: `60s`, `10m`, `2h`, `7d`, up to Discord's 28-day maximum. A bare number is interpreted as seconds. Slowmode takes an integer number of seconds from `0` (off) to `21600` (6 hours).

### IDs and mentions

User options accept a mention or an ID; `unban` takes a raw user ID because the user is no longer a member. Channel and role options accept a mention, an ID or a name where Discord resolves it. `/config set` also accepts `<#channel>` and `<@&role>` mentions.

### Errors

Failures are ephemeral and carry a human-readable reason (`Missing Permissions`, unknown case, invalid duration, external API unavailable). Every interaction is acknowledged first, so no command times out silently.

---

## `/start` — interactive guide

No options. Opens an **ephemeral** hub with 14 button panels besides Home: Modération, CTF, Renseignement, Recon, Outils, Apprendre, Liens, Profil, Sondages, Tags, Tickets, Config, Aide and Fermer.

- Every panel is addressed by a fixed `start:<slug>` custom id (for example `start:home`, `start:recon`, `start:close`), so navigation is stateless and survives restarts.
- Buttons edit the same ephemeral message in place; **Fermer** deletes it (or replaces it with a short farewell if Discord refuses).
- **Modération** requires `Moderate Members`; **Config** requires `Manage Guild`. The check runs on every click, not only when the panel is drawn; administrators bypass it. A denial replies with an ephemeral explanation.
- Content is documentation only: `/start` never performs a moderation, config or intel action.

Example: `/start` → click **Recon** → click **Accueil** → click **Fermer**.

---

## `/mod` — moderation

| Subcommand | Purpose |
|---|---|
| `warn` | Record a warning and open a case. |
| `warnings` | List a member's active warnings. |
| `timeout` | Apply a communication timeout. |
| `untimeout` | Remove an active timeout. |
| `kick` | Kick a member. |
| `ban` | Ban a member, optionally deleting recent messages. |
| `unban` | Revoke a ban by user ID. |
| `purge` | Bulk-delete recent messages. |
| `slowmode` | Set channel slowmode. |
| `lock` | Deny `Send Messages` for `@everyone`. |
| `unlock` | Remove the `@everyone` send denial. |
| `case` | View, list or close moderation cases. |
| `raidmode` | Toggle the anti-raid join gate. |
| `quarantine` | Apply the quarantine role and remove the verify role. |
| `unquarantine` | Remove the quarantine role and restore the verify role. |
| `verify-panel` | Set the verify role and post the verification button panel. |
| `reports` | Read the `/report` staff queue. |
| `report-close` | Mark a report as handled. |
| `evidence` | Read recently captured automod/moderation evidence. |

**Permission:** per subcommand (table above); every sensitive action is re-checked in code.

### `/mod warn`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member to warn. |
| `reason` | yes | Why the warning was issued (max 400 chars). |
| `evidence` | no | Optional evidence: message link or note (max 400 chars). |

Creates a case (`kind = warn`), stores the reason and optional evidence, and posts a modlog embed. Warnings are counted per guild and per member. The confirmation is public.

Example: `/mod warn user:@mallory reason:"posting leaked credentials" evidence:"https://discord.com/channels/..."`.

Notes: warnings do not expire automatically; they are history, not an automatic punishment. Use `/mod case close` to deactivate a case.

### `/mod warnings`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member whose active warnings to show. |

Ephemeral. Shows up to 10 active `warn` cases with case number, moderator, reason and date.

### `/mod timeout`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member to time out. |
| `duration` | yes | `60s`, `10m`, `2h`, `7d`, up to 28 days. |
| `reason` | yes | Audit reason (max 400 chars). |

Requires `Moderate Members`. Creates a `timeout` case with an expiry and posts the modlog. The confirmation is public.

Example: `/mod timeout user:@spammer duration:1h reason:"flooding #general"`.

Notes: the bot must have `Moderate Members` and its role must be above the target. Durations below 60 seconds are raised to 60 seconds; Discord caps timeouts at 28 days.

### `/mod untimeout`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member whose timeout to clear. |
| `reason` | no | Audit reason (max 400 chars). |

Clears the timeout and records a separate `timeout_end` case (it does not edit the original timeout case). The confirmation is public.

### `/mod kick`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member to kick. |
| `reason` | yes | Audit reason (max 400 chars). |

Requires `Kick Members`. Kicks the member, then creates a `kick` case and posts the modlog. The confirmation is public.

### `/mod ban`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member or user ID to ban. |
| `reason` | yes | Audit reason (max 400 chars). |
| `delete_days` | no | Delete this many days of the member's messages (0–7, default 0). |

Requires `Ban Members`. Applies the ban, creates a `ban` case, and posts the modlog. The confirmation is public.

### `/mod unban`

| Option | Required | Description |
|---|---|---|
| `user_id` | yes | Raw ID of the banned user. |
| `reason` | yes | Audit reason (max 400 chars). |

Requires `Ban Members`. Use the ID (right-click user → Copy User ID) since the user is not in the member list. Creates an `unban` case.

### `/mod purge`

| Option | Required | Description |
|---|---|---|
| `count` | yes | Number of messages to delete (1–100). |
| `user` | no | Only delete messages from this member. |
| `contains` | no | Only delete messages containing this text (max 200 chars). |

Requires `Manage Messages`. Scans the most recent `count` messages in the channel, skips pinned messages and anything older than 14 days (Discord limit), then deletes the matches in batches. Up to 25 deleted-message snapshots are stored as evidence with credential-like content redacted. The result is ephemeral.

### `/mod slowmode`

| Option | Required | Description |
|---|---|---|
| `seconds` | yes | 0 (off) to 21600 (6 h). |
| `channel` | no | Target text channel; defaults to the current one. |

Requires `Manage Channels`. No `reason` option exists; the action is still written to the modlog. The confirmation is public.

### `/mod lock` / `/mod unlock`

| Option | Required | Description |
|---|---|---|
| `channel` | no | Target text channel; defaults to the current one. |
| `reason` | no | Audit reason (max 400 chars). |

Requires `Manage Channels`. `lock` merges a `Send Messages` deny for `@everyone` into the channel's existing permission overwrites (other overwrites are preserved); `unlock` removes that deny. No case is created; both post a modlog embed.

### `/mod case view|list|close`

| Option | Required | Description |
|---|---|---|
| `view.id` | yes | Case number shown in the modlog (≥ 1). |
| `list.user` | no | Filter by member. |
| `list.limit` | no | Page size (1–50, default 10). |
| `close.id` | yes | Case number to deactivate (≥ 1). |

Requires `Moderate Members`. Cases are numbered per guild starting at 1 and shown newest first.

- `view` shows kind, target, moderator, reason, evidence reference, created timestamp, expiry (when set) and active/inactive status.
- `list` shows case number, kind, target, date and a truncated reason.
- `close` marks a case inactive, so it stops counting as active (for example, an active warning).

All three replies are ephemeral.

Examples: `/mod case view id:42`, `/mod case list user:@mallory limit:10`, `/mod case close id:42`.

### `/mod raidmode on|off`

| Option | Required | Description |
|---|---|---|
| `state` | yes | `on` or `off`. |

Requires `Manage Guild`. Persists the anti-raid join gate in guild settings, posts a confirmation and writes a modlog embed when a modlog channel is configured. This command never kicks or bans anyone.

When enabled, joining human accounts are checked against the raid thresholds: accounts younger than `raid_min_account_age_hours` (default 24) are suspicious, and if `raid_require_avatar` is `true` (default `false`) accounts without an avatar are suspicious too. Suspicious joins get the quarantine role **only when a quarantine role is configured**; without one, the bot posts a warning in the modlog instead. Independently of `raidmode`, more than 10 joins in 60 seconds raises one "possible raid" modlog alert per 5 minutes.

This requires the **Server Members Intent**: enable it in the Discord Developer Portal and set `ENABLE_MEMBER_INTENT=true`. Without it Discord sends no join events, so the gate cannot act.

### `/mod quarantine`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member to quarantine. |
| `reason` | no | Audit reason (max 400 chars). |

Requires `Manage Roles`, a configured quarantine role (`/config set key:quarantine_role`), and a bot role above the target. Adds the quarantine role, removes the verify role when one is configured, creates a `quarantine` case and posts the modlog. The confirmation is public.

> The quarantine role must itself deny `Send Messages` (and optionally `View Channels`) through channel or category permission overwrites. Adding a role does not remove permissions granted by a member's other roles; Discord role overwrites only take effect when the role is configured that way.

> Role hierarchy: the CipherBot role must sit **above** the quarantine role, otherwise Discord returns `Missing Permissions`.

### `/mod unquarantine`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member to release. |
| `reason` | no | Audit reason (max 400 chars). |

Removes the quarantine role, re-adds the verify role when configured, and creates an `unquarantine` case. Requires `Manage Roles` and the configured quarantine role.

### `/mod verify-panel`

| Option | Required | Description |
|---|---|---|
| `role` | yes | Role granted when a member clicks **Verify**. |
| `channel` | no | Where to post the panel; defaults to the current channel. |

Requires `Manage Guild`. Saves the role as the guild verify role and posts an embed with a **Verify** button (`custom_id: verify:grant`). The panel command reply is ephemeral; pressing the button grants the role publicly.

The button is stateless: it checks the current roles and is idempotent (pressing twice is harmless).

### `/mod reports`

| Option | Required | Description |
|---|---|---|
| `status` | no | `Open` (default), `Handled` or `All` (stored values: `open`, `handled`, `all`). |
| `limit` | no | Maximum results (1–25, default 10). |

Requires `Moderate Members`. Ephemeral. Lists reports filed with `/report`, newest first, with reporter, target, time, attached message ID and reason. `/report` always creates a report with status `open`; `Handled` filters rows whose stored status is `handled`.

### `/mod report-close`

| Option | Required | Description |
|---|---|---|
| `id` | yes | Report ID shown by `/mod reports`. |

Requires `Moderate Members`. Ephemeral. Sets the report status to `handled` and records the moderator and timestamp. Re-running it on an already handled report returns a friendly error.

### `/mod evidence`

| Option | Required | Description |
|---|---|---|
| `limit` | no | Maximum results (1–25, default 10). |

Requires `Moderate Members`. Ephemeral. Shows the most recent evidence rows (ID, reason, date, author, channel, content) captured by automod, `/mod purge` and the flag watcher. Automod and purge redact credential-shaped content, and the flag watcher redacts flag-shaped content, before storing; evidence never leaves your SQLite database.

### Automod flow

Automod runs on every message before the flag watcher and the link pipeline:

| Check | Default | Behaviour |
|---|---|---|
| Invite links | on | Deletes Discord invite links. |
| Phishing domains | on | Deletes links matching the refreshed feed. |
| Mass mentions | on | Deletes messages with 8+ mentions or `@everyone`. |
| Duplicate spam | on | Deletes the same content posted 3 times within 30 seconds. |
| Excessive caps | on | Deletes messages with 12+ letters that are at least 70% uppercase. |
| Token leaks | on | Deletes messages containing bot tokens, cloud keys, private keys or Slack tokens. |

Members with `Manage Messages` or `Administrator` bypass automod. A hit deletes the message, writes an `evidence` row (content, author, channel, reason) and posts a modlog embed when a modlog channel is set. Phishing, mass mentions and token leaks also open an `automod` case; invite, caps and duplicate hits do not. Token-like content is redacted before evidence and modlog. Evidence stays in your SQLite file and is never sent to a third party.

Toggles: `/config automod feature:<name> enabled:<true|false>`. All six checks are enabled by default.

### Anti-raid and verification

1. Staff enables the Server Members Intent in the Developer Portal and sets `ENABLE_MEMBER_INTENT=true`.
2. `/mod raidmode state:on` activates the join gate; thresholds are set with `/config set key:raid_min_account_age_hours` and `key:raid_require_avatar` (defaults: 24 hours, avatar not required).
3. Suspicious joins are quarantined when a quarantine role is configured, and logged; staff review and either release (`/mod unquarantine`) or ban. Without a quarantine role the bot only alerts.
4. For normal onboarding, `/mod verify-panel role:@Verified` posts the button that grants the verify role.

---

## `/ctf` — challenges

Lifecycle: **create → solve → scoreboard**.

| Subcommand | Purpose |
|---|---|
| `create` | Register a challenge with a hashed flag. |
| `list` | List challenges. |
| `info` | Show one challenge, including solve count and first blood. |
| `solve` | Submit a flag. |
| `scoreboard` | Rank solvers. |
| `delete` | Remove a challenge and its solves. |

**Permission:** `create` and `delete` require `Manage Guild` or `Manage Messages` (administrators bypass); `list`, `info`, `solve` and `scoreboard` are open to members.

### `/ctf create`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Unique per guild, case-insensitive (max 64 chars). |
| `category` | yes | `Web`, `Pwn`, `Crypto`, `Reverse engineering`, `Forensics`, `OSINT`, `Network`, `Cloud` or `Misc`. |
| `points` | yes | Point value awarded to solvers (10–1000). |
| `flag` | yes | The secret flag (max 200 chars); stored only as a hash. |
| `channel` | no | Channel where the challenge announcement is posted. |
| `description` | no | Public description (max 400 chars); never put the flag here. |

The flag is hashed with SHA-256 in a guild-scoped namespace (`guild_id:flag`) before storage; the plaintext is discarded immediately and never logged. The confirmation is ephemeral.

If `channel` is provided, the challenge announcement is posted there. The `/config ctf-channel` setting does **not** receive announcements: it marks the channel where flag-shaped messages are allowed (the flag watcher ignores it). Leaving `channel` empty creates the challenge silently.

Example: `/ctf create name:"SQLi 101" category:Web points:100 flag:"flag{...}" channel:#ctf`.

### `/ctf list`

No options. Public. Shows up to 20 challenges with category, points and solve count.

### `/ctf info`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Challenge name (max 64 chars). |

Public. Shows the description, points, category, solve count, first blood and channel. Never shows the flag or its hash.

Example: `/ctf info name:"SQLi 101"`.

### `/ctf solve`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Challenge name (max 64 chars). |
| `flag` | yes | Submitted flag (max 200 chars). |

The response is **ephemeral**: correct/incorrect, points awarded, and first-blood status. Submissions are compared in memory against the stored hash using a constant-time comparison and are never logged. A member can solve each challenge once; duplicate solves are rejected, and submissions are limited to one every 3 seconds per user.

A correct solve awards the challenge's full points to the CTF scoreboard and **half the points (minimum 1) as community XP**.

Example: `/ctf solve name:"SQLi 101" flag:"flag{...}"` (use the real event flag, not this placeholder).

### `/ctf scoreboard`

| Option | Required | Description |
|---|---|---|
| `limit` | no | Number of entries (1–25, default 10). |

Public. Ranks members by total CTF points; ties break on the earliest final solve. Also feeds the `ctf` metric of `/leaderboard`.

### `/ctf delete`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Challenge to delete (max 64 chars). |

Staff only (`Manage Guild` or `Manage Messages`). Removes the challenge and its solve rows, which updates scores accordingly. Ephemeral. Prefer deleting test challenges before an event starts.

---

## `/intel` — threat intelligence

| Subcommand | Purpose | Source | Key |
|---|---|---|---|
| `cve` | Look up one CVE. | NVD | optional |
| `cve-latest` | Recent CVEs, optionally by severity. | NVD | optional |
| `cve-search` | Keyword search of CVE descriptions. | NVD | optional |
| `mitre` | Search embedded ATT&CK techniques. | bundled content | none |
| `ip` | IP geolocation, ASN and network ownership. | ipinfo + RDAP | optional |
| `dns` | Resolve A/AAAA/MX/TXT/NS/CNAME/SOA records. | Cloudflare DoH | none |
| `domain` | Domain registration data. | RDAP | none |
| `hash` | Identify a digest and check reputation. | local + VirusTotal | optional |
| `vt` | VirusTotal lookup for file, URL, domain or IP. | VirusTotal | required |

**Permission:** none — available to all members. Replies are ephemeral. Lookups are cached in SQLite; `/intel cve`, `cve-latest`, `cve-search`, `ip`, `dns`, `domain` and `vt` share a 4-second per-user cooldown.

### `/intel cve`

| Option | Required | Description |
|---|---|---|
| `id` | yes | CVE ID, e.g. `CVE-2024-3094` (12–20 chars). |

Shows description, CVSS score/severity, published/modified dates, weaknesses and references. Without `NVD_API_KEY` the request uses the public rate limit and may report a temporary failure.

Example: `/intel cve id:CVE-2024-3094`.

### `/intel cve-latest`

| Option | Required | Description |
|---|---|---|
| `severity` | no | `critical`, `high`, `medium`, `low` or `any` (default `any`). |
| `limit` | no | Number of CVEs (1–10, default 5). |

Recent CVEs sorted by publication date. The severity filter is applied after fetching, so a narrow filter can return fewer than `limit` rows.

### `/intel cve-search`

| Option | Required | Description |
|---|---|---|
| `query` | yes | Keyword, e.g. `log4j` or `exchange` (2–120 chars). |

Searches NVD CVE descriptions by keyword and returns up to 8 matches.

Example: `/intel cve-search query:log4j`.

### `/intel mitre`

| Option | Required | Description |
|---|---|---|
| `query` | yes | Technique ID or keyword, e.g. `T1059` or `phishing` (max 100 chars). |

Searches the ATT&CK techniques bundled into the binary — no network call. Shows the best match with tactic, description, detection notes and a reference URL, plus up to four other matches.

### `/intel ip`

| Option | Required | Description |
|---|---|---|
| `address` | yes | IPv4 or IPv6 address, e.g. `1.1.1.1` (max 45 chars). |

Queries ipinfo and RDAP in parallel (an `IPINFO_TOKEN` raises ipinfo limits), then merges location, ASN, organisation, country and network registration data. If both sources fail, the command returns an ephemeral error. Addresses reserved for private use may simply return no data.

### `/intel dns`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Domain without scheme, e.g. `example.com` (max 253 chars). |
| `type` | no | `A` (default), `AAAA`, `MX`, `TXT`, `NS`, `CNAME` or `SOA`. |

Resolves records over **Cloudflare DNS-over-HTTPS** (`cloudflare-dns.com`) using the JSON API. The name is validated as a bare hostname; passing an IP returns a hint to use `/intel ip`. Responses are cached for one hour and the reply lists each answer with type, TTL and data (TXT values are fenced/escaped). NXDOMAIN and SERVFAIL are reported as an explicit status rather than a crash.

Example: `/intel dns name:example.com type:MX`.

### `/intel domain`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Domain without scheme, e.g. `example.com` (max 253 chars). |

RDAP registrar, creation/expiry dates and status flags.

### `/intel hash`

| Option | Required | Description |
|---|---|---|
| `value` | yes | MD5, SHA-1, SHA-224, SHA-256, SHA-384 or SHA-512 hex digest (32–128 chars). |

Identifies the digest locally by length and, when `VT_API_KEY` is configured, adds the VirusTotal detection ratio, per-engine counts, last analysis date and known file names. Without a key it still reports the digest type and explains that reputation data is unavailable.

### `/intel vt`

| Option | Required | Description |
|---|---|---|
| `query` | yes | Hash, URL, domain or IP address (max 2048 chars). |
| `type` | yes | `file`, `url`, `domain` or `ip`; determines how the query is validated. |

General VirusTotal lookup. Requires `VT_API_KEY`; without it the command explains how to configure one. Results are cached in `intel_cache` until their expiry to respect free-tier quotas.

> All intel calls use fixed hosts, a 5-second timeout, and never send user or guild data — only the indicator being looked up.

---

## `/tool` — tradecraft

| Subcommand | Purpose | Reply |
|---|---|---|
| `encode` | Encode text (Base64, Base64URL, hex, URL, ROT13, binary, Morse). | ephemeral |
| `decode` | Decode text in the same schemes. | ephemeral |
| `hashid` | Guess hash, digest or token formats. | public |
| `entropy` | Shannon entropy, strength and crack-time estimate. | public |
| `cvss` | Score a CVSS 3.0/3.1 vector. | public |
| `password` | Check password strength and known breaches (HIBP k-anonymity). | ephemeral |
| `dork` | Generate search dorks for an authorised target. | public |
| `jwt` | Decode a JWT header and payload. | ephemeral |
| `hash` | Compute an MD5, SHA-1, SHA-256 or SHA-512 hex digest. | ephemeral |
| `ipcalc` | Analyse an IPv4 or IPv6 CIDR block. | ephemeral |
| `timestamp` | Convert a timestamp to UTC, unix time and RFC 3339. | ephemeral |

**Permission:** none.

### `/tool encode` / `/tool decode`

| Option | Required | Description |
|---|---|---|
| `scheme` | yes | `base64`, `base64url`, `hex`, `url`, `rot13`, `binary` or `morse`. |
| `input` | yes | Input string (max 1900 chars). |

Output is a code block, ephemeral. Invalid input returns a clear decoding error rather than a panic.

Example: `/tool encode scheme:base64 input:"hello"` → `aGVsbG8=`.

### `/tool hashid`

| Option | Required | Description |
|---|---|---|
| `value` | yes | Hash or token to identify (max 500 chars). |

Suggests likely formats from length and character set (MD5, SHA-1, SHA-256, SHA-512, bcrypt, JWT, …) with a confidence note. It is a heuristic, not proof. Public.

### `/tool entropy`

| Option | Required | Description |
|---|---|---|
| `value` | yes | Text to analyse (max 500 chars). |

Reports length, character-set size, Shannon entropy in bits per character, estimated total bits, a strength label and a crack-time estimate at 10^10 guesses/second. The input is never echoed back. Public.

### `/tool cvss`

| Option | Required | Description |
|---|---|---|
| `vector` | yes | CVSS 3.0/3.1 vector, e.g. `CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H` (max 200 chars). |

Returns the base score, severity and each metric. Malformed vectors are rejected with the offending metric. Public.

### `/tool password`

| Option | Required | Description |
|---|---|---|
| `value` | yes | Password to check (max 200 chars). |

Computes strength and crack time locally, then hashes the password with SHA-1 and sends **only the first 5 uppercase hex characters** to the Have I Been Pwned range API (k-anonymity); the suffix comparison happens locally. The response is ephemeral and reports the breach count. Limited to one check every 3 seconds per user. Do not type a real, in-use credential into Discord — the message still travels through Discord's infrastructure.

### `/tool dork`

| Option | Required | Description |
|---|---|---|
| `target` | yes | Domain or URL you are authorised to test, e.g. `example.com` (max 253 chars). |

Returns a handful of search-operator templates (e.g. exposed documents, directory listings). Public. For authorised reconnaissance only.

### `/tool jwt`

| Option | Required | Description |
|---|---|---|
| `token` | yes | JWT to decode (max 2000 chars). |

Ephemeral. Decodes the header and payload (Base64URL) and shows expiry/issued-at in human form. It does **not** verify the signature and makes no network call — never treat the output as proof of validity.

### `/tool hash`

| Option | Required | Description |
|---|---|---|
| `algorithm` | yes | `md5`, `sha1`, `sha256` or `sha512`. |
| `value` | yes | Text to hash (max 4,000 chars). |

Returns the lowercase hex digest in a code block, ephemeral, with the input byte length. Hashing is local; nothing is sent anywhere. Case-insensitive algorithm names are accepted internally, but Discord only offers the four choices.

Example: `/tool hash algorithm:sha256 value:"abc"`.

### `/tool ipcalc`

| Option | Required | Description |
|---|---|---|
| `cidr` | yes | CIDR block, e.g. `10.0.0.0/24` or `2001:db8::/32` (max 50 chars). |

Ephemeral. Reports the network address, prefix, netmask/wildcard, broadcast (IPv4), first/last usable host, address count, scope and classification. Host bits are cleared when you paste `10.0.0.5/24`, with a note saying so; IPv6 notes that it has no broadcast address. Invalid CIDR returns a clear ephemeral error.

Example: `/tool ipcalc cidr:192.168.1.0/24`.

### `/tool timestamp`

| Option | Required | Description |
|---|---|---|
| `value` | yes | `now`, unix seconds, unix milliseconds or an RFC 3339 timestamp (max 64 chars). |

Ephemeral. Shows the UTC time, unix seconds, unix milliseconds, weekday, RFC 3339 form and how the input was parsed. Values are auto-detected: `now`, 10-digit seconds, 13-digit milliseconds and RFC 3339 are all accepted.

Example: `/tool timestamp value:1700000000`.

---

## `/learn` — learning

| Subcommand | Purpose |
|---|---|
| `quiz` | Answer the daily multiple-choice question. |
| `glossary` | Look up a security term. |
| `roadmap` | Show a learning path. |
| `resources` | Curated resources, optionally filtered. |
| `tip` | Tip of the day. |

**Permission:** none. All content is compiled into the binary and works offline.

**Visibility:** every reply is **ephemeral** — quiz, glossary, roadmap, resources and tip are visible only to the caller. This is deliberate anti-spam: a learning command should not flood the channel, and quiz answers must not leak to other members. Buttons still work normally; answering a quiz updates stats and streak as before.

### `/learn quiz`

| Option | Required | Description |
|---|---|---|
| `category` | no | `Cryptography`, `Web`, `Pwn`, `Reverse Engineering`, `Forensics`, `Network`, `Blue Team`, `OSINT`, `Cloud` or `Misc` (default: today's pick). |

Posts an **ephemeral** question with four buttons (`custom_id: quiz:<question_id>:<choice_index>`). Answering updates `quiz_correct` / `quiz_total` and the daily streak; the correct answer and a short explanation are shown only to the clicker. A member can answer once per UTC day. A correct answer awards +10 XP; an attempt awards +2 XP. Stale buttons answer "This quiz question no longer exists."

Example: `/learn quiz category:Web`.

### `/learn glossary`

| Option | Required | Description |
|---|---|---|
| `term` | yes | Term or keyword to look up (max 100 chars). |

Case-insensitive search of the bundled glossary; returns up to 5 matches with definitions and categories. Ephemeral, including the "no match" warning. Unknown terms suggest a shorter keyword.

### `/learn roadmap`

| Option | Required | Description |
|---|---|---|
| `track` | yes | `Web Exploitation`, `Binary Exploitation`, `Cryptography`, `Digital Forensics`, `Blue Team / DFIR`, `Red Team / Pentest`, `Cloud Security` or `OSINT`. |

Ephemeral. Shows a staged path (fundamentals → practice → advanced) with resource pointers.

Example: `/learn roadmap track:"Blue Team / DFIR"`.

### `/learn resources`

| Option | Required | Description |
|---|---|---|
| `kind` | no | `Course`, `Book`, `Blog`, `Tool`, `Lab`, `Certification`, `Cheatsheet` or `Podcast`. |
| `search` | no | Only show resources matching this text (max 100 chars). |

Ephemeral. Lists curated resources bundled with the bot, with free/paid markers.

### `/learn tip`

No options. Ephemeral. Posts the tip of the day (deterministic per UTC day) with its category.

---

## `/poll` — button polls

| Subcommand | Purpose |
|---|---|
| `create` | Post a public poll with live vote buttons. |
| `close` | Close a poll (author or staff). |
| `results` | Show the current results. |

**Permission:** none to create or view results; closing requires the poll author, `Manage Messages` or `Administrator`. Poll ids are per-guild integers shown in the footer (`Poll #N`).

### `/poll create`

| Option | Required | Description |
|---|---|---|
| `question` | yes | Poll question (max 200 chars). |
| `option1` | yes | First option (max 80 chars). |
| `option2` | yes | Second option (max 80 chars). |
| `option3` | no | Third option (max 80 chars). |
| `option4` | no | Fourth option (max 80 chars). |
| `option5` | no | Fifth option (max 80 chars). |

Posts a **public** embed in the current channel with one row of numbered vote buttons (`poll:vote:<id>:<index>`) and a red **Clôturer** button (`poll:close:<id>`). Blank options are ignored; at least two distinct (case-insensitive) options are required. The command itself replies ephemerally with the poll number and a jump link.

Votes **edit the public message in place**: ten-cell proportional bars, raw counts, percentages and the total. No extra message is posted per vote. A member has one vote and may change it; the database stores one row per `(poll_id, user_id)`. Closed polls reject votes with an ephemeral "Sondage terminé."

Example: `/poll create question:"Best scanner?" option1:nmap option2:rustscan option3:masscan`.

### `/poll close`

| Option | Required | Description |
|---|---|---|
| `id` | yes | Poll number (≥ 1). |

Author, `Manage Messages` or `Administrator`. Flips `closed_at`, removes the buttons from the public message and marks the embed "Sondage terminé". Closing an already closed poll is harmless and reported as such. Replies ephemerally. There is **no duration or auto-close** — a poll stays open until someone closes it.

Example: `/poll close id:3`.

### `/poll results`

| Option | Required | Description |
|---|---|---|
| `id` | yes | Poll number (≥ 1). |

Public. Renders the same live bar embed without changing the poll. Useful when the original message scrolled away.

Example: `/poll results id:3`.

---

## `/tag` — snippets and FAQ

| Subcommand | Purpose | Reply |
|---|---|---|
| `get` | Show a saved tag by name. | public |
| `list` | List every tag saved in this server. | public |
| `add` | Save or replace a tag (staff). | ephemeral |
| `remove` | Delete a tag (staff). | ephemeral |

**Permission:** `get` and `list` are open to everyone; `add` and `remove` require `Manage Messages` (administrators bypass).

### `/tag get`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Tag name (max 32 chars). |

Public embed with the tag content, author ID and use count. Names are case-insensitive and normalized to lowercase; each read increments `uses`. Unknown names suggest `/tag list`.

Example: `/tag get name:faq`.

### `/tag list`

No options. Public. Shows the number of saved tags and up to 50 names; longer lists say only the first 50 are shown.

### `/tag add`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Tag name (max 32 chars). |
| `content` | yes | Text shown by `/tag get` (max 1,500 chars). |

Requires `Manage Messages`. Upserts the tag: an existing name is replaced and keeps its use counter; `created_by` tracks the author of the current content. Names are trimmed, lowercased and must be 1–32 characters from `a-z`, `0-9`, `_` or `-` — anything else is rejected (no silent transliteration). Empty content is rejected. Ephemeral confirmation.

Example: `/tag add name:faq content:"Read the pinned messages first."`.

### `/tag remove`

| Option | Required | Description |
|---|---|---|
| `name` | yes | Tag name (max 32 chars). |

Requires `Manage Messages`. Deletes the tag; deleting a missing tag returns an ephemeral error. Ephemeral confirmation.

---

## `/ticket` — private support threads

| Subcommand | Purpose |
|---|---|
| `panel` | Post the public support-ticket panel. |
| `close` | Close the ticket thread you are in. |
| `list` | List open tickets in this server. |

**Permission:** `panel` requires `Manage Guild`; `close` is allowed for the opener, `Manage Messages`, `Administrator` or the configured ticket staff role; `list` requires `Moderate Members` or the ticket staff role.

### `/ticket panel`

| Option | Required | Description |
|---|---|---|
| `channel` | no | Text channel to post the panel in (default: current). |
| `role` | no | Staff role notified for new tickets; saved as `ticket_role`. |

Posts a public embed plus a **📩 Ouvrir un ticket** button (`custom_id: ticket:open`). The button and panel are stateless: they keep working after a restart. The command reply is ephemeral and names the target channel.

The panel text mentions the role when one is configured. `/config set key:ticket_role value:<role ID>` sets the same value without reposting the panel.

Example: `/ticket panel channel:#support role:@Staff`.

### Opening a ticket (button)

Clicking **Ouvrir un ticket** creates a **private thread** named `ticket-<username>` in the panel channel (auto-archive: 1 day) and adds the opener.

- **One open ticket per member.** Clicking again while one is open replies ephemerally with a link to the existing thread instead of creating a second one.
- The intro embeds the opener mention, but mentions in it never notify. A separate one-line notice is posted in the panel channel and deliberately pings the configured ticket role, so staff get an entry point even without `Manage Threads`.
- A **Close ticket** button (`custom_id: ticket:close`) is posted in the thread.
- Failure to create the thread is reported ephemerally; the exact Discord error is only logged.

> **Staff visibility caveat:** Discord only lets roles with `Manage Threads` see private threads they were not explicitly added to. Grant `Manage Threads` to the staff role, or add staff members to the thread manually. The configured ticket role is pinged in the panel-channel notice but is **not** added to the thread. Without `Manage Threads`, staff who did not open the ticket will not see it.

### `/ticket close`

No options. Must be run inside a ticket thread. The opener, a member with `Manage Messages`/`Administrator`, or a holder of the ticket staff role may close it. The database row is marked `closed` first, then the thread is archived and locked; if archiving fails the ticket stays closed and the reply warns about the archive error. Ephemeral confirmation. Closing an already closed ticket returns a friendly error.

### `/ticket list`

No options. Ephemeral. Requires `Moderate Members` or the ticket staff role. Shows the total number of open tickets and up to 25 of the oldest, with thread mention, opener and creation date.

---

## `/recon` — passive OSINT

| Subcommand | Purpose | Source |
|---|---|---|
| `subdomains` | Subdomains from certificate transparency. | crt.sh |
| `ip` | Ports, hostnames, CPEs and CVEs. | Shodan InternetDB |
| `wayback` | Historical snapshots. | Wayback Machine CDX |
| `urlscan` | Recent public scans. | urlscan.io |

**Permission:** none. Replies are ephemeral. A 5-second per-user cooldown applies to every subcommand.

> **Passive only.** `/recon` never scans, connects to or fetches anything from the target: every request goes to one of the fixed third-party APIs above, and the user input only appears in that API's query string. There is no generic URL fetcher and no active scanning mode. Do not use the results to justify intrusive testing you are not authorised to perform.

Domains must be bare hostnames (no scheme, path, wildcard, underscore or single label); an IP passed to a domain subcommand is redirected to `/recon ip`. IP lookups accept **globally routable addresses only** — private, loopback, link-local, CGNAT, multicast, documentation and reserved ranges are refused so the command cannot be aimed at a LAN.

Results are cached in `intel_cache` (crt.sh 6 h, InternetDB 1 h, Wayback 6 h, urlscan 1 h). Upstream rate limits and outages surface as ephemeral errors, not crashes.

### `/recon subdomains`

| Option | Required | Description |
|---|---|---|
| `domain` | yes | Domain without scheme, e.g. `example.com` (max 253 chars). |

Reads crt.sh certificate transparency logs, strips wildcards, deduplicates and sorts names under the queried domain. Shows up to 200 subdomains plus whether the apex appeared and the total count.

Example: `/recon subdomains domain:example.com`.

### `/recon ip`

| Option | Required | Description |
|---|---|---|
| `address` | yes | Public IPv4 or IPv6 address (max 45 chars). |

Queries Shodan InternetDB for open ports, hostnames, CPEs, CVE ids and tags. A 404 means InternetDB has no record for that address; a 429 is reported as rate limiting. The embed links to `internetdb.shodan.io/<ip>`.

Example: `/recon ip address:1.1.1.1`.

### `/recon wayback`

| Option | Required | Description |
|---|---|---|
| `domain` | yes | Domain without scheme (max 253 chars). |
| `limit` | no | Snapshots to show (1–50, default 15). |

CDX query for `*.domain`, collapsed by URL key. Each row shows timestamp, HTTP status, MIME type and the original URL.

Example: `/recon wayback domain:example.com limit:10`.

### `/recon urlscan`

| Option | Required | Description |
|---|---|---|
| `domain` | yes | Domain without scheme (max 253 chars). |
| `limit` | no | Scans to show (1–15, default 10). |

Recent public urlscan.io results for the domain, with scan time, URL, malicious verdict count and a link to the result page. The embed turns red when at least one scan is flagged malicious.

Example: `/recon urlscan domain:example.com limit:5`.

---

## `/links` — curation and archive

| Subcommand | Purpose |
|---|---|
| `search` | Full-text/domain search in the archive. |
| `recent` | Latest archived links. |
| `stats` | Archive statistics. |

**Permission:** none. Replies are public.

### `/links search`

| Option | Required | Description |
|---|---|---|
| `query` | yes | Text or domain (max 200 chars). |
| `limit` | no | 1–25, default 10. |

Matches URL text and domain with a `LIKE` search, newest first. Output: domain, author, relative date and a jump link (preferring the thread when one exists).

Example: `/links search query:rust limit:5`.

### `/links recent`

| Option | Required | Description |
|---|---|---|
| `limit` | no | 1–25, default 10. |

Most recent archived links, newest first.

### `/links stats`

No options. Shows archived link count, contributor count and the top domain with its link count.

### Curation pipeline

Enabled only for channels whose parent category is listed in `MONITORED_CATEGORIES` (or `categories` in YAML):

- **Message with a link** → when auto-threading is on, a thread is created from the message and remembered as the channel's latest thread; the link is archived. If the same URL was already threaded, the bot replies with a link to the existing thread and deletes the new message (the duplicate is not archived).
- **Auto-threading off** → the link is still archived, but no thread is created and the message is left in the channel.
- **Message without a link** → forwarded into the channel's latest thread as `@author` + quoted content, then deleted from the parent channel. If no thread exists yet, the message is still deleted; nothing is forwarded.
- **TTL** → only forwarded copies are tracked and deleted after the configured TTL by a 60-second cleanup loop. Link messages and the archive are not TTL-deleted.

Bot and webhook messages are ignored; threads themselves are never processed. With auto-threading on, posting links keeps the parent channel clean and the discussion in threads.

### Guild overrides and precedence

Per-category YAML sets `auto_thread_links` and `message_ttl_hours` (defaults: `true`, 24 hours). Once `/config links` (or `/config set` with `auto_thread` / `link_ttl_hours`) is used, the guild-level values are marked custom and override the per-category YAML for **every** monitored category in that guild. `/config links ttl_hours` accepts 1–720 hours; the default is 24.

---

## Community

**Permission:** `/config` requires `Manage Guild`; every other command here is open to members.

### `/profile`

| Option | Required | Description |
|---|---|---|
| `user` | no | Member to inspect; defaults to you. |

Public. Shows level and XP progress, message count, quiz accuracy, current streak, CTF points and first-seen date. Active warnings are shown only to the member themselves or to staff with `Moderate Members`.

Example: `/profile user:@alice` (omit `user` for yourself).

### `/leaderboard`

| Option | Required | Description |
|---|---|---|
| `metric` | no | `XP` (default), `Quiz accuracy` or `CTF points`. |
| `limit` | no | Entries to show (1–25, default 10). |

Public. Top members for the chosen metric. The medal labels are cosmetic; the stored values are `xp`, `quiz` and `ctf`.

Example: `/leaderboard metric:"CTF points"`.

### `/report`

| Option | Required | Description |
|---|---|---|
| `user` | yes | Member being reported. |
| `reason` | yes | What happened (max 800 chars). |
| `message_id` | no | ID of the offending message (max 25 chars). |

Creates a `reports` row with status `open` and posts an embed to the modlog channel when configured. The confirmation is ephemeral so the report is not broadcast. Staff read the queue with `/mod reports`.

### `/suggest`

| Option | Required | Description |
|---|---|---|
| `text` | yes | The suggestion (max 800 chars). |

Posts a public suggestion embed with up/down vote buttons (`custom_id: vote:<up|down>:<suggestion_id>`) and replies ephemerally with the suggestion number. Votes are stored per user and update the totals; a member can change their vote.

### `/config`

| Subcommand | Purpose |
|---|---|
| `view` | Show the guild configuration. |
| `modlog` | Set the moderation log channel. |
| `ctf-channel` | Set the channel where flag-shaped messages are allowed. |
| `automod` | Toggle one automod check. |
| `verify` | Set the role granted by the verify panel. |
| `set` | Set one configuration key by name. |
| `links` | Override the link pipeline settings for this guild. |

**Permission:** `Manage Guild` for every subcommand. All replies are ephemeral.

#### `/config view`

No options. Renders the modlog channel, CTF channel, digest channel, quarantine role, verify role, ticket staff role, the six automod toggles, the anti-raid settings (enabled, minimum account age, avatar requirement) and the link pipeline settings (auto-thread, TTL).

#### `/config set`

| Option | Required | Description |
|---|---|---|
| `key` | yes | One of the keys below. |
| `value` | yes | New value (max 100 chars); IDs for channel/role keys, `true`/`false` for flags. |

Supported keys (Discord shows the picker label; the bot stores the value):

| Key | Picker label | Value format |
|---|---|---|
| `modlog_channel` | Modlog channel | Channel ID or `<#channel>` mention. |
| `ctf_channel` | CTF channel | Channel ID or mention; flags posted there are exempt from the flag watcher. |
| `digest_channel` | Digest channel | Channel ID or mention for the daily CVE digest. |
| `quarantine_role` | Quarantine role | Role ID or `<@&role>` mention. |
| `verify_role` | Verify role | Role ID or mention. |
| `ticket_role` | Ticket staff role | Role ID or mention; used for ticket pings, `/ticket list` and `/ticket close`. |
| `link_ttl_hours` | Link TTL (hours) | Integer, 1–720. Marks the guild link settings custom. |
| `auto_thread` | Auto-thread links | `true`/`false` (also `1/0`, `yes/no`, `on/off`). Marks the guild link settings custom. |
| `raid_min_account_age_hours` | Raid minimum account age (hours) | Integer, 0–720 (default 24). |
| `raid_require_avatar` | Raid require avatar | `true`/`false` (default `false`). |

Discord displays the picker labels; the bot stores the key values in the first column. `/ticket panel role:…` sets `ticket_role` too, without reposting the panel.

Examples: `/config set key:quarantine_role value:123456789012345678`, `/config set key:ticket_role value:123456789012345678`, `/config set key:link_ttl_hours value:48`, `/config set key:raid_min_account_age_hours value:72`, `/config set key:digest_channel value:123456789012345678`.

#### `/config automod`

| Option | Required | Description |
|---|---|---|
| `feature` | yes | `Invite links`, `Phishing links`, `Mass mentions`, `Duplicate spam`, `Excessive caps` or `Token leaks`. |
| `enabled` | yes | `true` to enable, `false` to disable. |

The picker labels map to the stored values `invites`, `phishing`, `mass_mentions`, `duplicate_spam`, `caps` and `token_leaks`. All six default to enabled.

#### `/config modlog`

| Option | Required | Description |
|---|---|---|
| `channel` | yes | Channel for moderation embeds. |

#### `/config ctf-channel`

| Option | Required | Description |
|---|---|---|
| `channel` | yes | Channel where members may paste flag-shaped text without the flag watcher deleting it. |

This does not control `/ctf create` announcements; use the `channel` option there.

#### `/config verify`

| Option | Required | Description |
|---|---|---|
| `role` | yes | Role granted by the verify panel. |

The role must be below the bot's highest role. Changing it does not retroactively remove or grant roles. `/mod verify-panel` also sets this role when it posts a panel.

#### `/config links`

| Option | Required | Description |
|---|---|---|
| `auto_thread` | no | Create a thread for every link. |
| `ttl_hours` | no | Hours before forwarded messages are deleted (1–720). |

At least one option is required. The first use marks the guild's link settings custom, so these values override the per-category YAML for all monitored categories. Defaults: auto-thread `true`, TTL 24 hours.

Example: `/config links auto_thread:false ttl_hours:12`.

#### Daily CVE digest

When `digest_channel` is set, the scheduler posts a digest of the five most recent **critical** CVEs from NVD at **09:00 UTC**. It checks every 15 minutes and sends at most one digest per UTC day; without a digest channel, nothing is sent. A failed NVD fetch is logged and the day is skipped.

---

## Core

**Permission:** none — every member can use these.

### `/help`

| Option | Required | Description |
|---|---|---|
| `command` | no | Show details for one command (max 32 chars). |

Without an option, publicly lists all 20 commands with one-line descriptions. With a command name, replies ephemerally with its description and a hint to open the Discord option list.

### `/stats`

No options. Public. Shows uptime, connected guilds, configured guilds, commands served, messages seen, automod actions, tracked (TTL) messages, phishing domains loaded, handler errors, database health and version.

### `/ping`

No options. Ephemeral `Pong!` with process uptime. This measures gateway/process liveness, not Discord latency.

### `/about`

No options. Public. Version, guild count and source repository link.

---

## Component flows

| Custom ID | Owner | Behaviour |
|---|---|---|
| `verify:grant` | moderation | Grants the configured verify role; idempotent. |
| `quiz:<question_id>:<choice_index>` | learning | Records the answer and reveals the explanation ephemerally. |
| `vote:<up\|down>:<suggestion_id>` | community | Adds or flips a suggestion vote. |
| `ctf:hint:<challenge_id>` | ctf | Reserved; no CTF components are emitted yet. |
| `start:<panel>` | start | Edits the ephemeral hub in place; `start:close` deletes it. Panel access re-checked per click. |
| `poll:vote:<poll_id>:<index>` | poll | Records or changes the vote and edits the public poll message in place. |
| `poll:close:<poll_id>` | poll | Author/staff closes the poll; removes the buttons. |
| `ticket:open` | ticket | Creates a private ticket thread in the panel channel. |
| `ticket:close` | ticket | Closes and archives the current ticket thread. |

Unknown or stale components answer "This button is no longer available." Modals are not used in v1 and are politely rejected.

---

## Limits

- Interaction acknowledgement happens immediately; slow external calls (intel) edit the response when done.
- Message content is capped at Discord's 2,000 characters and truncated safely; embeds are used for long output.
- Most list commands cap at 25 entries per page; `/mod case list` allows up to 50 and `/mod evidence` up to 25.
- Intel lookups are cached and rate-limited (4-second per-user cooldown); `/tool password` and `/ctf solve` have 3-second cooldowns; `/recon` has a 5-second per-user cooldown.
- Passive recon caches results for 1–6 hours; upstream rate limits are reported as ephemeral errors.
- Polls have no duration: they stay open until the author or staff closes them.
- Ticket threads archive after one day of inactivity (Discord auto-archive) but remain readable; close them with the button or `/ticket close`.
- External API calls time out after 5 seconds and fail with an ephemeral error.
- One bot instance per SQLite file. Scale vertically, not horizontally.
