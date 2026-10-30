# Deploying CipherBot in production

A checklist-driven guide for running CipherBot on a small VPS, a homelab or a container platform. The short version lives in the [README](../README.md); this document covers the details that bite in production.

---

## 1. Prerequisites

- A Discord application and bot token.
- Docker 24+ (or a Rust toolchain if you run the binary directly).
- A persistent volume for the SQLite database.
- Optional: a reverse proxy for `/healthz`, and a Prometheus-compatible scraper for `/metrics`.

One process handles all guilds. There is no sharding in v1 and no horizontal scaling: **one bot instance per database file**.

---

## 2. Discord application setup

1. **Create the app** — [Developer Portal](https://discord.com/developers/applications) → *New Application*.
2. **Bot → token** — reset and copy the token into your secret store. Treat it as a password: anyone with it controls the bot.
3. **Privileged intents**
   - *Message Content Intent*: **required** (commands and automod read message content).
   - *Server Members Intent*: **only** if `ENABLE_MEMBER_INTENT=true` (anti-raid join gate). Enabling the intent without the env flag is harmless but unnecessary.
4. **Invite** — OAuth2 URL Generator, scopes `bot` + `applications.commands`, permissions `1497064631318` (see the README table). Missing `applications.commands` means no slash commands, even if the bot is online. The integer includes **Create Private Threads**, required by `/ticket`; without it the panel button fails with `Missing Permissions`.
5. **Role hierarchy** — drag the CipherBot role above the roles it must assign (quarantine, verify). Discord refuses role changes above the bot's highest role.
6. **Ticket staff visibility** — private threads are only visible to roles holding `Manage Threads` (or to members explicitly added). Grant `Manage Threads` to the staff role, otherwise staff cannot see tickets opened by other members. The `ticket_role` ping does not grant access.

Commands are registered as **guild commands** on startup and on `guild_create`. There is a short propagation delay after the first registration; refresh the client if the list looks stale.

---

## 3. Configuration

Configuration comes from environment variables, optionally layered over a YAML file (`CONFIG_PATH`, default `config.yaml`). Environment wins.

Minimum:

```dotenv
DISCORD_TOKEN=...
DATABASE_PATH=/data/cipherbot.db
```

Recommended:

```dotenv
HEALTH_ADDR=0.0.0.0:8080
RUST_LOG=info,cipherbot=debug
MONITORED_CATEGORIES=123456789012345678,234567890123456789
```

Optional intel keys: `NVD_API_KEY`, `VT_API_KEY`, `IPINFO_TOKEN`. Without them, the corresponding lookups either degrade (NVD public rate limit) or explain that a key is required (VirusTotal).

The polls, tags, tickets, `/start` and `/recon` features add **no environment variables**. `/recon`, `/intel dns` and `/tool` need no API key; they only need outbound HTTPS.

Secret handling:

- Keep `.env` at mode `0600`, owned by the deploying user.
- Never bake tokens into the image or commit `config.yaml`.
- On orchestrators, inject env vars from the platform's secret store; the binary only reads the environment.
- Rotating the token = update the secret and restart. No state is tied to the token.

---

## 4. Storage

SQLite is the only state. WAL mode creates three files:

```
cipherbot.db
cipherbot.db-wal
cipherbot.db-shm
```

The container runs as UID/GID `10001`. Before the first start:

```bash
mkdir -p /srv/cipherbot/data
chown 10001:10001 /srv/cipherbot/data
chmod 700 /srv/cipherbot/data
```

If the directory is not writable, the process exits at startup with `unable to open database file`.

Do **not** put the database on NFS/SMB. WAL locking over network filesystems is unreliable and can corrupt the file.

---

## 5. Container deployment

### Compose (recommended)

```bash
cp .env.example .env && $EDITOR .env
mkdir -p data && sudo chown 10001:10001 data
docker compose up -d --build
docker compose ps
curl -fsS http://127.0.0.1:8080/healthz
```

The shipped compose file:

- builds locally and tags `ghcr.io/cleeryy/cipherbot:latest`;
- runs read-only with `cap_drop: ALL` and `no-new-privileges`;
- mounts `./data` at `/data`, tmpfs at `/tmp`;
- limits to 512 MB RAM and 1.0 CPU;
- uses `stop_signal: SIGINT` so shard draining runs on `docker stop`;
- healthchecks `/healthz` every 30 s after a 15 s start period.

### Plain Docker

```bash
docker run -d --name cipherbot \
  --env-file .env \
  -e DATABASE_PATH=/data/cipherbot.db \
  -p 127.0.0.1:8080:8080 \
  -v /srv/cipherbot/data:/data \
  --read-only --tmpfs /tmp:size=16m \
  --security-opt no-new-privileges:true --cap-drop ALL \
  --memory 512m --cpus 1.0 \
  --restart unless-stopped \
  ghcr.io/cleeryy/cipherbot:latest
```

### Platforms (Dockploy, Kubernetes, Nomad)

- Mount a persistent volume at `/data`; set `DATABASE_PATH=/data/cipherbot.db`.
- Keep **one replica**. A second replica on the same volume will hit `database is locked` and can corrupt state.
- Give the process 10–20 s to start; the health endpoint comes up before the gateway handshake completes, so also watch logs for `Connected as`.
- Do not rely on `/healthz` alone for readiness: it reports process/database health, not gateway session state.

---

## 6. Network exposure

`/healthz` and `/metrics` have **no authentication**.

- Bind the port to loopback (`-p 127.0.0.1:8080:8080`) and scrape it from the same host or the container network.
- Never publish `/metrics` on the public internet — it reveals guild counts, command volume and uptime.
- If a reverse proxy fronts the bot, expose only `/healthz` and restrict `/metrics` to your monitoring network:

```caddyfile
bot.example.com {
    @health path /healthz
    handle @health {
        reverse_proxy 127.0.0.1:8080
    }
    respond 404
}
```

```nginx
location = /healthz {
    proxy_pass http://127.0.0.1:8080;
}
location / { return 404; }
```

The bot itself needs **no inbound traffic** from Discord (gateway is outbound); the HTTP server exists only for probes.

Outbound HTTPS (443) is required to `discord.com`/`gateway.discord.gg`, plus the optional intel and recon hosts: `services.nvd.nist.gov`, `www.virustotal.com`, `ipinfo.io`, `rdap.org`, `cloudflare-dns.com`, `crt.sh`, `internetdb.shodan.io`, `web.archive.org` and `urlscan.io`. `/recon` only queries those fixed passive APIs — it never sends traffic to the domain or IP being looked up, and IP targets must be globally routable.

If you change `HEALTH_ADDR`, remember the Docker healthcheck still probes `127.0.0.1:8080`. Either keep the default port or override the healthcheck.

---

## 7. Resource limits

| Deployment | RAM | CPU |
|---|---|---|
| 1–5 guilds, light traffic | 256 MB | 0.5 |
| 5–50 guilds | 512 MB | 1.0 |
| Large / busy | 512 MB–1 GB | 1–2 |

The process is I/O-bound; CPU spikes come from burst message handling and linking. Memory grows with Serenity's guild/member cache — if you enable `GUILD_MEMBERS` for anti-raid on very large guilds, budget more RAM.

Set `RUST_LOG=info` in production. `debug` is verbose and can produce gigabytes of logs per day on busy servers.

---

## 8. Backups

SQLite in WAL mode requires a consistent snapshot. Preferred method, from the host (the runtime image does not ship the `sqlite3` CLI):

```bash
sqlite3 /srv/cipherbot/data/cipherbot.db \
  ".backup '/srv/backups/cipherbot-$(date +%F-%H%M).db'"
```

Cron example (daily, 04:00, keep 14 days):

```cron
0 4 * * * sqlite3 /srv/cipherbot/data/cipherbot.db ".backup '/srv/backups/cipherbot-$(date +\%F).db'" && find /srv/backups -name 'cipherbot-*.db' -mtime +14 -delete
```

Alternative: `docker compose stop cipherbot`, copy `cipherbot.db` **and** the `-wal`/`-shm` files, then start again. Copying only `cipherbot.db` while the bot runs can lose recent writes.

Test restores quarterly:

1. Open the backup read-only: `sqlite3 backup.db 'PRAGMA integrity_check;'` — expect `ok`.
2. Verify row counts for `guild_config`, `cases`, `ctf_challenges`, `tags`, `polls` and `tickets`.
3. During a maintenance window, start the bot with `DATABASE_PATH` pointing at the restored copy and the real token; confirm `/healthz` and `/stats`.

A restore is only proven when the bot boots against the restored file.

---

## 9. Upgrades and rollback

```bash
# 1. Backup
sqlite3 data/cipherbot.db ".backup 'backup-pre-upgrade.db'"

# 2. New image
docker compose pull          # or: docker compose build --pull

# 3. Restart
docker compose up -d

# 4. Verify
docker compose ps
curl -fsS http://127.0.0.1:8080/healthz
docker compose logs --since 5m cipherbot | grep -Ei 'error|warn|connected'
```

Schema migrations run at startup, gated by `PRAGMA user_version`, and are idempotent. Schema **v2** adds `tags`, `polls`, `poll_votes` and `tickets`; the upgrade is automatic, no manual SQL is needed. They are forward-only: rolling back to an older binary after a migration may not work. Pin image tags (`ghcr.io/cleeryy/cipherbot:1.2.3`) instead of `latest` if you need controlled rollbacks, and keep the pre-upgrade backup until the new version has run for a day.

---

## 10. Log rotation

Docker's default `json-file` driver grows without bound. Add to the compose service (or your platform's equivalent):

