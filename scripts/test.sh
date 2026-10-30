#!/usr/bin/env bash
# Serialized cargo test (shares the cargo lock with ck.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
exec flock /tmp/cipherbot-cargo.lock -c \
  "cd '$ROOT' && set -o pipefail; CARGO_BUILD_JOBS=2 cargo test 2>&1 | tail -80"
