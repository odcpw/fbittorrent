#!/usr/bin/env bash
# Check the Markdown links from tracked (or committable) *.md files into
# docs/validation/. Only the receipt README.md summaries are kept in git (owner
# decision Q4, 2026-10-18), so a link to a README.md must resolve; a link to any
# other receipt file points at data deleted on 2026-10-18 and is listed, not
# failed. Run by hand whenever receipts change (P0-01); not a CI job (DL-59).
# Log: target/test-logs/docs/links.jsonl, one E.1 line per link plus a final line.
# Exit: 0 when every link to a receipt README.md resolves, 1 otherwise.
set -euo pipefail
cd "$(dirname "$0")/.." || exit 1
log=target/test-logs/docs/links.jsonl
mkdir -p "$(dirname "$log")"
git ls-files -z -co --exclude-standard -- '*.md' | python3 -c '
import json, os, re, sys, time
from datetime import datetime, timezone
root = os.getcwd()
log = sys.argv[1]
files = [f for f in sys.stdin.read().split("\0") if f and os.path.isfile(f)]
link = re.compile(r"\]\(([^)\s]+)\)")
def ts():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
step = 0
broken = 0
deleted = 0
start = time.monotonic()
with open(log, "w", encoding="utf-8") as out:
    for md in sorted(files):
        with open(md, encoding="utf-8", errors="replace") as fh:
            for ln, line in enumerate(fh, 1):
                for m in link.finditer(line):
                    target = m.group(1).split("#", 1)[0]
                    if not target or re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", target):
                        continue
                    path = target if target.startswith("/") else os.path.join(os.path.dirname(md), target)
                    rel = os.path.relpath(os.path.abspath(path), root)
                    if not rel.startswith("docs/validation/"):
                        continue
                    step += 1
                    found = os.path.exists(rel)
                    summary = os.path.basename(rel) == "README.md"
                    if summary:
                        expect, ok = "exists", found
                    else:
                        expect, ok = "exists, or receipt data deleted 2026-10-18 (Q4)", True
                        if not found:
                            deleted += 1
                    if not ok:
                        broken += 1
                        print(f"broken: {md}:{ln} -> {target}", file=sys.stderr)
                    observed = "found" if found else ("missing" if summary else "missing: deleted receipt data")
                    out.write(json.dumps({"ts": ts(), "suite": "docs", "case": "links", "step": step,
                        "action": f"{md}:{ln} -> {target}", "expect": expect,
                        "observed": observed, "ok": ok}) + "\n")
    verdict = broken == 0
    out.write(json.dumps({"ts": ts(), "suite": "docs", "case": "links", "step": step + 1,
        "action": "summarise", "expect": "every link to a receipt README.md resolves",
        "observed": f"{step} links checked, {broken} broken, {deleted} to deleted receipt data",
        "ok": verdict, "final": True, "ms": int((time.monotonic() - start) * 1000)}) + "\n")
print(f"check-doc-links: {step} links into docs/validation checked, {broken} broken, {deleted} to deleted receipt data; log {log}")
sys.exit(0 if verdict else 1)
' "$log"