```yaml
logging:
  driver: json-file
  options:
    max-size: "10m"
    max-file: "3"
```

Under systemd, logs go to the journal; cap it in `/etc/systemd/journald.conf` (`SystemMaxUse=500M`). Never log at `trace` in production.

---

## 11. Monitoring

Scrape `GET /metrics` (Prometheus text format) every 30–60 s.

Useful series and suggested alerts:

| Metric | Alert idea |
|---|---|
| `cipherbot_db_ok` | `== 0` for 2 minutes → database problem |
| `cipherbot_uptime_seconds` | resets unexpectedly → crash loop |
| `cipherbot_errors_total` | rate increase → handler regressions |
| `cipherbot_guilds` | drop → gateway/cache issue |
| `cipherbot_commands_total` | flat during active hours → registration or gateway issue |
| `cipherbot_messages_total` | flat while members chat → gateway or intent problem |
| `cipherbot_automod_actions_total` | sudden spike → raid or spam wave |
| `cipherbot_links_threaded_total` | flat while link channels are active → permission problem |
| `cipherbot_build_info` | version label mismatches the deployed tag → stale image |

`/healthz` returns `200` with `"status":"degraded"` when the database probe fails; alert on the JSON field, not only on the HTTP code. A failed bind is only a warning — the bot keeps running without HTTP.

