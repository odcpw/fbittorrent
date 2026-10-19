#!/usr/bin/env python3
"""Regression tests for CI failure logs, summary fixtures, flags and resource thresholds."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from fbtlog import Case, REPO, suite_status
from resources import compare

HERE = Path(__file__).resolve().parent


def summary_fixture():
    with Case("ci_gates", "summary_fixture") as case:
        fixture = HERE / "fixtures/testlogs"
        result = subprocess.run([sys.executable, str(REPO / "scripts/testlog-summary.py"), str(fixture)],
                                cwd="/", capture_output=True, text=True, timeout=10)
        case.check(result.returncode == 1, "two-suite fixture with one failure exits 1", result.stderr)
        rows = {parts[0]: parts for line in result.stdout.splitlines() if (parts := line.split()) and parts[0] in ("alpha", "beta")}
        case.check("alpha" in rows and "beta" in rows and int(rows["alpha"][2]) > 0
                   and int(rows["alpha"][3]) == 0 and int(rows["beta"][3]) > 0,
                   "summary exposes passing and failing suites without depending on column spacing", result.stdout)


def command_logging():
    with Case("ci_gates", "command_logging") as case:
        root = case.suite.directory / "fixtures/runner"
        regression = root / "agent_ergonomics_audit/audit/regression_tests"
        regression.mkdir(parents=True)
        for name in ("ci.py", "fbtlog.py"):
            shutil.copyfile(HERE / name, regression / name)
        for name, program, scan, expected in (
            ("green", "import sys; print('ready'); sys.stderr.write('child diagnostic\\n')", False, 0),
            ("red", "import sys; print('planted failure'); sys.exit(1)", False, 1),
            ("worker", "import sys; sys.stderr.write(\"thread 'native-worker' panicked at planted failure\\n\")", True, 1),
            ("state", "import os; from pathlib import Path; p=Path(os.environ['XDG_STATE_HOME']); "
             "assert os.environ['TMPDIR']==str(p); (p/'owned').write_text('fixture'); print(p)", False, 0),
        ):
            command = [sys.executable, str(regression / "ci.py"), "--suite", name]
            if scan:
                command.append("--scan-workers")
            if name == "state":
                command.append("--isolate-state")
            result = subprocess.run([*command, "--", sys.executable, "-c", program], capture_output=True,
                                    text=True, timeout=10, env=dict(os.environ, FBT_TEST_FORCE_FAIL=""))
            directory = root / "target/test-logs" / name
            rows = [json.loads(line) for line in (directory / "command.jsonl").read_text().splitlines()]
            case.check(result.returncode == expected and rows[-1]["final"]
                       and rows[-1]["ok"] == (expected == 0), f"{name}: command and final JSONL verdict agree",
                       f"Exit {result.returncode}; final {rows[-1]}")
            case.check((directory / "command.stdout.log").exists()
                       and (directory / "command.daemon.log").exists(), f"{name}: stdout and stderr artifacts retained")
            if name == "worker":
                case.check("Exit 0;" in rows[0]["observed"] and not rows[0]["ok"],
                           "background panic invalidates an otherwise successful child exit")
            if name == "state":
                state = Path((directory / "command.stdout.log").read_text().strip())
                case.check(state.name.startswith("fbt-ci-state-") and not state.exists(),
                           "older fixtures receive isolated XDG state and the wrapper removes it afterward")


def resource_thresholds():
    with Case("ci_gates", "resource_thresholds") as case:
        ref = dict(host="fixture", machine="fixture", cpu="fixture", idle_seconds=10, active_mib=32, active_mib_s=4,
                   ratings_fixture="local fixture")
        config = {mode: dict(cpu_s=[10, 10, 10], rss_kb=[100, 105, 110]) for mode in ("idle", "active")}
        baseline = dict(reference=ref, configs=config)
        current = {mode: dict(cpu_s=[11], rss_kb=[135]) for mode in config}
        results = compare(baseline, current, ref)
        case.check(all(row["ok"] for row in results), "10-percent CPU floor and three-spread RSS boundary pass")
        case.check([row["limit"] for row in results] == [11, 135, 11, 135],
                   "threshold derivation is median plus max(10 percent median, three times range)")
        for mode in config:
            for metric in ("cpu_s", "rss_kb"):
                planted = copy.deepcopy(current)
                planted[mode][metric].append(max(planted[mode][metric]) + .001)
                outcomes = compare(baseline, planted, ref)
                case.check(sum(not row["ok"] for row in outcomes) == 1,
                           f"one {mode}/{metric} regression cannot be hidden by other samples")
        for changed in (dict(ref, host="other"), dict(ref, ratings_fixture="different workload")):
            try:
                compare(baseline, current, changed)
            except ValueError as error:
                case.check("reference mismatch" in str(error), "incompatible host is rejected rather than silently compared")
            else:
                case.check(False, "incompatible host is rejected")
        invalid = copy.deepcopy(baseline)
        invalid["configs"]["idle"]["cpu_s"] = [10, 10]
        try:
            compare(invalid, current, ref)
        except ValueError:
            case.check(True, "a fabricated two-run baseline is rejected")
        else:
            case.check(False, "baseline requires three measured runs")


def umbrella_flags():
    with Case("ci_gates", "umbrella_flags") as case:
        sys.path.insert(0, str(REPO / "scripts"))
        from gate_test import run_named
        ok, output = run_named("test_full_optins_and_default_coverage", "test_all_alias_help_does_not_start_build",
                               "test_retry_is_serial_and_retains_both_attempts", "test_missing_final_cannot_pass",
                               "test_optional_commands_dispatch_and_missing_script_fails")
        case.check(ok, "full gate preserves default/opt-in coverage and retries failures once in isolation", output)


def native_tui_wiring():
    with Case("ci_gates", "native_tui_wiring") as case:
        sys.path.insert(0, str(REPO / "scripts"))
        import gate
        registry = gate.modules()
        selected = {check.name for check in gate.cases("full", Path("/missing/binary"), {}, registry)}
        case.check(all("tui_smoke/" + name in selected for name in registry["tui_smoke"].CASES),
                   "full gate runs every registered native terminal smoke", str(sorted(selected)))
        case.check(any(check.name == "tui_controls" for check in gate.full_checks({}, Path("/missing/binary"))),
                   "full gate retains the planted terminal failure control")
        unused = case.suite.directory / "fixtures/list_only"
        listed = subprocess.run([sys.executable, str(HERE / "tui_smoke.py"), "/missing/fbittorrent", "--list", "--log-root", str(unused)],
                                capture_output=True, text=True, timeout=5)
        names = listed.stdout.splitlines()
        case.check(listed.returncode == 0 and all(name in names for name in ("layout_80x24", "layout_120x30", "layout_160x45"))
                   and not unused.exists(), "listing exposes the supported terminal sizes without launching or logging a daemon", listed.stdout + listed.stderr)


def main():
    summary_fixture()
    command_logging()
    resource_thresholds()
    umbrella_flags()
    native_tui_wiring()
    return suite_status("ci_gates")


if __name__ == "__main__":
    sys.exit(main())
