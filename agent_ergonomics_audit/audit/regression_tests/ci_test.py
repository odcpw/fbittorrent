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
        case.check(result.stdout == (fixture / "expected.txt").read_text(),
                   "summary table exactly matches the two-suite fixture and slowest cases", result.stdout)


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
        root = case.suite.directory / "fixtures/umbrella"
        regression = root / "agent_ergonomics_audit/audit/regression_tests"
        regression.mkdir(parents=True)
        (root / "scripts").mkdir()
        (root / "plugin/tests").mkdir(parents=True)
        for name in ("all.sh", "ci.py", "fbtlog.py"):
            shutil.copyfile(HERE / name, regression / ("umbrella.sh" if name == "all.sh" else name))
        shutil.copyfile(REPO / "scripts/testlog-summary.py", root / "scripts/testlog-summary.py")
        umbrella = regression / "umbrella.sh"
        help_result = subprocess.run(["bash", str(umbrella), "--help"], capture_output=True, text=True, timeout=10)
        flags = ("QS", "VISUAL", "WIDGET", "VPN", "RESOURCES", "DAEMON_RESOURCES", "VERBOSE", "FORCE_FAIL")
        case.check(help_result.returncode == 0 and all("FBT_TEST_" + flag in help_result.stdout for flag in flags),
                   "--help documents every prescribed flag without starting a build", help_result.stdout)
        # These commands are dispatch-only fixtures; no compiler, shell plugin,
        # VPN interface or resource measurement is exercised by this unit test.
        def write_suite(suite):
            return ("import sys\nfrom pathlib import Path\n"
                    f"sys.path.insert(0, {str(regression)!r})\nfrom fbtlog import Case\n"
                    f"with Case({suite!r}, 'fixture') as case:\n"
                    " case.step('dispatch fixture', 'fixture selected').ok('fixture ran')\n")
        for name, suite in (("fbtlog_test.py", "fbtlog_test"), ("native.py", "native"),
                            ("ci_test.py", "ci_gates"), ("daemon.py", "daemon"), ("resources.py", "resources"),
                            ("tui_smoke.py","tui_smoke"),("tui_smoke_test.py","tui_smoke_control")):
            (regression / name).write_text(write_suite(suite))
        (regression / "tui_smoke.py").write_text(write_suite("tui_smoke")+"\n"+write_suite("tui_smoke_setup"))
        (regression / "resources.py").write_text(write_suite("resources").replace(
            "with Case('resources',", "with Case('daemon_budget' if '--daemon' in sys.argv else 'resources',"))
        (root / "plugin/tests/qs-log.py").write_text(write_suite("qs_log_test"))
        for name in ("check-gitignore.sh", "check-nova3-licences.sh"):
            p = root / "scripts" / name
            p.write_text("#!/bin/bash\n" + "exec python3 - <<'PY'\n" + write_suite("hygiene") + "PY\n")
            p.chmod(0o700)
        # all.sh also runs the widget lint (P0-11) and the online-declaration check
        # (P0-07) unconditionally; they are dispatch-only fixtures here as well.
        lint = root / "plugin/tests/lint.sh"
        lint.write_text("#!/bin/bash\n" + "exec python3 - <<'PY'\n" + write_suite("widget_lint") + "PY\n")
        lint.chmod(0o700)
        (root / "scripts/check-widget-online.py").write_text(write_suite("widget_online"))
        home = root / "fixture-home"
        shims = home / ".rch/shims"
        shims.mkdir(parents=True)
        for name, program in (("cargo", 'echo "$*" >> cargo-argv.log\nexit 0'), ("rustc", "echo 'host: x86_64-unknown-linux-gnu'")):
            p = shims / name
            p.write_text("#!/bin/bash\n" + program + "\n")
            p.chmod(0o700)
        for name in ("qs-store.sh", "qs-smoke.sh", "qs-visual.sh"):
            p = root / "plugin/tests" / name
            p.write_text("#!/bin/bash\necho " + name + " >> dispatch.log\n")
        (regression / "widget_keys.sh").write_text("#!/bin/bash\necho widget >> dispatch.log\n")
        (regression / "vpn.py").write_text("from pathlib import Path\nwith Path('dispatch.log').open('a') as f: f.write('vpn\\n')\n")
        env = dict(os.environ, HOME=str(home), FBT_TEST_LEGACY="0", FBT_TEST_FORCE_FAIL="",
                   **{"FBT_TEST_" + name: "1" for name in ("QS", "VISUAL", "WIDGET", "VPN", "RESOURCES", "DAEMON_RESOURCES")})
        result = subprocess.run(["bash", str(umbrella)], capture_output=True, text=True, timeout=30, env=env)
        (root / "stdout.log").write_text(result.stdout)
        (root / "stderr.log").write_text(result.stderr)
        case.check(result.returncode == 0 and set((root / "dispatch.log").read_text().splitlines()) ==
                   {"qs-store.sh", "qs-smoke.sh", "qs-visual.sh", "widget", "vpn"},
                   "every opted-in local suite dispatches to its named fixture", result.stdout + result.stderr)
        case.check(all(json.loads((root/"target/test-logs"/suite/"summary.json").read_text())["failed"]==0
                       for suite in ("tui_smoke","tui_smoke_setup","tui_smoke_control")),
                   "native terminal smoke and its failure control dispatch unconditionally with named summaries")
        case.check((root / "cargo-argv.log").read_text().count("--profile release-perf") == 1,
                   "the two resource gates share exactly one performance build")
        (root / "cargo-argv.log").write_text("")
        env["FBT_TEST_RESOURCES"] = "0"
        result = subprocess.run(["bash", str(umbrella)], capture_output=True, text=True, timeout=30, env=env)
        case.check(result.returncode == 0 and (root / "cargo-argv.log").read_text().count("--profile release-perf") == 1,
                   "the daemon-only resource gate builds its binary and runs independently", result.stdout + result.stderr)
        (root / "plugin/tests/qs-store.sh").unlink()
        result = subprocess.run(["bash", str(umbrella)], capture_output=True, text=True, timeout=30, env=env)
        rows = [json.loads(line) for line in (root / "target/test-logs/gate_qs_store/command.jsonl").read_text().splitlines()]
        case.check(result.returncode != 0 and not rows[-1]["ok"] and "not run" in result.stdout,
                   "missing opted-in scripts fail with retained JSONL and an unstarted-suite summary")


