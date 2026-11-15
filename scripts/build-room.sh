#!/bin/sh
# Builds the room program for the two kinds of processor a Daytona sandbox may have, as static
# Linux binaries, into dist/. The lobby uploads whichever one the sandbox reports.
set -eu
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
mkdir -p dist
for arch in x86_64 aarch64; do
  target="$arch-unknown-linux-musl"
  rustup target add "$target" >/dev/null 2>&1 || true
  cargo build -p duet-room --profile room --target "$target" --quiet
  cp "target/$target/room/duet-room" "dist/duet-room-$arch"
  printf '%s  %s bytes  %s\n' "dist/duet-room-$arch" "$(wc -c < "dist/duet-room-$arch" | tr -d ' ')" "$(file -b "dist/duet-room-$arch" | cut -d, -f1-3)"
done
