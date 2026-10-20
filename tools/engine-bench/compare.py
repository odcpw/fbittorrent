#!/usr/bin/env python3
"""Alternate two exact binaries on the same private loopback workloads."""
import argparse
import hashlib
from pathlib import Path
import signal
import subprocess
from types import SimpleNamespace

from run import REPO, Run, ProcessRun


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
    args = parser.parse_args()
    if args.output.exists() and any(args.output.iterdir()):
        parser.error("use a new output directory; measurements are never overwritten")
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
        output.mkdir(parents=True)
        variants[variant] = SimpleNamespace(**common, binary=binary, binary_hash=digest(binary),
                                            binary_source=getattr(args, variant + "_source"), output=output)
    owners = ProcessRun()
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        for phase, repeats in (("warmup-", args.warmup), ("", args.runs)):
            for index in range(repeats):
                order = ("before", "after") if index % 2 == 0 else ("after", "before")
                for case in args.cases:
                    for variant in order:
                        run = Run(variants[variant], owners)
                        name = f"{phase}{case}-{index+1}"
                        if case.startswith("download"):
                            run.download(name, int(case.removeprefix("download")))
                        elif case in ("magnet", "streaming"):
                            run.download(name, 1, magnet=case == "magnet", streaming=case == "streaming")
                        elif case == "recheck":
                            run.recheck(name)
                        elif case.startswith(("idle", "active")):
                            active = case.startswith("active")
                            run.many(name, int(case.removeprefix("active" if active else "idle")), active)
                        else:
                            raise ValueError(case)
    finally:
        survivors = owners.survivors()
        owners.cleanup()
        if survivors:
            raise RuntimeError(f"comparison leaked processes (cleaned): {survivors}")


if __name__ == "__main__":
    main()