def native_tui_wiring():
    with Case("ci_gates","native_tui_wiring") as case:
        workflow=(REPO/".github/workflows/ci.yml").read_text().split("\n  native:",1)[1].split("\n  lint:",1)[0]
        umbrella=(HERE/"all.sh").read_text()
        for script in ("tui_smoke.py","tui_smoke_test.py"):
            command=f"python3 agent_ergonomics_audit/audit/regression_tests/{script} target/release/fbittorrent"
            case.check(command in workflow and command in umbrella,
                       f"{script} is in the native CI job and default gate",command)
        for suite in ("tui_smoke","tui_smoke_setup","tui_smoke_control"):
            case.check(workflow.count("--suite "+suite)>=2 and "--suite "+suite in umbrella,
                       f"{suite} is reset, summarized and included in CI artifacts")
        unused=case.suite.directory/"fixtures/list_only"
        listed=subprocess.run([sys.executable,str(HERE/"tui_smoke.py"),"/missing/fbittorrent","--list","--log-root",str(unused)],capture_output=True,text=True,timeout=5)
        case.check(listed.returncode==0 and listed.stdout.splitlines()==["layout_80x24","layout_120x30","layout_160x45","poster_overlay_cleanup","low_chroma_theme_markers","log_history","turtle_limits","peer_port"]and not unused.exists(),
                   "--list has stable case slugs and starts no daemon or log suite",listed.stdout+listed.stderr)


def main():
    summary_fixture()
    command_logging()
    resource_thresholds()
    umbrella_flags()
    native_tui_wiring()
    return suite_status("ci_gates")


if __name__ == "__main__":
    sys.exit(main())
