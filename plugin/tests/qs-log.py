#!/usr/bin/env python3
"""Run Quickshell and translate prefixed FBTQS markers into Appendix E.1 logs."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] /
                       "agent_ergonomics_audit/audit/regression_tests"))
import fbtlog
from fbtlog import Case, suite_status


def run(suite, command, on_line=None):
    cases = {}
    capture_suite = fbtlog.get_suite(suite)

    def get(name):
        if name not in cases:
            cases[name] = Case(suite, name).__enter__()
        return cases[name]

    with subprocess.Popen(command, env=dict(os.environ, NO_COLOR="1"),
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) as child:
        for line in child.stdout:
            if on_line is not None:
                on_line(line)
            with capture_suite.stderr_path.open("a") as capture:
                capture.write(line)
            # Quickshell decorates both stdout and stderr (e.g. DEBUG qml:).
            match = re.search(r"FBTQS-(STEP|PASS|FAIL)\s+(.*)", line)
            if not match:
                continue
            kind, payload = match.groups()
            try:
                if kind == "STEP":
                    data = json.loads(payload)
                    case = get(data["case"])
                    step = case.step(data["action"], data["expect"])
                    step.started -= max(0, data.get("ms", 0)) / 1000
                    if not isinstance(data["ok"], bool):
                        raise ValueError("STEP ok must be a boolean")
                    step.close(data["ok"], data["observed"], data.get("artifacts", []))
                else:
                    name, _, diff = payload.partition(" ")
                    case = get(name.strip())
                    if kind == "FAIL" or not case.steps:
                        case.step("QML verdict", "QML case passes").close(kind == "PASS", diff or kind)
                    case.__exit__(None, None, None)
                    # A later duplicate verdict is malformed, not a second case.
                    cases[name] = None
            except (ValueError, KeyError, TypeError, AttributeError) as error:
                get("converter_error").step("decode QML marker", "valid FBTQS marker").fail(
                    f"{error}: {payload}")
        code = child.wait()
    for case in cases.values():
        if case is not None:
            case.step("receive verdict", "QML emits a PASS or FAIL verdict").fail(
                f"Process exited {code} without a verdict.")
            case.__exit__(None, None, None)
    if code or not cases:
        with Case(suite, "process_exit") as case:
            case.step("wait for Quickshell", "process exits 0 and emits test markers").fail(
                f"Exit {code}; cases: {len(cases)}")
    return int(bool(code) or bool(suite_status(suite)))


def self_test():
    with Case("qs_log_test", "marker_conversion") as case:
        # Deliberate failing input stays below the positive test case's directory.
        original = fbtlog.LOG_ROOT
        fbtlog.LOG_ROOT = case.suite.directory / "fixtures"
        try:
            data = dict(case="prefixed_step", action="read width", expect="width is 80 cells",
                        observed="Width is 80 cells.", ok=True, ms=7, artifacts=["capture.txt"])
            lines = ["DEBUG qml: FBTQS-STEP " + json.dumps(data),
                     "DEBUG qml: FBTQS-PASS prefixed_step",
                     "DEBUG qml: FBTQS-FAIL failed_case expected 80; observed 79"]
            command = [sys.executable, "-c", "import os,sys; assert os.environ['NO_COLOR']=='1'; "
                       "sys.stderr.write(" + repr("\n".join(lines) + "\n") + ")"]
            result = run("canned_markers", command)
            logs = fbtlog.LOG_ROOT / "canned_markers"
            first = [json.loads(line) for line in (logs / "prefixed_step.jsonl").read_text().splitlines()]
            summary = json.loads((logs / "summary.json").read_text())
            case.check(result == 1 and summary["passed"] == 1 and summary["failed"] == 1,
                       "three decorated markers produce one passing and one failing case, exit 1")
            case.check(first[0]["ms"] >= 7 and first[0]["artifacts"] == ["capture.txt"]
                       and first[-1]["final"] and first[-1]["ok"],
                       "step timing and artifacts survive conversion; PASS closes the case")
        finally:
            fbtlog.LOG_ROOT = original
    return suite_status("qs_log_test")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--suite")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.self_test:
        sys.exit(self_test())
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not args.suite or not command:
        parser.error("expected --suite <name> -- qs -p <dir> ...")
    sys.exit(run(args.suite, command))
