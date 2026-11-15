#!/bin/sh
# The stage setup in one command ("@demostart"): Duet in demo mode, then three Chrome windows side by side.
#
#   left    Person A, a normal window at phone width
#   middle  Person B, an Incognito window at phone width (Duet knows people by a cookie, so a second
#           person needs a separate cookie jar)
#   right   the admin page, /hood with the admin key, where "Build them now" is
#
#   scripts/run.sh demo                       rooms last 90 s, a fresh record every run
#   scripts/run.sh demo drawing               whatever people answer, the game is Four Hands
#   DUET_SESSION_SECS=600 scripts/run.sh demo  a longer room
#   scripts/run.sh demo --no-windows          only the lobby, no windows
#
# Games to force: drawing (Four Hands), story (Tale for Two), wavelength (Same Wavelength),
# thisorthat (This or That), signal (Signal), or any (the default: the planner picks).
#
# Ctrl-C stops the lobby the usual way: rooms built ahead in Daytona are deleted on the way out.
# Running it again stops the demo already running first (the same way, so its rooms are deleted too)
# and closes its windows.
set -eu
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Demo values given on the command line win over .env; otherwise the demo's own.
secs="${DUET_SESSION_SECS:-90}"
if [ -f .env ]; then set -a; . ./.env; set +a; fi
export DUET_SESSION_SECS="$secs"
export DUET_ADMIN_KEY="${DUET_ADMIN_KEY:-$(openssl rand -hex 8)}"
data="${DUET_DEMO_DATA:-data/demo}"

windows=1
for arg in "$@"; do
  case "$arg" in
    --no-windows) windows=0 ;;
    any) unset DUET_ACTIVITIES ;;
    drawing|draw|sketch|fourhands) export DUET_ACTIVITIES=sketch ;;
    story|tale) export DUET_ACTIVITIES=story ;;
    wavelength|thisorthat|signal) export DUET_ACTIVITIES="$arg" ;;
    *) echo "demo: unknown option $arg (a game: drawing, story, wavelength, thisorthat, signal, any; or --no-windows)" >&2; exit 2 ;;
  esac
done

# Always rebuilt (seconds when nothing changed): a stale room program would send old pages into every sandbox.
./scripts/build-room.sh >/dev/null
cargo build --release -p duet --quiet

bind="${DUET_BIND:-127.0.0.1:8787}"
port="${bind##*:}"

# The demo already running on this port stops first, with Ctrl-C's signal, so it deletes its rooms
# built ahead on the way out. Only a Duet lobby is stopped: anything else on the port is left alone.
old=$(lsof -nP -t -iTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)
for pid in $old; do
  case "$(ps -o command= -p "$pid" 2>/dev/null)" in
    *release/duet*) ;;
    *) echo "demo: port $port is taken by another program ($(ps -o comm= -p "$pid")); stop it first" >&2; exit 1 ;;
  esac
  echo "demo: stopping the demo already running (its rooms built ahead are deleted first)"
  kill -INT "$pid"
done
for pid in $old; do
  i=0
  while kill -0 "$pid" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 60 ]; then echo "demo: the old demo did not stop in 30 s; stop it, then run this again" >&2; exit 1; fi
    sleep 0.5
  done
done

# A fresh record each run, kept apart from the normal one, so a rehearsal never meets the last one.
# Only now: the old demo writes its record once more as it stops.
rm -rf "$data"
export DUET_DATA="$data"
# The windows open the lobby's own public address, the one it puts in every link it hands out
# (the room's way back included). Another spelling of the same place, such as localhost for
# 127.0.0.1, is another site to the browser: the person's cookie would stay behind on the way back.
base="${DUET_PUBLIC_URL:-http://127.0.0.1:$port}"
base="${base%/}"

if [ "$windows" = 1 ]; then
  (
    # Wait for the lobby (it looks for leftover sandboxes first, which can take a while).
    i=0
    until curl -sf -o /dev/null "http://127.0.0.1:$port/api/state?v=0"; do
      i=$((i + 1))
      if [ "$i" -gt 240 ]; then echo "demo: the lobby did not answer in 2 minutes, no windows opened" >&2; exit 0; fi
      sleep 0.5
    done
    osascript - "$base" "$base/hood#key=$DUET_ADMIN_KEY" "http://localhost:$port" "http://127.0.0.1:$port" <<'APPLESCRIPT' >/dev/null || echo "demo: could not open the Chrome windows (macOS may be asking to allow control of Chrome)" >&2
on run argv
  set baseUrl to item 1 of argv
  set adminUrl to item 2 of argv
  set localA to item 3 of argv
  set localB to item 4 of argv
  tell application "Finder" to set {dLeft, dTop, dRight, dBottom} to bounds of window of desktop
  set winTop to dTop + 38
  set phoneW to 500 -- Chrome will not make a window narrower than this
  tell application "Google Chrome"
    -- The previous run's demo windows: every tab on this lobby, under any of its addresses.
    repeat with i from (count of windows) to 1 by -1
      set w to window i
      set mine to true
      repeat with t in (tabs of w)
        set u to URL of t
        if u does not start with baseUrl and u does not start with localA and u does not start with localB then set mine to false
      end repeat
      if mine then close w
    end repeat
    set a to make new window
    set URL of active tab of a to baseUrl
    set bounds of a to {dLeft, winTop, dLeft + phoneW, dBottom}
    set b to make new window with properties {mode:"incognito"}
    set URL of active tab of b to baseUrl
    set bounds of b to {dLeft + phoneW + 8, winTop, dLeft + 2 * phoneW + 8, dBottom}
    set h to make new window
    set URL of active tab of h to adminUrl
    set bounds of h to {dLeft + 2 * phoneW + 16, winTop, dRight, dBottom}
    activate
  end tell
end run
APPLESCRIPT
  ) &
fi

exec ./target/release/duet
