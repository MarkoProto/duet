#!/bin/sh
# Straight into a game, in one command ("@demo"): restarts the demo setup, signs two people up
# (Marko and Nina), pairs them, and opens their two Chrome windows inside the room.
#
#   scripts/demo-play.sh                  any game, 90-second room
#   scripts/demo-play.sh signal           only Signal is offered
#   scripts/demo-play.sh any 15           any game, 15-minute room
#   scripts/demo-play.sh thisorthat 15    This or That, 15 minutes
#   scripts/demo-play.sh stop             stop the demo (rooms built ahead in Daytona are deleted)
#
# Games: wavelength, sketch, signal, thisorthat, story (or any). The lobby keeps running after this
# script ends; its output goes to data/demo.log. Left window: Marko. Middle (Incognito): Nina.
set -eu
cd "$(dirname "$0")/.."
game="${1:-any}"
mins="${2:-1.5}"
base="http://127.0.0.1:8787"

stop() {
  pkill -INT -f 'target/release/duet$' 2>/dev/null || true
  i=0
  while pgrep -f 'target/release/duet$' >/dev/null 2>&1; do
    i=$((i + 1)); [ "$i" -gt 40 ] && break; sleep 0.5
  done
}
stop
[ "$game" = "stop" ] && { echo "demo stopped"; exit 0; }

secs=$(awk "BEGIN { printf \"%d\", $mins * 60 }")
case "$game" in
  any) activities="" ;;
  wavelength|sketch|signal|thisorthat|story) activities="$game" ;;
  *) echo "unknown game: $game (wavelength, sketch, signal, thisorthat, story or any)" >&2; exit 2 ;;
esac
# Who plays what: "talk" people get Same Wavelength or This or That, "make" Four Hands or Tale for
# Two, "play" Signal. With one game offered, it is that one whatever they answer.
case "$game" in
  sketch|story) mode=make ;;
  signal) mode=play ;;
  *) mode=talk ;;
esac

mkdir -p data
DUET_SESSION_SECS="$secs" DUET_ACTIVITIES="$activities" nohup scripts/run.sh demo > data/demo.log 2>&1 &
echo "starting the demo ($game, $secs s rooms)…"
i=0
until grep -q 'session ' data/demo.log 2>/dev/null; do
  i=$((i + 1)); [ "$i" -gt 600 ] && { echo "the lobby did not start; see data/demo.log" >&2; exit 1; }; sleep 0.5
done
# With Daytona, wait until the rooms built ahead are ready, so the room opens at once.
if grep -q 'Daytona sandboxes' data/demo.log; then
  i=0
  until curl -s "$base/api/hood" | grep -q '"state":"ready"'; do
    i=$((i + 1)); [ "$i" -gt 120 ] && break; sleep 0.5
  done
fi
sleep 1

tmp=$(mktemp -d)
join() { # $1 jar, $2 name
  curl -s -c "$1" -b "$1" -H 'Content-Type: application/json' -d "{\"name\":\"$2\",\"lang\":\"en\",\"adult\":true}" "$base/api/join" >/dev/null
  curl -s -c "$1" -b "$1" -H 'Content-Type: application/json' \
    -d "{\"langs\":[\"en\"],\"mood\":\"playful\",\"mode\":\"$mode\",\"style\":\"team\",\"talk\":\"both\",\"interests\":[\"food\",\"music\"],\"smile\":\"\"}" \
    "$base/api/answers" >/dev/null
}
join "$tmp/a" Marko
join "$tmp/b" Nina
url_of() { curl -s -b "$1" "$base/api/state?v=0" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("url") or "")'; }
i=0
until [ -n "$(url_of "$tmp/a")" ] && [ -n "$(url_of "$tmp/b")" ]; do
  i=$((i + 1)); [ "$i" -gt 120 ] && { echo "the two were not paired in time" >&2; exit 1; }; sleep 0.5
done
urlA=$(url_of "$tmp/a"); urlB=$(url_of "$tmp/b")
secA=$(awk '$6 == "duet" { print $7 }' "$tmp/a"); secB=$(awk '$6 == "duet" { print $7 }' "$tmp/b")
kind=$(curl -s -b "$tmp/a" "$base/api/state?v=0" | python3 -c 'import json,sys; print((json.load(sys.stdin).get("activity") or {}).get("kind",""))')
rm -rf "$tmp"

# Each window signs in as its person (the private return link sets the cookie), then enters.
osascript - "$base" "$secA" "$urlA" "$secB" "$urlB" <<'APPLESCRIPT' >/dev/null
on run argv
  set {baseUrl, secA, urlA, secB, urlB} to argv
  tell application "Google Chrome"
    -- Windows are kept by id: their order changes as soon as one comes to the front.
    set idA to missing value
    set idB to missing value
    repeat with w in windows
      set u to URL of active tab of w
      if u starts with baseUrl and u does not contain "/hood" then
        if mode of w is "incognito" then
          if idB is missing value then set idB to id of w
        else
          if idA is missing value then set idA to id of w
        end if
      end if
    end repeat
    if idA is missing value or idB is missing value then return
    set URL of active tab of window id idA to baseUrl & "/me/" & secA
    set URL of active tab of window id idB to baseUrl & "/me/" & secB
    delay 2
    set URL of active tab of window id idA to urlA
    set URL of active tab of window id idB to urlB
  end tell
end run
APPLESCRIPT
echo "in the room: $kind, $secs s. Left window Marko, middle (Incognito) Nina, right the admin page."
echo "room links (for this session only):"
echo "  Marko $urlA"
echo "  Nina  $urlB"
