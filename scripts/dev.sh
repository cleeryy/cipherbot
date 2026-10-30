#!/usr/bin/env bash
# Local development entrypoint.
#
# Loads `.env` (shell-compatible KEY=VALUE lines) if present, sets a sane log
# filter, then runs the bot. The binary also calls `dotenvy`, so running
# `cargo run` directly picks up the same file.
#
#   ./scripts/dev.sh              # cargo run (debug)
#   ./scripts/dev.sh --release    # cargo run --release
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# rustup installs live outside the default non-login PATH on many machines.
export PATH="$HOME/.cargo/bin:$PATH"

if [[ -f .env ]]; then
    set -a
    # shellcheck disable=SC1091
    . ./.env
    set +a
fi

export RUST_LOG="${RUST_LOG:-info,cipherbot=debug}"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found — install Rust from https://rustup.rs" >&2
    exit 1
fi

exec cargo run "$@"
