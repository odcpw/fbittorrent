#!/usr/bin/env python3
"""Exercise real log writing, stderr boundaries, forced failure, and the summary CLI."""
import contextlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import time
from unittest.mock import patch

import fbtlog
from fbtlog import Case, REPO, suite_status
from native import NativeHarness


def polling_cli_exits():
    with Case("fbtlog_test", "polling_cli_exits") as test:
        fixtures = test.suite.directory / "fixtures/polling"
        fixtures.mkdir(parents=True)
        binary = fixtures / "cli.py"
        binary.write_text(f"#!{sys.executable}\n" + '''import json, os, sys
from pathlib import Path
counter = Path(os.environ["FBT_POLL_COUNTER"])
attempt = int(counter.read_text()) + 1 if counter.exists() else 1
counter.write_text(str(attempt))
print(json.dumps({"ready": attempt > 1}))
sys.exit(1 if attempt == 1 else 0)
''')
        binary.chmod(0o700)
        for mode in ("normal", "verbose"):
            diagnostics = io.StringIO()
            first_seen = []
            suite = "poll_" + mode
            with patch.object(fbtlog, "LOG_ROOT", fixtures), patch.dict(
                    os.environ, FBT_TEST_VERBOSE="1" if mode == "verbose" else "0",
                    FBT_TEST_FORCE_FAIL=""), contextlib.redirect_stderr(diagnostics), \
                    contextlib.redirect_stdout(diagnostics):
                with Case(suite, "retry_after_failure") as case:
                    harness = NativeHarness(lambda: case, str(binary), dict(
                        os.environ, FBT_POLL_COUNTER=str(fixtures / f"{mode}.counter")))

                    def ready():
                        result = harness.cli("status")
                        if not result["ready"]:
                            rows = [json.loads(line) for line in case.path.read_text().splitlines()]
                            first_seen.append(len(rows) == 1 and rows[0]["ok"] is False)
                        return result["ready"]

                    result = harness.wait_for(ready, "CLI ready", seconds=2)
                status = suite_status(suite)
                # Failed captures must also be immediate; only successful
                # intermediate panes may be omitted in normal mode.
                with Case(suite, "pane_polls") as panes:
                    harness = NativeHarness(lambda: panes, str(binary), os.environ)
                    attempts = []

                    def capture():
                        attempts.append(True)
                        if len(attempts) == 1:
                            harness.record("capture terminal pane", "pane captured", "capture failed", False)
                        harness.record("capture terminal pane", "pane captured",
                                       "asserted pane" if len(attempts) == 2 else "intermediate pane")
                        return len(attempts) == 2

                    harness.wait_for(capture, "pane ready", seconds=2)
            diagnostic_path = fixtures / f"{mode}.diagnostics.txt"
            diagnostic_path.write_text(diagnostics.getvalue())
            rows = [json.loads(line) for line in case.path.read_text().splitlines()]
            calls = [row for row in rows if row.get("action") == "CLI status"]
            test.check(first_seen == [True], f"{mode}: failed CLI result is written before the next poll")
            test.check(result and len(calls) == 2 and calls[0]["ok"] is False
                       and calls[0]["observed"].startswith("Exit 1;") and calls[1]["ok"] is True
                       and calls[1]["observed"].startswith("Exit 0;"),
                       f"{mode}: both real CLI exits remain in JSONL after a successful retry")
            test.check(status == 1 and rows[-1]["ok"] is False and rows[-1]["failed"] == [1],
                       f"{mode}: a successful retry cannot erase the failed case verdict")
            captures = [row for row in map(json.loads, panes.path.read_text().splitlines())
                        if row.get("action") == "capture terminal pane"]
            test.check([row["observed"] for row in captures] == (
                ["capture failed", "intermediate pane", "asserted pane"] if mode == "verbose"
                else ["capture failed", "asserted pane"]),
                f"{mode}: suppress only successful intermediate pane captures")


