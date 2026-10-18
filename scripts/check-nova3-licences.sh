#!/usr/bin/env bash
# Assert that every file under vendor/nova3/ has a row in the licence table of
# vendor/nova3/README.md (P0-03), and log the table.
# Log: target/test-logs/hygiene/nova3-licences.jsonl. Exit 1 on a missing row.
set -euo pipefail
cd "$(dirname "$0")/.." || exit 1
log=target/test-logs/hygiene/nova3-licences.jsonl
readme=vendor/nova3/README.md
mkdir -p "$(dirname "$log")"
: >"$log"
ts() { date -u +%Y-%m-%dT%H:%M:%S.%3NZ; }
json() { python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"; }
step=0 missing=0
while IFS= read -r file; do
  name=${file#vendor/nova3/}
  step=$((step + 1))
  row=$(grep -F -m1 "| \`$name\` |" "$readme" || true)
  ok=true observed="row: $row"
  if [[ -z "$row" ]]; then ok=false missing=$((missing + 1)) observed="no row"; echo "nova3: no licence row for $name in $readme" >&2; fi
  printf '{"ts":"%s","suite":"hygiene","case":"nova3_licences","step":%d,"action":%s,"expect":"a licence row for the file","observed":%s,"ok":%s}\n' \
    "$(ts)" "$step" "$(json "look up $name in $readme")" "$(json "$observed")" "$ok" >>"$log"
done < <(find vendor/nova3 -type f | LC_ALL=C sort)
verdict=true; [[ $missing -eq 0 ]] || verdict=false
printf '{"ts":"%s","suite":"hygiene","case":"nova3_licences","step":%d,"action":"summarise","expect":"every vendor/nova3 file has a licence row","observed":"%d files, %d without a row","ok":%s,"final":true}\n' \
  "$(ts)" "$((step + 1))" "$step" "$missing" "$verdict" >>"$log"
echo "check-nova3-licences: $step files, $missing without a row; log $log"
[[ $missing -eq 0 ]]
