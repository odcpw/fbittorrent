#!/usr/bin/env python3
"""Review PLAN mockups, adapted from the round-2 script in review-log.md (not a CI gate)."""
import argparse
from pathlib import Path
import re
import sys
import unicodedata


def width(text):
    return sum(0 if unicodedata.combining(ch) or unicodedata.category(ch) in ("Cf", "Mn") else
               2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1 for ch in text)


def review(text):
    active, fenced, measured, heading = False, False, False, ""
    reports, errors, rows = [], [], []

    def finish(opening, declared, closed):
        issues, details, frame = [], [], None
        widths = sorted({cells for _, cells, line in rows if line.lstrip().startswith(tuple("│┌└├╭╰"))})
        for number, cells, line in rows:
            if declared and cells > declared:
                issues.append(f"line {number}: {cells} cells overflow declared width {declared}")
                details.append(f"   OVERFLOW {cells}: {line}")
            stripped = line.strip()
            if len(widths) > 1 and stripped.startswith(tuple("│┌└├╭╰")) and cells != widths[-1]:
                issues.append(f"line {number}: ragged box, {cells} cells; expected {widths[-1]}")
                details.append(f"   RAGGED {cells} (expected {widths[-1]}): {line}")
            if stripped.startswith(("┌", "╭")):
                if frame is not None:
                    issues.append(f"line {number}: previous box was not closed")
                frame = number
                if not stripped.endswith(("┐", "╮")):
                    issues.append(f"line {number}: malformed top box edge")
            elif stripped.startswith(("│", "├", "└", "╰")):
                if not stripped.endswith(("│", "┤", "┘", "╯")):
                    issues.append(f"line {number}: malformed box edge")
                if stripped.startswith(("└", "╰")):
                    frame = None
        if frame is not None:
            issues.append(f"line {frame}: box was not closed")
        if not closed:
            issues.append(f"line {opening}: mockup fence was not closed")
        # Keep the review-round per-block summary and OVERFLOW/RAGGED diagnostics.
        # Structural checks are additions to that script, so name them separately.
        details.extend("   INVALID " + issue for issue in issues
                       if "overflow" not in issue and "ragged" not in issue)
        reports.append((f"L{opening} [{'FIX' if issues else 'OK'}] {heading[:70]!r}: "
                        f"declared {declared}, max {max((w for _, w, _ in rows), default=0)}, "
                        f"box widths {widths}", details, bool(issues)))
        errors.extend(issues)

    for number, line in enumerate(text.splitlines(), 1):
        if line.startswith("## ") and not fenced:
            active = bool(re.match(r"## (?:5[. ]|Appendix C\b)", line))
            heading = line.strip("*# ")
        if not active:
            continue
        if not fenced and (re.match(r"#{2,6} ", line) or line.startswith("**")):
            heading = line.strip("*# ")
        if line.startswith("```"):
            fenced = not fenced
            if fenced:
                measured = line.strip() in ("```", "```text", "```plaintext")
                opening, rows = number, []
                found = re.search(r"\b(\d{2,3})(?:\s*[x×]\s*\d+\b|[- ](?:cells|(?:text )?columns|wide)\b)", heading)
                declared = int(found[1]) if found else None
            elif measured:
                finish(opening, declared, closed=True)
            continue
        if not fenced or not measured:
            continue
        # Notes after a frame's closing edge describe the mockup; they are not
        # terminal cells (for example "focus starts on Cancel").
        if line.lstrip().startswith("│"):
            line = line[:line.rfind("│") + 1]
        rows.append((number, width(line), line))
    if fenced and measured:
        finish(opening, declared, closed=False)
    return reports, errors


def check(text):
    reports, errors = review(text)
    return len(reports), errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", nargs="?", type=Path,
                        default=Path(__file__).resolve().parents[1] / "docs/plan-2026-10-18/PLAN.md")
    args = parser.parse_args()
    reports, errors = review(args.plan.read_text())
    for summary, details, _ in reports:
        print(summary)
        for detail in details:
            print(detail)
    print(f"{len(reports)} blocks measured, {sum(failed for _, _, failed in reports)} need fixing")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
