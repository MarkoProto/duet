#!/bin/sh
# Live security checks against a running Duet: a lobby of its own on this computer, with rooms inside
# it and no keys, so nothing outside is touched. Every line is PASS or FAIL; the exit code is the
# number of failures.
#
#   scripts/security-check.sh
#
# What it tries: the pages' security headers (no framing by other sites, nothing loaded from them),
# the session cookie, oversized, malformed and cross-site requests, admin actions without the key,
# rooms with a wrong or missing key, path tricks on the room address, markup in chat, a chat flood
# and an over-long message.
set -u
cd "$(dirname "$0")/.." || exit 1
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo build --release -p duet --quiet || exit 1
S=$(mktemp -d "${TMPDIR:-/tmp}/duet-security.XXXXXX")
fails=0
P=$((8700 + $$ % 200)); B="http://127.0.0.1:$P"; AK=probe-admin-key-0123456789
env HOME="$S" DUET_BIND=127.0.0.1:$P DUET_PUBLIC_URL="$B" DUET_ROOMS=local DUET_DATA="$S/data" DUET_ADMIN_KEY=$AK \
  DUET_SESSION_SECS=120 DUET_SETTLE_SECS=0.5 DUET_GATHER_SECS=1 DAYTONA_API_KEY= NOSANA_API_KEY= ./target/release/duet > "$S/lobby.log" 2>&1 &
LP=$!
i=0; until curl -sf -o /dev/null "$B/api/state?v=0"; do i=$((i + 1)); [ $i -gt 60 ] && { echo "lobby did not start"; exit 1; }; sleep 0.5; done
ok() { echo "  PASS $1"; }; no() { echo "  FAIL $1"; fails=$((fails + 1)); }
hdr() { curl -s -D - -o /dev/null "$1" | tr -d '\r'; }

echo "== Security headers on the pages"
for path in / /hood; do
  h=$(hdr "$B$path")
  for want in "Content-Security-Policy: .*frame-ancestors" "X-Frame-Options: DENY" "X-Content-Type-Options: nosniff" "Referrer-Policy" "Permissions-Policy"; do
    echo "$h" | grep -qi "^$want" && ok "$path sends $want" || no "$path sends $want"
  done
done

echo "== Cookie"
J=$S/jar; rm -f $J
c=$(curl -s -D - -o /dev/null -c $J -H 'Content-Type: application/json' -d '{"name":"<img src=x onerror=alert(1)>","lang":"en","adult":true}' "$B/api/join" | tr -d '\r' | grep -i '^set-cookie')
echo "$c" | grep -qi "HttpOnly" && ok "session cookie is HttpOnly (scripts cannot read it)" || no "cookie HttpOnly"
echo "$c" | grep -qi "SameSite=Lax\|SameSite=Strict" && ok "session cookie is SameSite (no cross-site use)" || no "cookie SameSite"

echo "== Untrusted input"
name=$(curl -s -b $J "$B/api/state?v=0" | python3 -c 'import json,sys; d=json.load(sys.stdin); print((d.get("me") or {}).get("name") or d.get("name") or "")')
echo "  stored name: $name"
case "$name" in *"<"*) echo "  (kept as text; safe only if every page sets it with textContent)";; esac
big=$(python3 -c 'print("A"*20000)')
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"name\":\"$big\",\"lang\":\"en\",\"adult\":true}" "$B/api/join")
[ "$code" = 413 ] || [ "$code" = 400 ] || [ "$code" = 422 ] && ok "a 20 KB body is refused ($code)" || no "a 20 KB body was accepted ($code)"
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Content-Type: text/plain' -d '{"name":"x","lang":"en","adult":true}' "$B/api/join")
[ "$code" = 415 ] && ok "a cross-site form post (not JSON) is refused ($code), so no CSRF through forms" || no "non-JSON post accepted ($code)"
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"name":"x","lang":"en"' "$B/api/join")
[ "$code" -ge 400 ] && [ "$code" -lt 500 ] && ok "broken JSON is refused ($code)" || no "broken JSON gave $code"
acao=$(curl -s -D - -o /dev/null -H 'Origin: https://evil.example' "$B/api/state?v=0" | tr -d '\r' | grep -i '^access-control-allow-origin')
[ -z "$acao" ] && ok "no CORS: other sites cannot read the API's answers" || no "CORS header sent: $acao"

