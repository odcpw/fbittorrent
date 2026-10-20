#!/usr/bin/env python3
"""Summarize uninstrumented paired rechecks; failed drains never qualify."""
import argparse
import json
from pathlib import Path
import random
from statistics import median


def spread(values):
    return dict(median=median(values), minimum=min(values), maximum=max(values), values=values)


def analyze(root, runs=10):
    variants = {}
    for name in ("before", "after"):
        rows = []
        for path in sorted((root / name).glob("recheck-*/result.json")):
            row = json.loads(path.read_text())
            drain = json.loads(path.with_name("teardown.json").read_text())
            assert drain["drained"] and drain["survivors"] == 0, path
            assert not row["instrumented"], path
            assert row["bytes"] == 4 * 1024**3, path
            rows.append((path.parent.name, row))
        assert rows, name
        variants[name] = dict(rows)
        assert set(variants[name]) == {f"recheck-{i}" for i in range(1, runs + 1)}, "every scheduled trial must qualify"
    assert variants["before"].keys() == variants["after"].keys()
    seeds = {row["seed_sha256"] for rows in variants.values() for row in rows.values()}
    assert len(seeds) == 1, "comparison must use one seed executable"
    payloads = {row["payload_sha256"] for rows in variants.values() for row in rows.values()}
    assert len(payloads) == 1, "independently verified payloads must match"
    result = dict(runs=len(variants["before"]), seed_sha256=seeds.pop(), payload_sha256=payloads.pop(), phases={})
    for phase in ("initial", "recheck"):
        summary = {}
        for name, rows in variants.items():
            summary[name] = {key: spread([row[phase][key] for row in rows.values()])
                             for key in ("seconds", "cpu_s", "mb_s", "rss_kb", "read_calls", "read_bytes")}
        pairs = [1 - variants["after"][name][phase]["seconds"] / before[phase]["seconds"]
                 for name, before in variants["before"].items()]
        rng = random.Random(42)
        boots = sorted(median(rng.choices(pairs, k=len(pairs))) for _ in range(10000))
        summary["paired_wall_reduction"] = dict(**spread(pairs), bootstrap_95=[boots[250], boots[9749]])
        result["phases"][phase] = summary
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--runs", type=int, default=10)
    args = parser.parse_args()
    print(json.dumps(analyze(args.root, args.runs), indent=2))
