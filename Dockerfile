# syntax=docker/dockerfile:1

# ---- builder ---------------------------------------------------------------
# All deps are crates.io (no git rev) so no git/network beyond the registry.
# rusqlite=bundled (compiles SQLite -> needs a C compiler, which rust:bookworm
# already ships). reqwest/teloxide use rustls -> no openssl.
FROM rust:1-bookworm AS builder

WORKDIR /app

# Only the build inputs (.dockerignore keeps data/, target/, tests/ etc.
# out). `cargo build --release --bin tnt-delivery-bot` builds the lib + bin
# only, not the integration tests under tests/, so their sources aren't needed.
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# BuildKit cache mounts persist the cargo registry + target dir across builds
# so deps are not recompiled every time. Copy the binary out before the cache
# layer ends (cache mounts are not part of the image).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin tnt-delivery-bot \
 && cp /app/target/release/tnt-delivery-bot /usr/local/bin/tnt-delivery-bot

# ---- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# gosu: the entrypoint must start as root to chown the root-owned bind
# mount, then drop to the non-root user before exec'ing the bot.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates gosu \
 && rm -rf /var/lib/apt/lists/*

# Non-privileged runtime user (fixed uid/gid for predictable bind-mount
# ownership on the host).
RUN groupadd --gid 10001 app \
 && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin app

WORKDIR /app

COPY --from=builder /usr/local/bin/tnt-delivery-bot /usr/local/bin/tnt-delivery-bot
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# No USER directive: the entrypoint intentionally runs as root only long
# enough to mkdir -p + chown the bind-mounted DB dir (compose sets DB_PATH
# /app/data/...; rusqlite Connection::open creates the file but NOT the
# parent dir), then `gosu app` drops privileges and exec's the bot.
ENTRYPOINT ["docker-entrypoint.sh"]
