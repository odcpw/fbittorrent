#!/usr/bin/env bash
# Bar-widget keyboard test: opens the popup through the shell IPC, injects keys with
# wtype, and reads the cursor state back through the `cursor` IPC hook.
# Needs: a running Omarchy shell with oliver.torrents enabled, wtype, a startable backend.
set -u
F="${1:-fbittorrent}"
IPC="omarchy-shell oliver.torrents"
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1: $2"; }
cur() { $IPC cursor 2>/dev/null; }
field() { python3 -c 'import sys,json; d=json.loads(sys.stdin.read()); print(d[sys.argv[1]])' "$1"; }
cleanup() { $IPC close >/dev/null 2>&1; $F delete fbt-wk-one fbt-wk-two --files --yes --json >/dev/null 2>&1; }
trap cleanup EXIT
command -v wtype >/dev/null || { echo "wtype not installed"; exit 1; }

$F up --json >/dev/null 2>&1 || { echo "backend unavailable"; exit 1; }
$F add "magnet:?xt=urn:btih:c9bc96d23542e5430f6bf8232b402a85172ca28b&dn=fbt-wk-one" --stopped --quiet --json >/dev/null
$F add "magnet:?xt=urn:btih:2a501e132638eb2217615588a30a2bc0c924603c&dn=fbt-wk-two" --stopped --quiet --json >/dev/null
# the widget polls every 10 s while closed; opening triggers an immediate refresh
$IPC open >/dev/null; sleep 2.5
for i in 1 2 3 4 5 6; do [ "$(cur | field rows)" -ge 2 ] && break; sleep 1; done

echo "W-001 popup opens and knows the torrents"
if [ "$(cur | field opened)" = "True" ] && [ "$(cur | field rows)" -ge 2 ]; then ok W-001; else bad W-001 "$(cur)"; fi

echo "W-002 j moves the cursor down a row (first j only reveals the cursor)"
wtype j; sleep 0.2; wtype j; sleep 0.3
if [ "$(cur | field section)" = "rows" ] && [ "$(cur | field row)" = "1" ]; then ok W-002; else bad W-002 "$(cur)"; fi

echo "W-003 k moves back up, another k jumps to the header actions"
wtype k; sleep 0.2
r=$(cur | field row); wtype k; sleep 0.3
if [ "$r" = "0" ] && [ "$(cur | field section)" = "header" ]; then ok W-003; else bad W-003 "row=$r $(cur)"; fi

echo "W-004 l moves along the header actions"
wtype l; sleep 0.3
if [ "$(cur | field header)" = "1" ]; then ok W-004; else bad W-004 "$(cur)"; fi

echo "W-005 Escape closes the popup"
wtype -k Escape
closed=0; for i in 1 2 3 4 5 6 7 8 9 10; do sleep 0.2; [ "$(cur | field opened)" = "False" ] && { closed=1; break; }; done
if [ "$closed" = 1 ]; then ok W-005; else bad W-005 "$(cur)"; fi

echo; echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
