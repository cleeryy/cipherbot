# Contributing to CipherBot

First off, thanks for taking the time to contribute! 🎉

## Code of Conduct

This project adheres to a [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold it.

## How Can I Contribute?

### 🐛 Reporting Bugs

Before submitting a bug report, check the [issues](https://github.com/cleeryy/cipherbot/issues) to see if it's already been reported.

When creating a bug report, include as much detail as possible:

- **Steps to reproduce** — what did you do?
- **Expected behavior** — what should have happened?
- **Actual behavior** — what actually happened?
- **Environment** — Rust version, OS, Docker or native, Discord library version

### 💡 Suggesting Features

Open an [issue](https://github.com/cleeryy/cipherbot/issues/new) describing:

1. What you want to achieve
2. Why existing functionality isn't sufficient
3. How you envision it working (rough API sketch is great)

### 🔧 Pull Requests

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/my-awesome-feature`
3. Make your changes
4. Run `cargo build` and fix any compilation errors
5. Run `cargo clippy` (if available) to ensure code quality
6. Commit with a descriptive message (see [commit style](#commit-style))
7. Push and open a PR against the `master` branch

#### Development Setup

```bash
git clone https://github.com/cleeryy/cipherbot
cd cipherbot
cp config.example.yaml config.yaml
cp .env.example .env
# Fill in .env with your Discord bot token and category IDs
cargo build
cargo run
```

### Commit Style

I use conventional commits:

```
feat: add support for multiple monitored categories
fix: crash when message content is empty
docs: update quick start example
refactor: extract link detection into helper
chore: bump serenity to 0.12.5
```

### Code Style

- **No `as any` or `#[allow(...)]`** unless absolutely necessary
- **Meaningful names** — `track_message` not `tm`
- **No AI-generated comments** — code should be self-documenting
- **Match existing patterns** — look at the codebase before writing new code
- **Error handling** — use `anyhow::Result`, don't swallow errors

### Project Structure

```
src/
├── main.rs      # Entry point — keep it thin
├── config.rs    # Configuration loading
├── db.rs        # Database abstractions
└── handler.rs   # Discord events
```

Keep business logic in handlers, data access in db.rs, and configuration in config.rs. If a new module is needed, create it in `src/` and register it in `main.rs`.

## Review Process

PRs are reviewed within a few days. I look for:

- Correctness — does it work?
- Safety — no unwrap() on user-controlled data, no silent failures
- Consistency — matches existing style and patterns
- Minimalism — does the simplest thing that works

---

*Questions? Open a discussion or an issue — I'm happy to help.*
