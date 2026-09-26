#!/usr/bin/env bash
# SC-006: no retired flow ships in the frontend and it can only read.
# Exits non-zero (and says why) if any check fails.
set -u
cd "$(dirname "$0")/../frontend" || exit 2
fail=0
say() { echo "FAIL: $*"; fail=1; }

# 1. Only GET requests, with one exception (constitution 3.0.0): the review-decision
#    write in src/consultation_client.js, which may use POST and nothing else.
#    No other file may name a non-GET method; no other transport or write API at all.
if grep -rnIE "method\s*:\s*['\"](POST|PUT|PATCH|DELETE)['\"]" src index.html --exclude=consultation_client.js; then say "non-GET request method outside consultation_client.js"; fi
if [ -f src/consultation_client.js ] && grep -nIE "method\s*:\s*['\"](PUT|PATCH|DELETE)['\"]" src/consultation_client.js; then say "consultation_client.js may only POST"; fi
if grep -rnIE "\bnew WebSocket\b|\bEventSource\b|\bsendBeacon\b|\bXMLHttpRequest\b|\.open\(\s*['\"](POST|PUT|PATCH|DELETE)" src index.html; then say "non-fetch transport / write API found"; fi
# fetch() calls must pass through the two client helpers.
n=$(grep -rnI "fetch(" src | grep -v -e "^src/service_client.js" -e "^src/consultation_client.js" | wc -l | tr -d ' ')
[ "$n" = "0" ] || say "fetch() used outside service_client.js / consultation_client.js"

# 2. No retired files.
for f in draft_editor graph_editor node_inspector paper_review algo registry samples kb_catalog; do
  [ -e "src/$f.js" ] && say "retired file src/$f.js is still shipped"
done
[ -e mcv-workstation.html ] && say "retired page mcv-workstation.html is still shipped"

# 3. No retired concepts in source: dataset / run / preview / designer / paper review flows.
if grep -rnIiE "dataset|/runs|/preview|/designer|/papers|paper[_ -]?review|draft[_ -]?editor|graph[_ -]?editor|contentEditable|<textarea" src index.html; then say "retired flow vocabulary found"; fi

# 4. No storage of catalog data (stale entries must never be shown as current) and no third-party origins.
if grep -rnIE "localStorage|sessionStorage|indexedDB|caches\." src; then say "client-side persistence found"; fi
if grep -rnIE "https?://" index.html src | grep -v "^src/.*://127.0.0.1"; then say "external origin referenced (offline-first)"; fi

[ $fail = 0 ] && echo "frontend read-only check: OK"
exit $fail
