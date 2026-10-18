#!/usr/bin/env python3
"""Manual parity tests for the review-round mockwidth tool (not a CI job)."""
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "agent_ergonomics_audit/audit/regression_tests"))
from fbtlog import Case, suite_status


def run_plan(case, text):
    with tempfile.TemporaryDirectory(prefix="fbt-mockwidth-") as directory:
        plan = Path(directory) / "PLAN.md"
        plan.write_text(text)
        child = case.popen([sys.executable, str(ROOT / "scripts/mockwidth.py"), str(plan)],
                           stdout=subprocess.PIPE, text=True)
        stdout, _ = child.communicate(timeout=10)
        return child.returncode, stdout


def spaced_dimensions():
    with Case("mockwidth_test", "spaced_dimensions") as case:
        for dimensions in ("80 × 24", "80x24", "80 x 24", "80×24", "80 columns", "80 text columns",
                           "80-cells", "80 wide"):
            code, output = run_plan(case, "## 5. TUI\n**Screen at " + dimensions + ":**\n\n```\n" +
                                    "x" * 81 + "\n```\n## 6. Roadmap\n")
            case.check(code == 1 and "[FIX]" in output and "declared 80, max 81" in output
                       and "   OVERFLOW 81: " + "x" * 81 in output
                       and output.endswith("1 blocks measured, 1 need fixing\n"),
                       f"{dimensions}: an 81-cell row fails with the reviewed overflow diagnostics", output)


def heading_width_reset():
    with Case("mockwidth_test", "heading_width_reset") as case:
        code, output = run_plan(case, "## 5. TUI\n**Narrow at 80x24:**\n```\n" + "x" * 80 +
                                "\n```\n**Undeclared dashboard:**\n```\n" + "x" * 90 +
                                "\n```\n### A new heading\n```\n" + "x" * 90 + "\n```\n## 6. Roadmap\n")
        expected = ("L3 [OK] 'Narrow at 80x24:': declared 80, max 80, box widths []\n"
                    "L7 [OK] 'Undeclared dashboard:': declared None, max 90, box widths []\n"
                    "L11 [OK] 'A new heading': declared None, max 90, box widths []\n"
                    "3 blocks measured, 0 need fixing\n")
        case.check(code == 0 and output == expected,
                   "each bold or Markdown heading resets its declared width; per-block diagnostics match", output)
        code, output = run_plan(case, "## 5. TUI\n**Screen at 80x24:**\n```rust\n" + "x" * 81 +
                                "\n```\n## 6. Roadmap\n")
        case.check(code == 0 and output == "0 blocks measured, 0 need fixing\n",
                   "Rust examples remain outside the measured screen mockups", output)


def rounded_frames():
    with Case("mockwidth_test", "rounded_frames") as case:
        code, output = run_plan(case, "## 5. TUI\n**Rounded frame:**\n```\n╭──╮\n│ok│\n╰──╯\n```\n")
        case.check(code == 0 and output ==
                   "L3 [OK] 'Rounded frame:': declared None, max 4, box widths [4]\n"
                   "1 blocks measured, 0 need fixing\n",
                   "rounded corners form a closed four-cell frame and appear in box widths", output)
        for edge, body in (("row", "╭──╮\n│x│\n╰──╯"), ("bottom", "╭──╮\n│ok│\n╰─╯")):
            code, output = run_plan(case, "## 5. TUI\n**Rounded frame:**\n```\n" + body + "\n```\n")
            case.check(code == 1 and "[FIX]" in output and "box widths [3, 4]" in output
                       and "   RAGGED 3 (expected 4): " in output
                       and output.endswith("1 blocks measured, 1 need fixing\n"),
                       f"a short rounded frame {edge} fails with the reviewed ragged diagnostics", output)
        code, output = run_plan(case, "## 5. TUI\n**Rounded frame:**\n```\n╭──╮\n│ok│\n```\n")
        case.check(code == 1 and "box was not closed" in output,
                   "rounded frames retain the existing unclosed-box check", output)
        code, output = run_plan(case, "## 5. TUI\n**Rounded frame:**\n```\n╭──╮\n│界│  outside annotation\n"
                                "│e\u0301\ue000│\n╰──╯\n```\n")
        case.check(code == 0 and "max 4, box widths [4]" in output,
                   "wide, combining and private-use glyphs retain their cell widths; external notes are excluded", output)
        code, output = run_plan(case, "## 5. TUI\n```\n╭──╮\n## heading-like text inside the frame\n")
        case.check(code == 1 and "box was not closed" in output and "fence was not closed" in output,
                   "an apparent heading inside an unclosed mockup cannot escape validation", output)


def main():
    spaced_dimensions()
    heading_width_reset()
    rounded_frames()
    return suite_status("mockwidth_test")


if __name__ == "__main__":
    sys.exit(main())
