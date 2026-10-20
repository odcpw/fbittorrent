"""Scheduling/timeout controls; these do not benchmark an engine or hold its lock."""
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid

import compare


def wait_for(check):
    deadline = time.monotonic() + 20
    while not check():
        if time.monotonic() >= deadline:
            raise AssertionError("control did not reach its synchronization point")
        time.sleep(.01)


class SliceTests(unittest.TestCase):
    def test_controller_runs_each_pair_in_a_fresh_wrapped_worker(self):
        with tempfile.TemporaryDirectory(prefix="fbt-slice-control-") as temporary:
            root = Path(temporary)
            lock, output = root / "lock", root / "output"
            bootstrap = root / "worker.py"
            bootstrap.write_text(f"""
import sys
from pathlib import Path
sys.path.insert(0, {str(Path(compare.__file__).parent)!r})
import compare
compare.RELEASE_LOCK = Path({str(lock)!r})
class Control(compare.Run):
    def download(self, name, peers):
        directory = self.args.output / name
        directory.mkdir()
        (directory / 'entered').write_text(str(peers))
        with (self.args.output.parent / 'order').open('a') as stream:
            stream.write(name + ':' + self.args.output.name + '\\n')
compare.Run = Control
compare.main()
""")
            real_run_slice = compare.run_slice
            def wrapped_worker(command, budget):
                return real_run_slice([sys.executable, str(bootstrap), *command[2:]],
                                      budget, lock=lock)
            argv = ["compare.py", "--before", "/usr/bin/true", "--after", "/usr/bin/true",
                    "--seed", "/usr/bin/true", "--before-source", "control",
                    "--after-source", "candidate", "--output", str(output),
                    "--disk-root", temporary, "--runs", "2", "--warmup", "1",
                    "--cases", "download4"]
            with patch.object(sys, "argv", argv), patch.object(compare, "run_slice", wrapped_worker):
                compare.main()
            observed = (output / "order").read_text().splitlines()
            self.assertEqual(observed, [
                "warmup-download4-1:before", "warmup-download4-1:after",
                "download4-1:before", "download4-1:after",
                "download4-2:after", "download4-2:before",
            ])
            for variant in ("before", "after"):
                self.assertTrue((output / variant / "download4-2" / "entered").exists())

    def test_waiting_build_can_take_lock_between_pairs(self):
        with tempfile.TemporaryDirectory(prefix="fbt-slice-control-") as temporary:
            root = Path(temporary)
            lock, started, release, acquired = (root / n for n in ("lock", "started", "release", "acquired"))
            code = ("from pathlib import Path; import sys,time; "
                    "Path(sys.argv[1]).touch(); "
                    "exec('while not Path(sys.argv[2]).exists(): time.sleep(.01)')")
            results = []
            thread = threading.Thread(target=lambda: results.append(compare.run_slice(
                [sys.executable, "-c", code, str(started), str(release)], 30, lock=lock)))
            thread.start()
            contender = None
            try:
                wait_for(started.exists)
                contender = subprocess.Popen(["flock", "-w", "30", str(lock), sys.executable, "-c",
                                              "from pathlib import Path; import sys; Path(sys.argv[1]).touch()",
                                              str(acquired)])
                self.assertFalse(acquired.exists(), "first pair still owns the lock")
                release.touch()
                thread.join(20)
                self.assertFalse(thread.is_alive())
                self.assertEqual(results, [0])
                self.assertEqual(contender.wait(timeout=20), 0)
                self.assertTrue(acquired.exists(), "queued build entered before the next pair")
                self.assertEqual(compare.run_slice([sys.executable, "-c", "pass"], 10, lock=lock), 0)
            finally:
                release.touch()
                thread.join(20)
                if contender and contender.poll() is None:
                    contender.kill()
                    contender.wait()

    def test_failed_or_interrupted_worker_invalidates_both_halves_and_stops_matrix(self):
        for error in (17, KeyboardInterrupt()):
            with self.subTest(failure=type(error).__name__), tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary) / "output"
                calls = []
                def fail(*_):
                    calls.append("entered")
                    for variant in ("before", "after"):
                        directory = output / variant / "download4-1"
                        directory.mkdir(parents=True)
                        (directory / "teardown.json").write_text('{"drained":true,"survivors":0}')
                    if isinstance(error, BaseException):
                        raise error
                    return error
                argv = ["compare.py", "--before", "/usr/bin/true", "--after", "/usr/bin/true",
                        "--seed", "/usr/bin/true", "--before-source", "control",
                        "--after-source", "candidate", "--output", str(output),
                        "--disk-root", temporary, "--runs", "2", "--warmup", "0",
                        "--cases", "download4"]
                with patch.object(sys, "argv", argv), patch.object(compare, "run_slice", fail):
                    with self.assertRaises(KeyboardInterrupt if isinstance(error, BaseException) else SystemExit):
                        compare.main()
                self.assertEqual(calls, ["entered"])
                for variant in ("before", "after"):
                    self.assertTrue((output / variant / "download4-1" / "failure.json").exists())

    def test_scope_timeout_stops_detached_child_ignoring_interrupt(self):
        with tempfile.TemporaryDirectory(prefix="fbt-slice-control-") as temporary:
            root = Path(temporary)
            child_pid = root / "child.pid"
            unit = f"fbt-bench-control-{uuid.uuid4().hex}.scope"
            child = "import signal,time; signal.signal(signal.SIGINT,signal.SIG_IGN); time.sleep(60)"
            parent = ("import subprocess,signal,sys,time; from pathlib import Path; "
                      "signal.signal(signal.SIGINT,signal.SIG_IGN); "
                      "p=subprocess.Popen([sys.executable,'-c',sys.argv[2]],start_new_session=True); "
                      "Path(sys.argv[1]).write_text(str(p.pid)); time.sleep(60)")
            try:
                result = compare.run_slice([sys.executable, "-c", parent, str(child_pid), child],
                                           2, grace=1, lock=root / "lock", unit=unit)
                self.assertNotEqual(result, 0)
                self.assertTrue(child_pid.exists(), "timeout must exercise a started detached child")
                pid = int(child_pid.read_text())
                def gone():
                    try:
                        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] == "Z"
                    except FileNotFoundError:
                        return True
                wait_for(gone)
            finally:
                subprocess.run(["systemctl", "--user", "stop", unit], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)

    def test_outer_matrix_lock_is_detected_without_starting_work(self):
        with tempfile.TemporaryDirectory(prefix="fbt-slice-control-") as temporary:
            lock = Path(temporary) / "lock"
            code = ("import sys; from pathlib import Path; import compare; "
                    "assert compare.ancestor_holds_lock(Path(sys.argv[1]))")
            self.assertFalse(compare.ancestor_holds_lock(lock))
            subprocess.run(["flock", str(lock), sys.executable, "-c", code, str(lock)],
                           cwd=Path(compare.__file__).parent, check=True, timeout=20)

    def test_interrupted_controller_stops_its_scope(self):
        with tempfile.TemporaryDirectory(prefix="fbt-slice-control-") as temporary:
            root = Path(temporary)
            marker = root / "worker.pid"
            unit = f"fbt-bench-control-{uuid.uuid4().hex}.scope"
            worker = ("import os,signal,sys,time; from pathlib import Path; "
                      "signal.signal(signal.SIGINT,signal.SIG_IGN); "
                      "Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(60)")
            controller = ("import compare,sys; from pathlib import Path; "
                          "compare.run_slice([sys.executable,'-c',sys.argv[1],sys.argv[2]],"
                          "60,grace=1,lock=Path(sys.argv[3]),unit=sys.argv[4])")
            child = subprocess.Popen([sys.executable, "-c", controller, worker, str(marker),
                                      str(root / "lock"), unit], cwd=Path(compare.__file__).parent,
                                     start_new_session=True, stderr=subprocess.DEVNULL)
            try:
                wait_for(marker.exists)
                child.send_signal(signal.SIGINT)
                self.assertNotEqual(child.wait(timeout=20), 0)
                pid = int(marker.read_text())
                def gone():
                    try:
                        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] == "Z"
                    except FileNotFoundError:
                        return True
                wait_for(gone)
            finally:
                subprocess.run(["systemctl", "--user", "stop", unit], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
                if child.poll() is None:
                    child.kill()
                    child.wait()

    def test_pair_order_and_failure_do_not_run_later_work(self):
        order = []
        class Observed:
            def __init__(self, args, owners):
                self.variant = args.variant
            def download(self, name, peers):
                order.append((name, self.variant, peers))
        variants = {v: SimpleNamespace(variant=v) for v in ("before", "after")}
        args = SimpleNamespace(warmup=1, runs=2, cases=["download4", "download16"])
        with patch.object(compare, "Run", Observed):
            for phase, index, case in compare.comparison_plan(args):
                compare.run_pair(variants, phase, index, case)
        for index in range(0, len(order), 2):
            first, second = order[index:index+2]
            self.assertEqual(first[0], second[0])
            self.assertEqual(first[2], second[2])
            expected = ["after", "before"] if first[0].endswith("-2") else ["before", "after"]
            self.assertEqual([first[1], second[1]], expected)
        order.clear()
        def fail(*_):
            order.append("entered")
            raise RuntimeError("injected first-half failure")
        with patch.object(compare, "run_case", fail):
            with self.assertRaises(RuntimeError):
                compare.run_pair(variants, "measured", 0, "download4")
        self.assertEqual(order, ["entered"], "a failed half cannot proceed to the next half")


if __name__ == "__main__":
    unittest.main()
