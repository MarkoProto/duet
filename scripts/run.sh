#!/bin/sh
# Builds what is needed and runs the lobby. Settings come from the environment and from .env.
#   scripts/run.sh            the lobby
#   scripts/run.sh doctor     test the Daytona and Nosana connections, step by step
#   scripts/run.sh check      a second opinion: the same accounts through Daytona's own Python package
#   scripts/run.sh demo       the stage setup: demo mode and three Chrome windows (scripts/demo.sh)
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" = "demo" ]; then shift; exec ./scripts/demo.sh "$@"; fi
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if [ -f .env ]; then set -a; . ./.env; set +a; fi
if [ "${1:-}" = "check" ]; then
  exec uv run -q --python 3.12 --with 'daytona[v0-sdk]==2.0.0' python -I scripts/sponsor-check.py
fi
# The Linux room binaries go into each Daytona sandbox. Always rebuilt (seconds when nothing
# changed): a stale one would send old pages into every room.
./scripts/build-room.sh >/dev/null
cargo build --release -p duet --quiet
exec ./target/release/duet "$@"
