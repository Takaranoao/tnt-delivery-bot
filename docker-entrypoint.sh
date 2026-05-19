#!/bin/sh
set -eu

# Runs as root (no USER in the Dockerfile) only for this prep: rusqlite
# Connection::open (src/db.rs) creates the SQLite file but not its parent
# dir, and the bind-mounted ./data is created root-owned by Docker. mkdir
# -p + chown hands it to the non-root `app` user. No FIFO/console: this bot
# uses a BotFather token (no interactive login) and handles SIGTERM itself.
DB="${DB_PATH:-/app/data/tnt-delivery-bot.sqlite}"
DB_DIR="$(dirname "$DB")"
mkdir -p "$DB_DIR"
chown -R app:app "$DB_DIR"

# Drop privileges to `app` and run the bot as PID 1 (gosu exec's the
# target, replacing itself) so `docker stop` (SIGTERM) reaches it;
# src/main.rs wait_for_signal() handles SIGTERM -> graceful drain.
exec gosu app tnt-delivery-bot "$@"
