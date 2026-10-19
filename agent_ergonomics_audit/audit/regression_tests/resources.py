#!/usr/bin/env python3
"""Measure the current daemon's CPU/RSS against the P1-08 bus baseline.

The default is the complete 10/100 torrent × 0/1/8 subscriber matrix. Fixtures
own their HOME/XDG paths, loopback payloads and daemons. Closing a TUI detaches;
the obsolete in-process TUI/engine lifetime is not a resource fixture.
"""
import argparse
import math
import os
from pathlib import Path
import statistics


def compare(baseline, samples, current_reference):
    """Median baseline + max(10% of median, 3 * (maximum - minimum)).

    Compare every current observation, so a single regression cannot be hidden
    by averaging. Zero baselines stay strict; no arbitrary CPU allowance is added.
    """
    for key in ("host", "machine", "cpu", "idle_seconds", "active_mib", "active_mib_s", "ratings_fixture"):
        if baseline["reference"][key] != current_reference[key]:
            raise ValueError(f"baseline reference mismatch for {key}: remeasure deliberately on this host")
    results = []
    for mode in ("idle", "active"):
        for metric in ("cpu_s", "rss_kb"):
            values = baseline["configs"][mode][metric]
            current = samples[mode][metric]
            if len(values) != 3 or not current or any(not math.isfinite(v) or v < 0 for v in [*values, *current]):
                raise ValueError(f"{mode}/{metric} requires three finite nonnegative baseline values and current samples")
            center = statistics.median(values)
            margin = max(center * .10, 3 * (max(values) - min(values)))
            limit = center + margin
            results.append(dict(mode=mode, metric=metric, baseline=values, current=current,
                                median=center, margin=margin, limit=limit, ok=max(current) <= limit))
    return results



def process_stats(pid):
    base = Path(f"/proc/{pid}")
    fields = (base / "stat").read_text().rsplit(")", 1)[1].split()
    result = {"cpu_ticks": int(fields[11]) + int(fields[12])}
    for line in (base / "status").read_text().splitlines():
        name, _, value = line.partition(":")
        if name in ("VmRSS", "VmHWM", "Threads"):
            result[name] = int(value.strip().split()[0])
    for line in (base / "smaps_rollup").read_text().splitlines():
        if line.startswith("Pss:"):
            result["Pss"] = int(line.split()[1])
    result["wait_channels"] = {}
    for task in (base / "task").iterdir():
        try:
            name = (task / "comm").read_text().strip()
            channel = (task / "wchan").read_text().strip()
            result["wait_channels"].setdefault(name, []).append(channel)
        except (FileNotFoundError, ProcessLookupError):
            # A thread can exit between directory/comm/wchan observations.
            # These optional wait channels do not affect process metrics.
            pass
    return result



def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--daemon", action="store_true", help="measure one daemon configuration (or --matrix)")
    parser.add_argument("--matrix", action="store_true", help="10/100 torrents × 0/1/8 subscribers; default without --daemon")
    parser.add_argument("--torrents", type=int, default=10)
    parser.add_argument("--subscribers", type=int, default=1)
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--baseline", type=Path, help="compare against the recorded daemon workloads without changing limits")
    group.add_argument("--write-baseline", type=Path, help="deliberately record a complete three-run 60-second matrix")
    args = parser.parse_args(argv)
    args.binary = args.binary.resolve()
    args.matrix = args.matrix or not args.daemon
    from resources_daemon import main as daemon_main
    return daemon_main(args)


if __name__ == "__main__":
    raise SystemExit(main())
