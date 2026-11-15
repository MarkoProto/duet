#!/bin/sh
# Everything that can be tested without a key: the Rust tests, then two Chrome windows through
# the whole flow and through the harder cases. About two minutes.
set -eu
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo test --workspace --quiet
cargo build -p duet -p duet-room --quiet
cd tests/e2e
[ -d node_modules ] || npm install --silent --no-fund --no-audit
node flow.mjs "${1:-/tmp/duet-shots}"
node edges.mjs "${1:-/tmp/duet-shots}"
node games.mjs "${1:-/tmp/duet-shots}"
