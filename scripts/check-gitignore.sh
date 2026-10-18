#!/usr/bin/env bash
# Feed a synthetic path list through `git check-ignore` and assert the plan §8.3
# rules: generated files and receipt data ignored, receipt README.md files kept.
# Log: target/test-logs/hygiene/gitignore.jsonl (one E.1 line per path, then a
# final line). Exit 1 when any path is not classified as expected.
set -euo pipefail
cd "$(dirname "$0")/.." || exit 1
log=target/test-logs/hygiene/gitignore.jsonl
mkdir -p "$(dirname "$log")"
: >"$log"
cases=(
  "agent_ergonomics_audit/audit/regression_tests/__pycache__/x.pyc ignored"
  "docs/validation/2026-10-09/big.bin ignored"
  "docs/validation/2026-10-09/README.md kept"
  "docs/validation/2026-10-12/cpu-root-cause/README.md kept"
  "docs/validation/2026-10-12/cpu-root-cause/final-512-monitor.jsonl ignored"
  "target/test-logs/x.jsonl ignored"
)
ts() { date -u +%Y-%m-%dT%H:%M:%S.%3NZ; }
step=0 failed=0
for entry in "${cases[@]}"; do
  path=${entry% *} expect=${entry##* }
  step=$((step + 1))
  # --no-index: classify by the rules alone, whether or not the path is tracked.
  line=$(git check-ignore -v -n --no-index -- "$path" || true)
  rule=${line%%$'\t'*}
  pattern=${rule#*:*:}
  if [[ -n "$pattern" && "$pattern" != '!'* ]]; then observed=ignored; else observed=kept; fi
  ok=true
  if [[ "$observed" != "$expect" ]]; then ok=false failed=$((failed + 1)); echo "gitignore: $path is $observed, expected $expect (rule: ${rule:-none})" >&2; fi
  printf '{"ts":"%s","suite":"hygiene","case":"gitignore","step":%d,"action":"git check-ignore %s","expect":"%s","observed":"%s by %s","ok":%s}\n' \
    "$(ts)" "$step" "$path" "$expect" "$observed" "${rule:-no rule}" "$ok" >>"$log"
done
verdict=true; [[ $failed -eq 0 ]] || verdict=false
printf '{"ts":"%s","suite":"hygiene","case":"gitignore","step":%d,"action":"summarise","expect":"every path classified as expected","observed":"%d paths, %d wrong","ok":%s,"final":true}\n' \
  "$(ts)" "$((step + 1))" "$step" "$failed" "$verdict" >>"$log"
echo "check-gitignore: $step paths, $failed wrong; log $log"
[[ $failed -eq 0 ]]