echo "== Admin"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'Content-Type: application/json' -d '{"n":3}' "$B/api/hood/warm")
[ "$code" = 403 ] || [ "$code" = 401 ] && ok "rebuilding rooms without the admin key is refused ($code)" || no "admin action without key gave $code"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "Authorization: Bearer wrong-key" -H 'Content-Type: application/json' -d '{"n":3}' "$B/api/hood/warm")
[ "$code" = 403 ] || [ "$code" = 401 ] && ok "a wrong admin key is refused ($code)" || no "wrong admin key gave $code"
hood=$(curl -s "$B/api/hood")
echo "$hood" | grep -q "onerror" && no "the public admin numbers include a person's name" || ok "the public numbers show no names"

echo "== Rooms"
K=$S/k; rm -f ${K}a ${K}b
for w in a b; do
  curl -s -c ${K}$w -b ${K}$w -H 'Content-Type: application/json' -d "{\"name\":\"P$w\",\"lang\":\"en\",\"adult\":true}" "$B/api/join" >/dev/null
  curl -s -c ${K}$w -b ${K}$w -H 'Content-Type: application/json' -d '{"langs":["en"],"mood":"calm","mode":"talk","style":"team","talk":"both","interests":["music"],"smile":""}' "$B/api/answers" >/dev/null
done
i=0; url=""; until [ -n "$url" ]; do url=$(curl -s -b ${K}a "$B/api/state?v=0" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("url") or "")'); i=$((i + 1)); [ $i -gt 40 ] && break; sleep 0.5; done
room=$(echo "$url" | sed -E 's|#.*||'); key=$(echo "$url" | sed -E 's|.*#k=([^&]*).*|\1|')
echo "  room: $room"
rh=$(hdr "$room")
for want in "Content-Security-Policy: .*frame-ancestors 'none'" "X-Frame-Options: DENY" "Permissions-Policy" "X-Content-Type-Options: nosniff"; do
  echo "$rh" | grep -qi "^$want" && ok "the room page sends $want" || no "the room page sends $want"
done
code=$(curl -s -o /dev/null -w '%{http_code}' "${room}hello?k=not-the-key&after=0")
[ "$code" = 403 ] && ok "a wrong room key is refused ($code)" || no "wrong room key gave $code"
code=$(curl -s -o /dev/null -w '%{http_code}' "${room}hello?after=0")
[ "$code" = 403 ] || [ "$code" = 400 ] && ok "no room key is refused ($code)" || no "no room key gave $code"
code=$(curl -s -o /dev/null -w '%{http_code}' "${room}hello?k=$key&after=0")
[ "$code" = 200 ] && ok "the right key opens the room ($code)" || no "right key gave $code"
code=$(curl -s -o /dev/null -w '%{http_code}' "$B/r/r_does_not_exist/hello?k=$key")
[ "$code" = 404 ] && ok "an unknown room id is not found ($code)" || no "unknown room gave $code"
for t in '../api/hood' '..%2F..%2Fapi%2Fhood' '%2e%2e/%2e%2e/etc/passwd'; do
  code=$(curl -s --path-as-is -o /dev/null -w '%{http_code}' "${room}$t")
  echo "  path trick ${t}: $code"
done
# Chat with markup, a flood, and an oversized message.
post() { curl -s -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "$1" "${room}send?k=$key"; }
post '[{"t":"chat","mid":"x1","text":"<script>alert(1)</script><img src=x onerror=alert(2)>"}]' >/dev/null
sleep 0.5
log=$(curl -s "${room}poll?k=$key&after=0&la=0&hold=0")
echo "$log" | grep -q '<script>' && echo "  chat keeps markup as plain text (safe only with textContent on the page)" || ok "chat markup is cleaned before it is stored"
n=0; for i in $(seq 1 30); do post "[{\"t\":\"chat\",\"mid\":\"f$i\",\"text\":\"flood $i\"}]" >/dev/null; done
sleep 0.5
kept=$(curl -s "${room}poll?k=$key&after=0&la=0&hold=0" | grep -o 'flood [0-9]*' | wc -l | tr -d ' ')
[ "$kept" -lt 30 ] && ok "a chat flood is rate-limited ($kept of 30 kept)" || no "chat flood not limited ($kept of 30)"
long=$(python3 -c 'print("x"*5000)')
post "[{\"t\":\"chat\",\"mid\":\"big\",\"text\":\"$long\"}]" >/dev/null; sleep 0.3
curl -s "${room}poll?k=$key&after=0&la=0&hold=0" | grep -q "$(python3 -c 'print("x"*700)')" && no "a 5000-character chat message was kept whole" || ok "an over-long chat message is refused or cut"

kill -INT $LP; sleep 1; rm -rf "$S"
echo
[ "$fails" -eq 0 ] && echo "All checks passed." || echo "$fails check(s) failed."
exit "$fails"
