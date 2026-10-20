#!/usr/bin/env python3
"""Alternate two exact binaries on the same private loopback workloads."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
from types import SimpleNamespace
import uuid

from run import REPO, Run, ProcessRun

RELEASE_LOCK = Path("/tmp/fbittorrent-release.lock")


def comparison_plan(args):
    for phase, repeats in (("warmup", args.warmup), ("measured", args.runs)):
        for index in range(repeats):
            for case in args.cases:
                yield phase, index, case


def ancestor_holds_lock(path):
    """Reject an outer matrix-wide flock, which would defeat pair-level release."""
    pid, seen = os.getpid(), set()
    while pid > 1 and pid not in seen:
        seen.add(pid)
        try:
            root = Path("/proc") / str(pid)
            argv = (root / "cmdline").read_bytes().split(b"\0")
            if argv and Path(os.fsdecode(argv[0])).name == "flock" and os.fsencode(path) in argv:
                return True
            pid = int((root / "stat").read_text().rsplit(")", 1)[1].split()[1])
        except (FileNotFoundError, ProcessLookupError):
            break
    return False


def run_slice(command, budget, *, lock=RELEASE_LOCK, grace=10, unit=None):
    """The lock covers one pair, including fixture generation and teardown.

    The scope deadline also covers detached descendants: their new process
    sessions cannot escape the cgroup's SIGINT/cleanup/SIGKILL sequence.
    """
    unit = unit or f"fbt-bench-pair-{uuid.uuid4().hex}.scope"
    wrapped = ["flock", "-w", "7200", str(lock),
               "systemd-run", "--user", "--scope", "--unit", unit,
               "-p", "MemoryHigh=10G", "-p", "MemoryMax=14G",
               "-p", f"RuntimeMaxSec={budget}s", "-p", f"TimeoutStopSec={grace}s",
               "-p", "KillMode=control-group", "-p", "KillSignal=SIGINT",
               "-p", "FinalKillSignal=SIGKILL", "--quiet", "--", *command]
    child = subprocess.Popen(wrapped, start_new_session=True)
    try:
        return child.wait(timeout=7200 + budget + grace + 30)
    except BaseException:
        # Also handles interruption while queued, before the scope exists.
        try:
            os.killpg(child.pid, signal.SIGINT)
        except ProcessLookupError:
            pass
        try:
            subprocess.run(["systemctl", "--user", "stop", unit],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=grace + 2, check=False)
        finally:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait(timeout=2)
            # Close the queued-flock/start-scope race before returning.
            subprocess.run(["systemctl", "--user", "stop", unit],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=grace + 2, check=False)
        raise


def run_case(run, case, name):
    if case.startswith("download"):
        run.download(name, int(case.removeprefix("download")))
    elif case in ("magnet", "streaming"):
        run.download(name, 1, magnet=case == "magnet", streaming=case == "streaming")
    elif case == "recheck":
        run.recheck(name)
    else:
        active = case.startswith("active")
        run.many(name, int(case.removeprefix("active" if active else "idle")), active)


def pair_name(phase, index, case):
    return f"{'warmup-' if phase == 'warmup' else ''}{case}-{index+1}"


def invalidate_pair(output, name, **reason):
    failure = dict(pair=name, **reason)
    (output / f"{name}-failure.json").write_text(json.dumps(failure) + "\n")
    for variant in ("before", "after"):
        directory = output / variant / name
        if directory.exists():
            (directory / "failure.json").write_text(json.dumps(failure) + "\n")


def run_pair(variants, phase, index, case):
    owners = ProcessRun()
    try:
        order = ("before", "after") if index % 2 == 0 else ("after", "before")
        for variant in order:
            run_case(Run(variants[variant], owners), case, pair_name(phase, index, case))
    finally:
        survivors = owners.survivors()
        owners.cleanup()
        if survivors:
            raise RuntimeError(f"comparison leaked processes (cleaned): {survivors}")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("before", "after", "seed", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--before-source", required=True)
    parser.add_argument("--after-source", required=True)
    parser.add_argument("--disk-root", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--mib", type=int, default=1024)
    parser.add_argument("--preload", type=Path)
    parser.add_argument("--strace", type=Path)
    parser.add_argument("--profile", action="store_true")
    parser.add_argument("--cases", nargs="+", default=["download1", "download4", "download16"])
    parser.add_argument("--pair-budget-secs", type=float, default=600,
                        help="wall-clock limit per matched pair, including setup and teardown; then 10s cleanup")
    parser.add_argument("--_pair", nargs=3, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not math.isfinite(args.pair_budget_secs) or args.pair_budget_secs <= 0:
        parser.error("pair budget must be finite and positive")
    if args.runs < 1 or args.warmup < 0 or args.mib < 1:
        parser.error("runs/mib must be positive and warmup must be nonnegative")
    if any(not re.fullmatch(r"(?:download|idle|active)[1-9][0-9]*|magnet|streaming|recheck", c)
           for c in args.cases):
        parser.error("unknown workload")
    if not args._pair and ancestor_holds_lock(RELEASE_LOCK):
        parser.error("run compare.py directly: it acquires the release wrapper for each pair")
    if args._pair and not ancestor_holds_lock(RELEASE_LOCK):
        parser.error("internal pair worker requires the release wrapper")
    if not args._pair and args.output.exists() and any(args.output.iterdir()):
        parser.error("use a new output directory; measurements are never overwritten")
    for key in ("before", "after", "seed", "output", "disk_root", "preload", "strace"):
        if getattr(args, key) is not None:
            setattr(args, key, getattr(args, key).resolve(strict=key != "output"))
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    if not args._pair:
        args.output.mkdir(parents=True, exist_ok=True)
        worker = [sys.executable, str(Path(__file__).resolve())]
        for key, value in vars(args).items():
            if value is None or value is False:
                continue
            worker.append("--" + key.replace("_", "-"))
            if value is not True:
                worker.extend(map(str, value if isinstance(value, list) else [value]))
        for phase, index, case in comparison_plan(args):
            name = pair_name(phase, index, case)
            print(f"benchmark pair {name}: waiting for release lock", flush=True)
            try:
                code = run_slice([*worker, "--_pair", phase, str(index), case], args.pair_budget_secs)
            except BaseException as error:
                invalidate_pair(args.output, name, error=f"pair interrupted: {type(error).__name__}")
                raise
            if code:
                invalidate_pair(args.output, name, returncode=code,
                                error="pair failed or exceeded its wall-clock limit; exclude both halves")
                raise SystemExit(code if code > 0 else 128 - code)
        return
    seed = args.seed.resolve(strict=True)
    common = dict(seed=seed, seed_hash=digest(seed), disk_root=args.disk_root.resolve(strict=True),
                  mib=args.mib, timeout=300, seconds=20, profile=args.profile,
                  preload=args.preload.resolve(strict=True) if args.preload else None,
                  strace=args.strace.resolve(strict=True) if args.strace else None, probe_upload=False,
                  worktree_source=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip())
    variants = {}
    for variant in ("before", "after"):
        binary = getattr(args, variant).resolve(strict=True)
        output = args.output.resolve() / variant
        output.mkdir(parents=True, exist_ok=True)
        variants[variant] = SimpleNamespace(**common, binary=binary, binary_hash=digest(binary),
                                            binary_source=getattr(args, variant + "_source"), output=output)
    phase, index, case = args._pair
    if (phase, int(index), case) not in comparison_plan(args):
        parser.error("pair is outside the requested comparison")
    run_pair(variants, phase, int(index), case)


if __name__ == "__main__":
    main()
