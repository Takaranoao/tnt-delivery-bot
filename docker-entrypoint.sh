#!/bin/sh
set -eu

# rusqlite Connection::open (src/db.rs) creates the SQLite file but not its
# parent directory. The data volume guarantees /app/data exists; this also
# covers a DB_PATH pointing at a deeper subdir. No FIFO/console: this bot
# uses a BotFather token (no interactive login) and handles SIGTERM itself.
DB="${DB_PATH:-/app/data/tnt-delivery-bot.sqlite}"
mkdir -p "$(dirname "$DB")"

# Run the bot as PID 1 so `docker stop` (SIGTERM) reaches it; src/main.rs
# wait_for_signal() handles SIGTERM -> graceful drain.
exec tnt-delivery-bot "$@"
