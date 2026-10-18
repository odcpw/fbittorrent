#!/usr/bin/env python3
"""Review cell widths and rectangular boxes in PLAN §5 and Appendix C (not a CI gate)."""
import argparse
from pathlib import Path
import re
import sys
import unicodedata


def width(text):
    return sum(0 if unicodedata.combining(ch) or unicodedata.category(ch) in ("Cf", "Mn") else
               2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1 for ch in text)


def check(text):
    active, fenced, measured, declared, box = False, False, False, None, None
    errors, blocks = [], 0
    for number, line in enumerate(text.splitlines(), 1):
        if line.startswith("## ") and not fenced:
            active = bool(re.match(r"## (?:5[. ]|Appendix C\b)", line))
            fenced, measured, box, declared = False, False, None, None
        if not active:
            continue
        if not fenced:
            found = re.search(r"\b(80|120|160)(?:[x×]\d+|[- ](?:cells|columns|wide))", line)
            if found:
                declared = int(found[1])
        if line.startswith("```"):
            fenced = not fenced
            if fenced:
                measured = line.strip() in ("```", "```text", "```plaintext")
                blocks += int(measured)
            elif box is not None:
                errors.append(f"line {number}: box was not closed")
                box = None
            continue
        if not fenced or not measured:
            continue
        # Notes after a frame's closing edge describe the mockup; they are not
        # terminal cells (for example "focus starts on Cancel").
        if box is not None and line.lstrip().startswith("│") and "│" in line:
            line = line[:line.rfind("│") + 1]
        cells = width(line)
        limit = declared or (next((n for n in (80, 120, 160) if cells <= n), 160) if box is None else box[1])
        if cells > limit:
            errors.append(f"line {number}: {cells} cells overflow declared/inferred width {limit}")
        stripped = line.strip()
        if stripped.startswith("┌") and stripped.endswith("┐"):
            box = (cells, declared or next((n for n in (80, 120, 160) if cells <= n), 160))
        elif box is not None and stripped.startswith(("│", "├", "└")):
            if not stripped.endswith(("│", "┤", "┘")) or cells != box[0]:
                errors.append(f"line {number}: ragged box, {cells} cells; top edge has {box[0]}")
            if stripped.startswith("└"):
                box = None
    if box is not None:
        errors.append("unclosed box at end of plan")
    return blocks, errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", nargs="?", type=Path,
                        default=Path(__file__).resolve().parents[1] / "docs/plan-2026-10-18/PLAN.md")
    args = parser.parse_args()
    blocks, errors = check(args.plan.read_text())
    print(f"checked {blocks} mockup blocks in §5 and Appendix C")
    for error in errors:
        print(error, file=sys.stderr)
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
