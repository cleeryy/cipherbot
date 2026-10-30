#!/usr/bin/env bash
# Serialized cargo check (never run bare `cargo build` in parallel).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
exec flock /tmp/cipherbot-cargo.lock -c \
  "cd '$ROOT' && set -o pipefail; CARGO_BUILD_JOBS=2 cargo check --message-format short 2>&1 | tail -60"
