#!/usr/bin/env bash
# Agent-ergonomics regression tests for fbittorrent.
# Each R-NNN pins one applied recommendation. Run: agent_ergonomics_audit/audit/regression_tests/run.sh [binary]
# Exit 0 when every test passes, 1 otherwise. Needs a reachable (or auto-startable) qbittorrent-nox for the R-01x tests.
set -u
F="${1:-fbittorrent}"
pass=0; fail=0
ok()   { pass=$((pass+1)); echo "  ok   $1"; }
bad()  { fail=$((fail+1)); echo "  FAIL $1: $2"; }
json() { python3 -c 'import sys,json; json.load(sys.stdin)' 2>/dev/null; }

echo "R-001 capabilities --json is valid JSON with an exit-code dictionary"
if $F capabilities --json | python3 -c 'import sys,json; d=json.load(sys.stdin); assert d["ok"]; assert len(d["exit_codes"])>=8; assert d["contract_version"]' 2>/dev/null; then ok R-001; else bad R-001 "invalid or incomplete"; fi

echo "R-002 bare invocation without a TTY prints triage JSON instead of a TUI"
if out=$($F </dev/null 2>/dev/null) && echo "$out" | python3 -c 'import sys,json; d=json.load(sys.stdin); assert "backend" in d and "next_commands" in d' 2>/dev/null; then ok R-002; else bad R-002 "no triage JSON"; fi

echo "R-003 robot-docs prints the handbook"
if $F robot-docs | grep -q "DOWNLOAD SOMETHING" && $F docs | grep -q "EXIT CODES"; then ok R-003; else bad R-003 "handbook missing sections"; fi

echo "R-004 unknown selector -> exit 6 with JSON envelope and hint"
$F stop zzzz9999 --json >/tmp/r004.out 2>/dev/null; code=$?
if [ "$code" = 6 ] && python3 -c 'import json; d=json.load(open("/tmp/r004.out")); assert d["ok"] is False and d["code"]==6 and d["hint"]' 2>/dev/null; then ok R-004; else bad R-004 "exit $code"; fi

echo "R-005 delete --files without --yes -> exit 3, hint names --yes"
$F delete all --files >/tmp/r005.err 2>&1 </dev/null; code=$?
# With no torrents the selector resolves to an empty set, which is still a refusal.
if [ "$code" = 3 ] && grep -q -- '--yes' /tmp/r005.err; then ok R-005; else
  # if there are no torrents, `all` may resolve to zero and still refuse; any other code is a failure
  bad R-005 "exit $code: $(head -c 200 /tmp/r005.err)"; fi

echo "R-006 flag typo gets a suggestion and exit 2"
$F list --jsno >/tmp/r006.err 2>&1; code=$?
if [ "$code" = 2 ] && grep -q -- '--json' /tmp/r006.err; then ok R-006; else bad R-006 "exit $code"; fi

echo "R-007 stdout is data only: status --json parses with stderr discarded"
if $F status --json 2>/dev/null | json; then ok R-007; else bad R-007 "stdout not pure JSON"; fi

echo "R-008 capabilities output is byte-identical across runs"
if diff <($F capabilities) <($F capabilities) >/dev/null; then ok R-008; else bad R-008 "differs"; fi

echo "R-009 --help mentions --json, exit codes and robot-docs"
if $F --help | grep -q -- '--json' && $F --help | grep -qi 'exit codes' && $F --help | grep -q 'robot-docs'; then ok R-009; else bad R-009 "help lacks agent pointers"; fi

echo "R-010 aliases resolve (ls, rm, pause, resume, find, info, docs)"
allok=1
for a in ls rm pause resume find info; do $F "$a" --help >/dev/null 2>&1 || { allok=0; echo "    alias $a broken"; }; done
if [ $allok = 1 ]; then ok R-010; else bad R-010 "alias missing"; fi

echo "R-011 empty selector -> exit 2 with a copy-pasteable hint"
$F stop --json >/tmp/r011.out 2>/dev/null; code=$?
if [ "$code" = 2 ] && grep -q 'list --json' /tmp/r011.out; then ok R-011; else bad R-011 "exit $code"; fi

echo "R-012 add --json returns the hash; show/wait/delete round trip (needs backend)"
if $F up --json >/dev/null 2>&1; then
  M='magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=fbt-regression-sintel'
  h=$($F add "$M" --stopped --quiet --json 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["hash"])' 2>/dev/null)
  if [ "$h" = "08ada5a7a6183aae1e09d831df6748d566095a10" ]; then
    $F show fbt-regression --json 2>/dev/null | python3 -c 'import sys,json; d=json.load(sys.stdin); assert d["ok"] and "open_target" in d and "files" in d' 2>/dev/null && ok "R-012 add+show" || bad "R-012 show" "bad payload"
    $F wait 08ada5a7 --timeout 2 --json >/tmp/r012.out 2>/dev/null; code=$?
    [ "$code" = 8 ] && grep -q '"code_name":"timeout"' /tmp/r012.out && ok "R-012 wait timeout=8" || bad "R-012 wait" "exit $code"
    $F delete 08ada5a7 --files --json >/tmp/r012b.out 2>/dev/null; code=$?
    [ "$code" = 3 ] && ok "R-012 delete safety" || bad "R-012 delete safety" "exit $code"
    $F delete 08ada5a7 --files --yes --json 2>/dev/null | python3 -c 'import sys,json; d=json.load(sys.stdin); assert d["ok"] and d["count"]==1' 2>/dev/null && ok "R-012 delete --yes" || bad "R-012 delete --yes" "not removed"
  else
    bad R-012 "add did not return the hash (got '$h')"
  fi
else
  echo "  skip R-012 (backend unavailable)"
fi

echo
echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