def early_abort_summary():
    with Case("fbtlog_test", "early_abort_summary") as test:
        root = test.suite.directory / "fixtures/umbrella"
        regression = root / "agent_ergonomics_audit/audit/regression_tests"
        regression.mkdir(parents=True)
        (root / "scripts").mkdir()
        (root / "plugin/tests").mkdir(parents=True)
        umbrella = regression / "umbrella.sh"
        shutil.copyfile(REPO / "agent_ergonomics_audit/audit/regression_tests/all.sh", umbrella)
        shutil.copyfile(REPO / "scripts/testlog-summary.py", root / "scripts/testlog-summary.py")
        (regression / "fbtlog_test.py").write_text("pass\n")
        (root / "plugin/tests/qs-log.py").write_text("pass\n")
        for name in ("check-gitignore.sh", "check-nova3-licences.sh"):
            path = root / "scripts" / name
            path.write_text("#!/bin/bash\nexit 0\n")
            path.chmod(0o700)
        home = root / "home"
        shims = home / ".rch/shims"
        shims.mkdir(parents=True)
        # This fixture cannot compile anything: its cargo records the stages
        # and aborts at the release stage, without invoking the real toolchain.
        cargo = shims / "cargo"
        cargo.write_text('''#!/bin/bash
echo "$1" >> "$HOME/cargo-calls.txt"
case "$1" in
  test) exit 0 ;;
  build) echo "fixture release abort" >&2; exit 137 ;;
  *) exit 2 ;;
esac
''')
        cargo.chmod(0o700)
        rustc = shims / "rustc"
        rustc.write_text("#!/bin/bash\necho 'host: x86_64-unknown-linux-gnu'\n")
        rustc.chmod(0o700)
        logs = root / "target/test-logs"
        native = logs / "native"
        native.mkdir(parents=True)
        (native / "stale_native_case.jsonl").write_text(json.dumps(dict(
            final=True, ok=True, ms=1)) + "\n")
        (native / "summary.json").write_text('{"cases": 1, "passed": 1, "failed": 0}\n')
        archive = logs / "fbt-p0-15-3hq/keep.jsonl"
        archive.parent.mkdir()
        archive.write_text('{"final": true, "ok": false, "ms": 1}\n')
        archived = archive.read_bytes()
        result = subprocess.run(["bash", str(umbrella)], capture_output=True, text=True,
                                timeout=20, env=dict(os.environ, HOME=str(home),
                                    CARGO_BUILD_JOBS="4", FBT_TEST_LEGACY="0", FBT_TEST_WIDGET="0",
                                    FBT_TEST_FORCE_FAIL=""))
        (root / "stdout.log").write_text(result.stdout)
        (root / "stderr.log").write_text(result.stderr)
        test.check(result.returncode == 137 and (home / "cargo-calls.txt").read_text() == "test\nbuild\n",
                   "early abort retains the release failure exit code after the final summary")
        test.check(not list(native.glob("*.jsonl")) and not (native / "summary.json").exists(),
                   "all scheduled suite logs are cleared before a release stage can abort")
        test.check(re.search(r"^native\s+.*not run$", result.stdout, re.MULTILINE) is not None
                   and "stale_native_case" not in result.stdout
                   and not re.search(r"^native\s+1\s+1\s+0", result.stdout, re.MULTILINE),
                   "aborted run reports native as not run without displaying stale success")
        test.check(archive.read_bytes() == archived and "fbt-p0-15-3hq" not in result.stdout,
                   "archived bead evidence is preserved and excluded from the scheduled summary")
        test.check(all(re.search(rf"^{suite}\s+.*not run$", result.stdout, re.MULTILINE)
                       for suite in ("fbtlog_test", "qs_log_test", "hygiene", "native")),
                   "every scheduled suite without current logs is labelled not run")


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
    polling_cli_exits()
    early_abort_summary()
    return suite_status("fbtlog_test")


if __name__ == "__main__":
    sys.exit(main())
