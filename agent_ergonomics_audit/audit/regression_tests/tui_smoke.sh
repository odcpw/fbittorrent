#!/usr/bin/env bash
# Headless TUI smoke test: drives `fbittorrent tui` inside tmux and checks what it paints.
# Needs: tmux, a startable qbittorrent-nox. Exit 0 when all checks pass.
set -u
F="${1:-fbittorrent}"
S="fbt-smoke-$$"
ERR=$(mktemp)
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1: $2"; }
cap() { tmux capture-pane -t "$S" -p; }
wait_for() { # wait_for <regex> <seconds>
  local i; for i in $(seq 1 $(( $2 * 10 ))); do cap | grep -q -E "$1" && return 0; sleep 0.1; done; return 1
}
cleanup() { tmux send-keys -t "$S" q 2>/dev/null; sleep 0.3; tmux kill-session -t "$S" 2>/dev/null; $F delete fbt-smoke --files --yes --json >/dev/null 2>&1; rm -f "$ERR"; }
trap cleanup EXIT

$F up --json >/dev/null 2>&1 || { echo "backend unavailable"; exit 1; }
$F add "magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=fbt-smoke-sintel" --stopped --quiet --json >/dev/null 2>&1

tmux new-session -d -s "$S" -x 150 -y 40 "$F tui 2>$ERR; sleep 60"

echo "T-001 first frame within 2s shows the torrent list with headers"
if wait_for 'Torrents \([0-9]+\)' 2 && cap | grep -q 'NAME' && wait_for 'fbt-smoke-sintel' 4; then ok T-001; else bad T-001 "no list"; fi

echo "T-002 details pane shows flags and the upload line"
if cap | grep -q 'sequential' && cap | grep -q 'upload'; then ok T-002; else bad T-002 "details missing"; fi

echo "T-003 Enter opens the file selector modal"
tmux send-keys -t "$S" Enter
if wait_for 'Files · fbt-smoke' 3; then ok T-003; else bad T-003 "no files modal"; fi
tmux send-keys -t "$S" Escape; sleep 0.2

echo "T-004 add prompt exposes F2/F3/F4 toggles and they flip"
tmux send-keys -t "$S" a
if wait_for 'F2 sequential:on' 1; then
  tmux send-keys -t "$S" F2 F4; sleep 0.3
  if cap | grep -q 'F2 sequential:off' && cap | grep -q 'F4 upload:on'; then ok T-004; else bad T-004 "toggles did not flip"; fi
else bad T-004 "prompt missing"; fi
tmux send-keys -t "$S" Escape; sleep 0.2

echo "T-005 screen switching: 2 = Search with input, 3 = RSS, 1 = back"
tmux send-keys -t "$S" 2; sleep 0.3
tmux send-keys -t "$S" Escape; sleep 0.2   # leave the search box so digits switch screens
tmux send-keys -t "$S" 3; sleep 0.3
r=$(cap | grep -c 'Feeds (')
tmux send-keys -t "$S" 1; sleep 0.3
t=$(cap | grep -c 'Torrents (')
if [ "$r" -ge 1 ] && [ "$t" -ge 1 ]; then ok T-005; else bad T-005 "rss=$r torrents=$t"; fi

echo "T-006 help overlay opens and closes"
tmux send-keys -t "$S" '?'; sleep 0.3
h=$(cap | grep -c 'GLOBAL')
tmux send-keys -t "$S" Escape; sleep 0.3
h2=$(cap | grep -c 'GLOBAL')
if [ "$h" -ge 1 ] && [ "$h2" -eq 0 ]; then ok T-006; else bad T-006 "help open=$h after=$h2"; fi

echo "T-007 q quits cleanly with nothing on stderr"
tmux send-keys -t "$S" q; sleep 0.6
if ! tmux list-panes -t "$S" -F '#{pane_current_command}' 2>/dev/null | grep -q fbittorrent && [ ! -s "$ERR" ]; then ok T-007; else bad T-007 "stderr: $(head -c 200 "$ERR")"; fi

echo; echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
