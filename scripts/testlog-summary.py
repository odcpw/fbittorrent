#!/usr/bin/env python3
"""Summarize Appendix E.1 JSONL; malformed/incomplete cases also fail the run."""
import argparse
import json
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path,
                        default=Path(__file__).resolve().parents[1] / "target/test-logs")
    args = parser.parse_args()
    bad = False
    print("suite                    cases  passes  failures  slowest five (ms)")
    for directory in sorted(args.root.iterdir()) if args.root.exists() else []:
        if not directory.is_dir():
            continue
        cases = []
        for path in sorted(directory.glob("*.jsonl")):
            try:
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                final = rows[-1]
                ok = (final.get("final") is True and final.get("ok") is True
                      and all(row.get("ok") is True for row in rows))
                cases.append(dict(case=path.stem, ok=ok, ms=final.get("ms", 0)))
            except (ValueError, IndexError, TypeError, AttributeError) as error:
                print(f"{path}: invalid case log: {error}", file=sys.stderr)
                cases.append(dict(case=path.stem, ok=False, ms=0))
        if not cases:
            continue
        failed = sum(not row["ok"] for row in cases)
        slowest = sorted(cases, key=lambda row: row["ms"], reverse=True)[:5]
        times = ", ".join(f"{row['case']}={row['ms']}" for row in slowest)
        print(f"{directory.name:<24} {len(cases):5} {len(cases)-failed:7} {failed:9}  {times}")
        bad |= failed != 0
    return int(bad)


if __name__ == "__main__":
    sys.exit(main())
