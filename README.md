<div align="center">
  <img src="https://img.shields.io/badge/rust-1.96+-de8b3e?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/serenity-0.12-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Serenity">
  <img src="https://img.shields.io/badge/license-MIT-blue?style=for-the-badge" alt="MIT License">
  <br>
  <img src="https://img.shields.io/github/stars/cleeryy/cipherbot?style=flat-square" alt="Stars">
  <img src="https://img.shields.io/github/last-commit/cleeryy/cipherbot?style=flat-square" alt="Last commit">
  <img src="https://img.shields.io/badge/docker-ready-2496ED?style=flat-square&logo=docker&logoColor=white" alt="Docker">
</div>

<h1 align="center">🔐 CipherBot</h1>

<p align="center">
  <strong>A Discord bot for organized link sharing — auto-thread links, auto-clean non-link messages.</strong>
  <br>
  <sub>Built with Rust, Serenity, SQLite. Deploy anywhere with Docker.</sub>
</p>

---

## ✨ Features

- **🔗 Auto-thread links** — Post a link in a monitored channel and CipherBot instantly creates a thread for discussion
- **🧹 Auto-clean non-link messages** — Messages without links are automatically deleted after a configurable TTL (default 24h)
- **🏷️ Per-category config** — Monitor multiple Discord categories with different settings
- **📦 Persistent SQLite storage** — Survives restarts, no external database needed
- **🐳 Docker-ready** — Multi-stage build, ~8MB final image, works with Dockploy out of the box
- **⚡ Blazing fast** — Built with Rust, minimal resource usage
- **🔧 YAML + env var configuration** — Flexible, 12-factor app style

---

## 🚀 Quick Start

### Prerequisites

- Rust 1.96+ (or Docker)
- A [Discord bot application](https://discord.com/developers/applications) with:
  - `MESSAGE CONTENT INTENT` enabled
  - Bot invited to your server with `Create Threads`, `Manage Messages`, `Read Message History`, `Send Messages` permissions

### Configuration

CipherBot uses a **YAML config file** with **environment variable overrides** (env vars take precedence).

```env
# .env
DISCORD_TOKEN=your_bot_token_here
MONITORED_CATEGORIES=123456789,987654321
DATABASE_PATH=cipherbot.db
CONFIG_PATH=config.yaml
RUST_LOG=info
```

```yaml
# config.yaml
discord_token: ""

database:
  path: "cipherbot.db"

categories:
  - id: 123456789
    auto_thread_links: true
    message_ttl_hours: 24
  - id: 987654321
    auto_thread_links: true
    message_ttl_hours: 48
```

All config values can be overridden by setting the corresponding environment variable. See [Configuration Reference](#configuration-reference) below.

### From Source

```bash
git clone https://github.com/cleeryy/cipherbot
cd cipherbot

# Copy and edit config
cp config.example.yaml config.yaml
cp .env.example .env
# Edit .env with your DISCORD_TOKEN and MONITORED_CATEGORIES

cargo run --release
```

### Docker / Dockploy

```bash
docker build -t cipherbot .
docker run -d \
  --name cipherbot \
  -e DISCORD_TOKEN=your_token_here \
  -e MONITORED_CATEGORIES=123456789 \
  -v cipherbot-data:/data \
  cipherbot
```

For **Dockploy**, this image works out of the box. Set the env vars in your Dockploy dashboard and mount a persistent volume at `/data`.

---

## 🧠 Architecture

```
                    ┌──────────────────┐
                    │    Discord API    │
                    └────────┬─────────┘
                             │ (Gateway)
                             ▼
┌─────────────────────────────────────────────┐
│              Serenity Client                │
│  ┌─────────────────┐  ┌─────────────────┐  │
│  │  Event Handler   │  │   HTTP Client    │  │
│  └────────┬────────┘  └────────┬────────┘  │
└───────────┼────────────────────┼───────────┘
            │                    │
            ▼                    ▼
┌──────────────────┐    ┌──────────────────┐
│   SQLite DB       │    │  Cleanup Task    │
│  (tracked msgs)   │◄───│  (60s interval)  │
└──────────────────┘    └──────────────────┘
```

### Data Flow

1. **Message received** → Handler checks if channel belongs to a monitored category
2. **Link detected** → Creates a thread from the message for organized discussion
3. **No link** → Message ID + channel ID + expiration timestamp stored in SQLite
4. **Background cleanup** → Every 60 seconds, expired messages are deleted via the Discord API

---

## 📁 Project Structure

```
cipherbot/
├── src/
│   ├── main.rs      # Entry point, client setup, cleanup task
│   ├── config.rs    # YAML + env var configuration
│   ├── db.rs        # SQLite database operations
│   └── handler.rs   # Discord event handler (message logic)
├── Dockerfile        # Multi-stage Docker build
├── Cargo.toml        # Rust dependencies
├── config.example.yaml
├── .env.example
└── ...
```

---

## ⚙️ Configuration Reference

### Environment Variables

| Variable | Overrides | Default | Description |
|---|---|---|---|
| `DISCORD_TOKEN` | `discord_token` | — | Bot token from Discord Developer Portal |
| `MONITORED_CATEGORIES` | `categories` | — | Comma-separated category IDs (e.g. `"111,222"`) |
| `DATABASE_PATH` | `database.path` | `cipherbot.db` | Path to SQLite database file |
| `CONFIG_PATH` | — | `config.yaml` | Path to YAML config file |
| `RUST_LOG` | — | `info` | Log level (debug, info, warn, error) |

### YAML Config

```yaml
discord_token: ""                    # Bot token (env override: DISCORD_TOKEN)

database:
  path: "cipherbot.db"               # SQLite file path (env override: DATABASE_PATH)

categories:                          # List of monitored Discord categories
  - id: 123456789                    # Category ID (right-click category → Copy ID)
    auto_thread_links: true          # Auto-create threads for link messages
    message_ttl_hours: 24            # Delete non-link messages after N hours
```

> **How to get a category ID:** Enable Developer Mode in Discord (User Settings → Advanced → Developer Mode), right-click the category → Copy ID.

---

## 🤝 Contributing

Contributions are welcome! Check out the [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

## 📄 License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

---

<div align="center">
  <sub>Built with ❤️ for organized Discord communities</sub>
</div>
