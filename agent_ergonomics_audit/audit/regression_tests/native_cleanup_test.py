#!/usr/bin/env python3
"""Real-process positive/negative controls for native fixture cleanup."""
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import fbtlog
import native
from process_cleanup import Process, ProcessRun, finish_cleanup, identity


class NativeCleanup(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fbt-cleanup-control-", dir="/tmp")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.run = ProcessRun()
        self.run.register(self.root)

    def child(self, root, *, ignore_term=False, cwd=None):
        ready = root / "ready"
        code = ("import os,signal,time; from pathlib import Path; " +
                (f"signal.signal(signal.SIGTERM, lambda *_: Path({str(root / 'term-received')!r}).touch()); " if ignore_term else "") +
                f"Path({str(ready)!r}).touch(); time.sleep(600)")
        child = subprocess.Popen([sys.executable, "-c", code],
                                 env=dict(os.environ, HOME=str(root / "home")),
                                 cwd=cwd, start_new_session=True)
        def reap():
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)
        self.addCleanup(reap)
        end = time.monotonic() + 10
        while not ready.exists() and time.monotonic() < end:
            time.sleep(.01)
        self.assertTrue(ready.exists(), "child must be running before cleanup is exercised")
        return child

    def test_cleanup_kills_detached_term_resistant_child(self):
        child = self.child(self.root, ignore_term=True)
        self.assertIn(child.pid, [p.pid for p in self.run.survivors()])
        self.run.cleanup()
        self.assertEqual(child.wait(timeout=5), -signal.SIGKILL)
        self.assertFalse(self.run.survivors())

    def test_expired_cleanup_budget_kills_without_starting_another_term_wait(self):
        child = self.child(self.root, ignore_term=True)
        self.run.cleanup(deadline=time.monotonic()-1)
        self.assertEqual(child.wait(timeout=5), -signal.SIGKILL)
        self.assertFalse((self.root / "term-received").exists())

    def test_other_run_and_similar_path_prefix_are_untouched(self):
        with tempfile.TemporaryDirectory(prefix=self.root.name + "-", dir="/tmp") as other:
            other_child = self.child(Path(other))
            owned = self.child(self.root)
            self.run.cleanup()
            self.assertIsNotNone(owned.poll())
            self.assertIsNone(other_child.poll())
            other_child.terminate()
            other_child.wait(timeout=5)

    def test_deleted_fixture_directory_still_identifies_its_process(self):
        child = self.child(self.root, cwd=self.root)
        self.temporary.cleanup()
        self.assertIn(child.pid, [p.pid for p in self.run.survivors()])
        self.run.cleanup()
        self.assertIsNotNone(child.poll())

    def test_cwd_identifies_child_without_fixture_environment(self):
        child = subprocess.Popen(["sleep", "600"], cwd=self.root, env={}, start_new_session=True)
        self.addCleanup(lambda: child.wait(timeout=5))
        self.addCleanup(lambda: child.kill() if child.poll() is None else None)
        self.assertIn(child.pid, [p.pid for p in self.run.survivors()])
        self.run.cleanup()
        self.assertIsNotNone(child.poll())

    def test_stale_pid_identity_cannot_signal_a_process(self):
        child = self.child(self.root)
        stale = Process(child.pid, str(int(identity(child.pid)[0]) - 1), "control", str(self.root))
        self.run.send(stale, signal.SIGKILL)
        self.assertIsNone(child.poll())

    def test_descendant_with_replaced_environment_is_also_owned(self):
        pid_file = self.root / "grandchild.pid"
        grandchild_code = ("import os,signal,time; from pathlib import Path; "
                           "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                           f"Path({str(pid_file)!r}).write_text(str(os.getpid())); time.sleep(600)")
        code = ("import subprocess,time; "
                f"child=subprocess.Popen([{sys.executable!r},'-c',{grandchild_code!r}], env={{}}, cwd='/'); "
                "time.sleep(600)")
        parent = subprocess.Popen([sys.executable, "-c", code],
                                  env=dict(os.environ, HOME=str(self.root / "home")), start_new_session=True)
        self.addCleanup(lambda: parent.wait(timeout=5))
        self.addCleanup(self.run.cleanup)
        end = time.monotonic() + 10
        while not pid_file.exists() and time.monotonic() < end:
            time.sleep(.01)
        self.assertTrue(pid_file.exists())
        grandchild = int(pid_file.read_text())
        self.assertIn(grandchild, [process.pid for process in self.run.survivors()])
        self.run.cleanup()
        self.assertIsNotNone(parent.poll())
        try:
            self.assertEqual(identity(grandchild)[1], "Z")
        except FileNotFoundError:
            pass

    def test_daemon_requiring_kill_still_fails_after_its_case_closed(self):
        child = self.child(self.root, ignore_term=True)
        def closed_case():
            self.fail("cleanup cannot append a failure to a case that has already closed")
        harness = native.NativeHarness(closed_case, "unused", {})
        harness.serve = child
        with self.assertRaisesRegex(TimeoutError, "required SIGKILL"):
            harness.close_serve()
        self.assertEqual(child.poll(), -signal.SIGKILL)

    def test_broad_or_unregistered_roots_are_rejected(self):
        for root in (Path("/"), Path("/tmp"), Path.home()):
            with self.subTest(root=root), self.assertRaises(ValueError):
                self.run.register(root)
        with self.assertRaises(ValueError):
            self.run.cleanup([self.root.parent])

    def test_artifact_error_and_cleanup_timeout_do_not_skip_later_cleanup(self):
        child = self.child(self.root)
        visited = []
        def broken_artifact():
            visited.append("artifact")
            raise OSError("injected artifact failure")
        def timed_out_cleanup():
            visited.append("tmux")
            child.wait(timeout=.01)
        def final_cleanup():
            visited.append("processes")
            self.run.cleanup()
        with self.assertRaises(ExceptionGroup) as error:
            finish_cleanup(broken_artifact, timed_out_cleanup, final_cleanup)
        self.assertEqual(visited, ["artifact", "tmux", "processes"])
        self.assertEqual(len(error.exception.exceptions), 2)
        self.assertIsNotNone(child.poll())

    def test_attached_tui_constructor_cleans_up_before_caller_can_register_it(self):
        for failure in (RuntimeError, KeyboardInterrupt):
            with self.subTest(failure=failure.__name__), native.daemon.Sandbox() as sandbox:
                run = ProcessRun()
                run.register(sandbox.root)
                started = []
                class FailedCase:
                    def check(self, *_):
                        started.extend(run.survivors())
                        raise failure("injected failure during TUI construction")
                # This is also the tui_smoke construction path: no native.main
                # audit is active, and no ui object is returned to the caller.
                with self.assertRaises(failure):
                    native.AttachedTui(FailedCase(), "unused", sandbox, None, session="fbt-constructor-control")
                self.assertTrue(any("tmux" in process.name for process in started), started)
                self.assertFalse(run.survivors())

    def test_suite_audit_fails_survivors_even_after_early_return_or_exception(self):
        for outcome in ("return", "exception", "termination", "interrupt"):
            with self.subTest(outcome=outcome):
                children = []
                def cases():
                    with native.private_directory(prefix="fbt-audit-control-", dir="/tmp") as root:
                        children.append(self.child(Path(root)))
                        if outcome == "exception":
                            raise subprocess.TimeoutExpired("injected case", .01)
                        if outcome == "termination":
                            os.kill(os.getpid(), signal.SIGTERM)
                        if outcome == "interrupt":
                            os.kill(os.getpid(), signal.SIGINT)
                        return 0
                logs = self.root / outcome
                with patch.object(native, "run_cases", cases), patch.object(fbtlog, "LOG_ROOT", logs), \
                        patch.object(fbtlog, "_suites", {}), contextlib.redirect_stdout(io.StringIO()), \
                        contextlib.redirect_stderr(io.StringIO()):
                    if outcome == "return":
                        self.assertEqual(native.main(), 1)
                    else:
                        failure = {"exception": subprocess.TimeoutExpired, "termination": SystemExit,
                                   "interrupt": KeyboardInterrupt}[outcome]
                        with self.assertRaises(failure):
                            native.main()
                self.assertIsNotNone(children[0].poll())
                rows = [json.loads(line) for line in (logs / "native/process_cleanup.jsonl").read_text().splitlines()]
                self.assertFalse(rows[0]["ok"], "a survivor must fail even when emergency cleanup succeeds")
                self.assertTrue(rows[1]["ok"])
                self.assertFalse(rows[-1]["ok"])

    def test_interrupt_during_final_audit_cannot_skip_emergency_cleanup(self):
        for phase in ("wait", "cleanup"):
            with self.subTest(phase=phase):
                children, sent = [], []
                original = getattr(ProcessRun, phase)
                def interrupt(run, *args, **kwargs):
                    if not sent:
                        sent.append(True)
                        os.kill(os.getpid(), signal.SIGINT)
                    return original(run, *args, **kwargs)
                def cases():
                    with native.private_directory(prefix="fbt-audit-interrupt-", dir="/tmp") as root:
                        children.append(self.child(Path(root)))
                    return 0
                previous = signal.getsignal(signal.SIGINT)
                with patch.object(native, "run_cases", cases), patch.object(ProcessRun, phase, interrupt), \
                        patch.object(fbtlog, "LOG_ROOT", self.root / phase), patch.object(fbtlog, "_suites", {}), \
                        contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(native.main(), 128 + signal.SIGINT)
                self.assertEqual(signal.getsignal(signal.SIGINT), previous)
                self.assertIsNone(native._cleanup_deadline.get())
                self.assertIsNotNone(children[0].poll())


@unittest.skipUnless(os.environ.get("FBT_CLEANUP_BINARY"), "set FBT_CLEANUP_BINARY for the real native fixture")
class NativeFixtureCleanup(unittest.TestCase):
    """Inject failures only after the real daemon and TUI have started."""
    def exercise(self, mode):
        logs = Path(os.environ.get("FBT_CLEANUP_LOG_ROOT", fbtlog.LOG_ROOT / "cleanup-controls")) / mode
        logs.mkdir(parents=True, exist_ok=True)
        runner = r'''
import json, os, signal, subprocess, sys, time
from pathlib import Path
from unittest.mock import patch
import native

mode, binary, logs = sys.argv[1:]
logs = Path(logs).resolve()
wait = native.NativeHarness.wait_for
run = subprocess.run
write_bytes = Path.write_bytes
observed = []
interrupted_at = None
def observe_started():
    owned = native._process_run.get().survivors()
    roles = set()
    for process in owned:
        try:
            arguments = Path(f"/proc/{process.pid}/cmdline").read_bytes().split(b"\0")
            roles.update(role for role in ("serve", "tui") if role.encode() in arguments)
        except FileNotFoundError:
            pass
    (logs / "started.json").write_text(json.dumps(dict(processes=[vars(p) for p in owned], roles=sorted(roles))))
    observed.extend(owned)

def wait_then_fail(self, check, label, seconds=10):
    global interrupted_at
    result = wait(self, check, label, seconds)
    if mode == "daemon_startup_interrupt" and label == "disposable daemon listener opens":
        assert result
        observe_started()
        raise KeyboardInterrupt("injected interrupt during daemon startup")
    if label == "actual terminal startup enables unpressed pointer motion":
        assert result, "fault must follow successful real TUI startup"
        if mode == "group_interrupt_slow_daemon":
            # A stubborn owned process plus a timed-out tmux cleanup must not
            # consume the whole grace before detached children are collected.
            ready = Path(self.env["HOME"]).parent / "stubborn.ready"
            code = ("import signal,time; from pathlib import Path; "
                    "signal.signal(signal.SIGINT,signal.SIG_IGN); signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                    f"Path({str(ready)!r}).touch(); time.sleep(600)")
            self.serve = subprocess.Popen([sys.executable,"-c",code], env=self.env, start_new_session=True)
            end = time.monotonic()+10
            while not ready.exists() and time.monotonic()<end: time.sleep(.01)
            assert ready.exists()
        observe_started()
        if mode == "termination":
            os.kill(os.getpid(), signal.SIGTERM)
        if mode.startswith("group_interrupt"):
            interrupted_at = time.monotonic()
            os.killpg(os.getpgrp(), signal.SIGINT)
        if mode not in ("normal", "default_log_root"):
            raise subprocess.TimeoutExpired("injected case timeout after real startup", 10)
    return result
def broken_tmux(argv, *args, **kwargs):
    if mode == "teardown" and argv[0] == "tmux" and "kill-server" in argv:
        raise subprocess.TimeoutExpired("injected tmux kill-server timeout", kwargs.get("timeout"))
    if mode in ("group_interrupt_cleanup_timeout", "group_interrupt_slow_daemon") and argv[0] == "tmux" and "kill-server" in argv:
        # Consume the real cleanup timeout, leaving the detached tmux server
        # alive for the final private-process sweep to find.
        return run(["sleep", "600"], env=kwargs["env"], timeout=kwargs["timeout"])
    result = run(argv, *args, **kwargs)
    if mode in ("startup_error", "startup_interrupt") and argv[0] == "tmux" and "new-session" in argv:
        observe_started()
        if mode == "startup_interrupt":
            raise KeyboardInterrupt("injected interrupt during tmux startup")
        raise RuntimeError("injected exception during tmux startup")
    return result
def broken_artifact(path, data):
    if mode == "teardown" and path.name == "no_fake_seed_counts.terminal.typescript":
        raise OSError("injected terminal artifact failure")
    return write_bytes(path, data)
sys.argv = ["native.py", binary, "--case", "no_fake_seed_counts" if mode == "teardown" else "dec_mouse_mode",
            "--log-root", str(logs)]
if mode == "default_log_root":
    # Use the invocation's existing default, without the optional CLI flag.
    native.fbtlog.LOG_ROOT = logs
    sys.argv = ["native.py", binary, "--case", "no_fake_seed_counts"]
if mode.startswith("group_interrupt"):
    # Shell-launched/background wrappers may have inherited SIGINT ignored.
    signal.signal(signal.SIGINT, signal.SIG_IGN)
try:
    with patch.object(native.NativeHarness, "wait_for", wait_then_fail), \
            patch.object(subprocess, "run", broken_tmux), patch.object(Path, "write_bytes", broken_artifact):
        status = native.main()
finally:
    # Independent observation after native's guard has completed, using the
    # exact identities captured while the real fixture was still running.
    from process_cleanup import identity
    alive = []
    for process in observed:
        try:
            started, state = identity(process.pid)
            if started == process.started and state != "Z":
                alive.append(vars(process))
        except (FileNotFoundError, ProcessLookupError):
            pass
    (logs / "survivors.json").write_text(json.dumps(alive))
    if interrupted_at is not None:
        (logs / "cleanup-time.json").write_text(json.dumps(dict(seconds=time.monotonic()-interrupted_at)))
sys.exit(status)
'''
        with (logs / "runner.log").open("w") as output:
            result = subprocess.run([sys.executable, "-c", runner, mode,
                                     str(Path(os.environ["FBT_CLEANUP_BINARY"]).resolve()), str(logs.resolve())],
                                    cwd=Path(__file__).parent, stdout=output, stderr=subprocess.STDOUT,
                                    start_new_session=True, timeout=90)
        self.assertEqual(result.returncode == 0, mode in ("normal", "default_log_root"), (logs / "runner.log").read_text()[-4000:])
        started = json.loads((logs / "started.json").read_text())
        if mode != "daemon_startup_interrupt":
            self.assertTrue(any("tmux" in process["name"] for process in started["processes"]), started)
        roles = {"serve"} if mode in ("startup_error", "startup_interrupt", "daemon_startup_interrupt") else {"serve", "tui"}
        self.assertTrue(roles <= set(started["roles"]), started)
        self.assertEqual(json.loads((logs / "survivors.json").read_text()), [])
        rows = [json.loads(line) for line in (logs / "native/process_cleanup.jsonl").read_text().splitlines()]
        self.assertTrue(rows[-1]["ok"], rows)
        if mode == "teardown":
            diagnostics = (logs / "runner.log").read_text()
            self.assertIn("injected terminal artifact failure", diagnostics)
            self.assertIn("injected tmux kill-server timeout", diagnostics)
        if mode.startswith("group_interrupt"):
            self.assertLess(json.loads((logs / "cleanup-time.json").read_text())["seconds"], 10,
                            "cleanup must finish before the gate's TERM/KILL escalation")

    def test_normal(self):
        self.exercise("normal")

    def test_case_timeout(self):
        self.exercise("timeout")

    def test_artifact_error_and_tmux_timeout(self):
        self.exercise("teardown")

    def test_terminated_runner(self):
        self.exercise("termination")

    def test_daemon_startup_interrupt(self):
        self.exercise("daemon_startup_interrupt")

    def test_tmux_startup_interrupt(self):
        self.exercise("startup_interrupt")

    def test_tmux_startup_exception(self):
        self.exercise("startup_error")

    def test_process_group_interrupt(self):
        self.exercise("group_interrupt")

    def test_process_group_interrupt_with_timed_out_cleanup(self):
        self.exercise("group_interrupt_cleanup_timeout")

    def test_process_group_interrupt_with_stubborn_daemon_and_tmux_timeout(self):
        self.exercise("group_interrupt_slow_daemon")

    def test_artifact_cleanup_without_log_root_flag(self):
        self.exercise("default_log_root")


if __name__ == "__main__":
    unittest.main()
