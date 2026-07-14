# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | ✅ |

## Reporting a Vulnerability

If you discover a security vulnerability in CipherBot, please report it privately.

**Do not open a public issue.** Instead, send an email or reach out on Discord.

### What to include

- **Description** of the vulnerability
- **Steps to reproduce** — minimal, complete, and reproducible
- **Impact** — what an attacker could do
- **Suggested fix** (optional but appreciated)

### What to expect

- I'll acknowledge receipt within 48 hours
- I'll assess the severity and impact
- I'll work on a fix and release as soon as practical
- I'll credit you (if desired) once the fix is published

## Scope

- The bot binary (`cipherbot`)
- Configuration handling (token leakage via env/config)
- SQLite database file permissions
- Discord API token security

## Out of Scope

- Discord API itself
- Rust/Cargo toolchain vulnerabilities
- Host/server security (Docker host, Dockploy, etc.)

## Responsible Disclosure

Please give me a reasonable time to fix the issue before disclosing it publicly (at least 30 days for medium+ severity).
