#!/usr/bin/env python3
"""Exercise real log writing, stderr boundaries, forced failure, and the summary CLI."""
import contextlib
import io
import json
import os
import re
import subprocess
import sys
import time

import fbtlog
from fbtlog import Case, REPO, suite_status


def main():
    with Case("fbtlog_test", "logging_contract") as test:
        original_root = fbtlog.LOG_ROOT
        original_force = os.environ.pop("FBT_TEST_FORCE_FAIL", None)
        diagnostics = io.StringIO()
        try:
            fbtlog.LOG_ROOT = test.suite.directory / "fixtures"
            stale = fbtlog.LOG_ROOT / "fake_suite/stale.jsonl"
            stale.parent.mkdir(parents=True)
            stale.write_text("stale")
            with Case("fake_suite", "passing") as case:
                step = case.step("run child", "stdout stays separate and child stderr is captured")
                child = case.popen([sys.executable, "-c",
                                    "import sys; print('data'); sys.stderr.write('first stderr\\n')"],
                                   stdout=subprocess.PIPE, text=True)
                stdout, _ = child.communicate(timeout=5)
                stdout_path = case.suite.directory / "stdout.txt"
                stdout_path.write_text(stdout)
                time.sleep(.01)
                step.ok(f"Child exited {child.returncode}; stdout: {stdout.strip()}.",
                        artifacts=[stdout_path.relative_to(REPO)])
                case.check(stdout == "data\n", "child stdout remains caller-owned")
            os.environ["FBT_TEST_FORCE_FAIL"] = "fake_suite/forced_failure"
            with contextlib.redirect_stderr(diagnostics):
                with Case("fake_suite", "forced_failure") as case:
                    case.step("first check", "failure injection overrides a passing observation").ok("Actual result passed.")
                    child = case.popen([sys.executable, "-c", "import sys; sys.stderr.write('second stderr\\n')"])
                    child.wait(timeout=5)
                    case.step("later check", "the case continues after a failing step").ok("Later symptom recorded.")
            os.environ.pop("FBT_TEST_FORCE_FAIL", None)
            if original_force is not None:
                os.environ["FBT_TEST_FORCE_FAIL"] = original_force
            directory = fbtlog.LOG_ROOT / "fake_suite"
            passing = [json.loads(line) for line in (directory / "passing.jsonl").read_text().splitlines()]
            failing = [json.loads(line) for line in (directory / "forced_failure.jsonl").read_text().splitlines()]
            test.check(not stale.exists(), "starting a suite removes stale case logs")
            for rows in (passing, failing):
                test.check([row["step"] for row in rows[:-1]] == list(range(1, len(rows))),
                           "step numbers increment from one")
                test.check(all(row["ms"] >= 0 and re.fullmatch(r".*\.\d{3}Z", row["ts"]) for row in rows),
                           "every row has nonnegative elapsed milliseconds and millisecond UTC timestamp")
            test.check(passing[-1]["ok"] and not failing[-1]["ok"] and failing[-1]["failed"] == [1]
                       and failing[1]["ok"], "forced first-step failure survives final verdict and later steps run")
            test.check(passing[0]["ms"] >= 10 and stdout_path.exists()
                       and passing[0]["artifacts"] == [str(stdout_path.relative_to(REPO))],
                       "step timing includes the child and preserves artifacts")
            test.check((directory / "passing.daemon.log").read_text() == "first stderr\n"
                       and (directory / "forced_failure.daemon.log").read_text() == "second stderr\n",
                       "stderr is sliced at case byte boundaries")
            test.check(all(str(directory / name) in diagnostics.getvalue() for name in
                           ("forced_failure.jsonl", "forced_failure.daemon.log"))
                       and "second stderr" in diagnostics.getvalue(),
                       "failure prints both artifact paths and their tails")
            summary = json.loads((directory / "summary.json").read_text())
            result = subprocess.run([sys.executable, str(REPO / "scripts/testlog-summary.py"),
                                     str(fbtlog.LOG_ROOT)], capture_output=True, text=True, cwd="/")
            test.check(result.returncode == 1 and summary["cases"] == 2 and summary["passed"] == 1
                       and summary["failed"] == 1 and "fake_suite" in result.stdout,
                       "summary run outside the repository counts one failure and exits 1")
            # A stale green final/summary must not hide a failed intermediate row.
            failing[-1]["ok"] = True
            (directory / "forced_failure.jsonl").write_text("\n".join(map(json.dumps, failing)) + "\n")
            result = subprocess.run([sys.executable, str(REPO / "scripts/testlog-summary.py"),
                                     str(fbtlog.LOG_ROOT)], capture_output=True, text=True)
            test.check(result.returncode == 1, "summary rejects a failed step even under a green final verdict")
            failing[-1]["ok"] = False
            (directory / "forced_failure.jsonl").write_text("\n".join(map(json.dumps, failing)) + "\n")
        finally:
            fbtlog.LOG_ROOT = original_root
            os.environ.pop("FBT_TEST_FORCE_FAIL", None)
            if original_force is not None:
                os.environ["FBT_TEST_FORCE_FAIL"] = original_force
    return suite_status("fbtlog_test")


if __name__ == "__main__":
    sys.exit(main())