---

## 12. Multi-guild notes

- Commands are guild-scoped and registered per guild; there are no global commands to clean up.
- Guild settings (`GuildSettings`) are independent per guild: modlog, quarantine/verify roles, CTF channel, digest channel, automod toggles, raid thresholds, link settings.
- Discord rate limits are per bot, shared across guilds. Intel lookups are cached in SQLite to reduce external quota use; automod and link threads are local operations.
- Leave one guild while the bot restarts and commands for it are gone; they return on the next `guild_create`/startup.
- A single bot token cannot be used by two running instances (gateway sessions conflict). For a second environment (staging), create a second application.

---

## 13. Optional API keys and quotas

| Key | Used by | Free-tier notes |
|---|---|---|
| `NVD_API_KEY` | `/intel cve`, `/intel cve-latest`, `/intel cve-search`, daily digest | Public access is heavily rate-limited (requests per rolling 30 s); a key raises it. |
| `VT_API_KEY` | `/intel hash`, `/intel vt` | Free keys allow ~4 requests/minute and a low daily quota. Cached results help. |
| `IPINFO_TOKEN` | `/intel ip` (enrichment) | Optional; RDAP still answers without it. |
| none | `/recon`, `/intel dns`, `/tool` | Fixed public APIs only; no key, but outbound HTTPS required. |

All external calls use fixed hosts and a 5-second timeout. A missing key never crashes a command; it produces an ephemeral explanation.

---

## 14. Security checklist

- [ ] Token only in env/secret store, `.env` mode `0600`, never committed.
- [ ] `.env`, `config.yaml` and `*.db*` covered by `.gitignore`.
- [ ] Container runs as UID 10001, read-only rootfs, `cap_drop: ALL`, `no-new-privileges`.
- [ ] `/metrics` not reachable from the internet.
- [ ] Data directory `0700`, backups encrypted at rest if they leave the host.
- [ ] Bot role above quarantine/verify roles; no `Administrator` permission granted.
- [ ] Staff role holds `Manage Threads` (otherwise private ticket threads are invisible), and `Create Private Threads` is in the invite integer.
- [ ] Outbound HTTPS allowed to Discord and the passive recon/intel hosts; no inbound bot traffic.
- [ ] `ENABLE_MEMBER_INTENT=true` only when the anti-raid gate is actually used.
- [ ] Log level `info`; no tokens or flags in logs (the `Secret` type redacts them).
- [ ] Restore drill completed against a backup.
- [ ] Upgrade path pinned to image tags with a documented rollback.

---

## 15. Launch checklist

1. [ ] Application created, token stored, intents configured.
2. [ ] Bot invited with `bot` + `applications.commands` and the permission integer.
3. [ ] `.env` filled; data directory owned by `10001`.
4. [ ] `docker compose up -d`; logs show `Connected as` and command registration.
5. [ ] `/help`, `/ping`, `/about`, `/start` answer.
6. [ ] `/config view` shows defaults; set modlog, verify role, CTF channel, ticket role.
7. [ ] `/mod verify-panel` posts a working button (role hierarchy checked).
8. [ ] `/ticket panel` posts a panel; opening a ticket creates a private thread staff can see (Manage Threads checked).
9. [ ] `/recon subdomains domain:<your-domain>` returns data and `/tag add`/`/tag get` round-trips.
10. [ ] `/healthz` returns `ok`; `/metrics` scraped internally.
11. [ ] Backup job scheduled and one restore verified.
12. [ ] Alerting on `cipherbot_db_ok` and process restarts.
