# syntax=docker/dockerfile:1
#
# CipherBot — reproducible multi-stage build.
# SQLite is compiled in (`rusqlite` with the `bundled` feature), so the builder
# needs no libsqlite3 and the runtime image needs no database packages.
#
#   docker build -t cipherbot .
#   docker run --rm -e DISCORD_TOKEN=... -p 8080:8080 -v cipherbot-data:/data cipherbot

ARG RUST_VERSION=1

# ---------------------------------------------------------------------------
# Build stage
# ---------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

WORKDIR /build

ENV CARGO_TERM_COLOR=always \
    CARGO_INCREMENTAL=0

# 1. Compile the dependency graph against a dummy binary so that source edits
#    do not invalidate the (slow) dependency layer.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

# 2. Build the real binary. `touch` guarantees the crate is recompiled even if
#    the copied mtimes are older than the dummy build.
COPY src ./src
RUN touch src/main.rs \
    && cargo build --release --locked

# ---------------------------------------------------------------------------
# Runtime stage
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

ARG VERSION=1.0.0

LABEL org.opencontainers.image.title="CipherBot" \
      org.opencontainers.image.description="Cybersecurity community Discord bot: moderation, CTF, threat intel, tradecraft, learning and link curation." \
      org.opencontainers.image.source="https://github.com/cleeryy/cipherbot" \
      org.opencontainers.image.url="https://github.com/cleeryy/cipherbot" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 cipher \
    && useradd --uid 10001 --gid cipher --home-dir /data --no-create-home --shell /usr/sbin/nologin cipher \
    && install -d -o cipher -g cipher /data

COPY --from=builder /build/target/release/cipherbot /usr/local/bin/cipherbot

ENV DATABASE_PATH=/data/cipherbot.db \
    RUST_LOG=info \
    SQLITE_TMPDIR=/tmp \
    TMPDIR=/tmp

WORKDIR /data
VOLUME ["/data"]

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

USER cipher

ENTRYPOINT ["/usr/local/bin/cipherbot"]
