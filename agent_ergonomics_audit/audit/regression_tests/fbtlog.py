"""Shared process-test logging (PLAN Appendix E.1).

Use ``with Case('suite', 'case') as case`` and close each ``case.step`` with
``ok(observed, artifacts=[])`` or ``fail(...)``. Failures do not interrupt the
case. At the end of a suite, exit with ``suite_status('suite')``.
"""
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[3]
LOG_ROOT = REPO / "target/test-logs"
_suites = {}


def timestamp():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def slug(value):
    if not re.fullmatch(r"[a-z0-9]+(?:_[a-z0-9]+)*", value):
        raise ValueError(f"invalid suite/case slug: {value!r}")
    return value


class Suite:
    def __init__(self, name):
        self.name = slug(name)
        self.directory = LOG_ROOT / name
        if self.directory.exists():
            shutil.rmtree(self.directory)
        self.directory.mkdir(parents=True)
        self.stderr_path = self.directory / "daemon.stderr.log"
        self.stderr_path.touch()
        self.results = []

    def summarize(self):
        failed = sum(not row["ok"] for row in self.results)
        summary = dict(suite=self.name, cases=len(self.results),
                       passed=len(self.results) - failed, failed=failed,
                       slowest=[dict(case=row["case"], ms=row["ms"]) for row in
                                sorted(self.results, key=lambda row: row["ms"], reverse=True)[:5]])
        (self.directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        return failed != 0


def suite_status(name):
    return int(_suites[name].summarize()) if name in _suites else 0


def get_suite(name):
    if name not in _suites:
        _suites[name] = Suite(name)
    return _suites[name]


class Step:
    def __init__(self, case, action, expect):
        self.case, self.action, self.expect = case, action, expect
        self.started = time.monotonic()
        self.closed = False
        case.pending.append(self)

    def ok(self, observed, artifacts=()):
        return self.close(True, observed, artifacts)

    def fail(self, observed, artifacts=()):
        return self.close(False, observed, artifacts)

    def close(self, ok, observed, artifacts=()):
        if self.closed:
            raise ValueError("step already closed")
        self.closed = True
        case = self.case
        case.pending.remove(self)
        case.steps += 1
        if case.steps == 1 and os.environ.get("FBT_TEST_FORCE_FAIL") == f"{case.suite.name}/{case.name}":
            ok, observed = False, f"Forced failure; actual observation: {observed}"
        if not ok:
            case.failed.append(case.steps)
        case.write(dict(step=case.steps, action=self.action, expect=self.expect,
                        observed=observed, ok=bool(ok),
                        ms=max(0, round((time.monotonic() - self.started) * 1000)),
                        artifacts=[str(path) for path in artifacts]))
        if not case.enabled and not ok:
            raise AssertionError(f"prerequisite failed: {self.expect}: {observed}")
        return bool(ok)


class Case:
    def __init__(self, suite, name, *, enabled=True):
        self.suite, self.name, self.enabled = get_suite(suite), slug(name), enabled
        self.path = self.suite.directory / f"{name}.jsonl"
        self.daemon_path = self.path.with_suffix(".daemon.log")
        self.steps, self.failed, self.pending = 0, [], []

    def __enter__(self):
        self.started = time.monotonic()
        self.stderr_offset = self.suite.stderr_path.stat().st_size
        if self.enabled:
            self.path.write_text("")
        return self

    def write(self, fields):
        if self.enabled:
            line = json.dumps(dict(ts=timestamp(), suite=self.suite.name, case=self.name, **fields))
            with self.path.open("a") as output:
                output.write(line + "\n")
            if os.environ.get("FBT_TEST_VERBOSE") == "1":
                print(line, file=sys.stderr)

    def step(self, action, expect):
        return Step(self, action, expect)

    def check(self, condition, expect, observed=None):
        step = self.step("check assertion", expect)
        return step.close(bool(condition), observed if observed is not None
                          else f"Condition evaluated to {condition!r}.")

    def popen(self, argv, **kwargs):
        if "stderr" in kwargs:
            raise ValueError("Case.popen owns stderr; capture stdout with stdout=PIPE")
        with self.suite.stderr_path.open("ab", buffering=0) as stderr:
            return subprocess.Popen(argv, stderr=stderr, **kwargs)

    def __exit__(self, kind, error, traceback):
        if error is not None:
            self.step("finish case", "case completes without an exception").fail(
                f"{kind.__name__}: {error}")
        for step in list(self.pending):
            step.fail("Step was not closed before the case ended.")
        if self.enabled:
            with self.suite.stderr_path.open("rb") as source:
                source.seek(self.stderr_offset)
                self.daemon_path.write_bytes(source.read())
            final = dict(final=True, ok=not self.failed, steps=self.steps, failed=self.failed,
                         ms=max(0, round((time.monotonic() - self.started) * 1000)))
            self.write(final)
            self.suite.results.append(dict(case=self.name, **final))
            self.suite.summarize()
            print(f"{'FAIL' if self.failed else 'ok'}: {self.suite.name}/{self.name}")
            if self.failed:
                for path in (self.path, self.daemon_path):
                    print(f"--- {path} (last 20 lines) ---", file=sys.stderr)
                    print("\n".join(path.read_text(errors="replace").splitlines()[-20:]), file=sys.stderr)
        return kind is not None and issubclass(kind, Exception) and self.enabled
