#!/usr/bin/env python3
"""Measure isolated empty-app startup, status, idle resources and shutdown.

No user config, services or downloads are touched. This measures the actual
TUI, including its normal discovery setup. It is not a throughput benchmark or
an allocation profiler. Run with a release-perf binary after behavior tests.
Gate/baseline modes use a fixed local ratings cache and a private loopback download.
"""
import argparse
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import pty
import signal
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time

from fbtlog import Case, suite_status


def reference(args):
    cpu = next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")), platform.machine())
    return dict(host=platform.node(), machine=platform.machine(), cpu=cpu,
                idle_seconds=args.idle_seconds, active_mib=32, active_mib_s=4,
                ratings_fixture="fresh empty TSV; active title lookup negatively cached",
                binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())


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


def distribution(values):
    ordered = sorted(values)
    def percentile(percent):
        return ordered[min(len(ordered) - 1, math.ceil(percent * len(ordered)) - 1)]
    return {"samples": len(values), "p50": percentile(.50), "p95": percentile(.95),
            "p99": percentile(.99), "max": max(values), "raw": values}


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


def measure_idle(args, case):
    binary = args.binary.resolve()
    starts, stops, queries = [], [], []
    resources = None
    samples = []
    for run in range(args.runs):
        step = case.step(f"measure idle run {run + 1}", "isolated native TUI starts, answers status and drains cleanly")
        with tempfile.TemporaryDirectory(prefix="fbittorrent-resources-") as temporary:
            root = Path(temporary)
            env = dict(os.environ, TERM="xterm-256color")
            for kind in ("CONFIG", "DATA", "RUNTIME", "CACHE", "STATE"):
                key = f"XDG_{kind}_HOME" if kind != "RUNTIME" else "XDG_RUNTIME_DIR"
                env[key] = str(root / kind.lower())
                Path(env[key]).mkdir(mode=0o700)
            config_dir = Path(env["XDG_CONFIG_HOME"]) / "fbittorrent"
            config_dir.mkdir()
            # TUI startup refreshes a missing IMDb dataset even with no torrents.
            # A fresh local header keeps gate measurements entirely fixture based.
            if args.baseline or args.write_baseline:
                cache = Path(env["XDG_CACHE_HOME"]) / "fbittorrent"
                cache.mkdir()
                (cache / "title.ratings.tsv").write_text("tconst\taverageRating\tnumVotes\n")
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                port = probe.getsockname()[1]
            (config_dir / "config.toml").write_text(
                f'engine = "native"\nautostart = false\nnotify = false\n'
                f'url = "http://127.0.0.1:9"\nbackend_unit = "fbittorrent-fixture-never.service"\n'
                f'peer_port = {port}\nsave_path = {json.dumps(str(root / "downloads"))}\n'
                + (f'network_interface = {json.dumps(args.vpn)}\n' if args.vpn else
                   'network_interface = "lo"\n' if args.baseline or args.write_baseline else ''))
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
            error_path = case.suite.stderr_path
            with error_path.open("ab") as errors:
                began = time.monotonic()
                process = subprocess.Popen([str(binary), "tui"], env=env, stdin=slave,
                                           stdout=slave, stderr=errors, start_new_session=True,
                                           preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0))
                os.close(slave)
                def drain():
                    try:
                        while os.read(master, 65536):
                            pass
                    except OSError:
                        pass
                reader = threading.Thread(target=drain, daemon=True)
                reader.start()
                try:
                    endpoint = Path(env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
                    while not endpoint.exists():
                        if process.poll() is not None or time.monotonic() - began > 15:
                            raise RuntimeError("app did not open: " + error_path.read_text())
                        time.sleep(.002)
                    starts.append((time.monotonic() - began) * 1000)
                    # Capture status command latency including process startup.
                    for _ in range(20 if run == args.runs - 1 else 1):
                        queried = time.monotonic()
                        status = subprocess.run([str(binary), "status", "--json"], env=env,
                                                capture_output=True, timeout=10, check=True)
                        parsed = json.loads(status.stdout)
                        assert parsed.get("ok") and parsed.get("total") == 0, parsed
                        assert parsed.get("engine") == "native" and parsed.get("app_open") is True, parsed
                        queries.append((time.monotonic() - queried) * 1000)
                    if run == args.runs - 1 or args.baseline or args.write_baseline:
                        # Settle launch-only discovery before measuring the idle window.
                        time.sleep(2)
                        before = process_stats(process.pid)
                        measured = time.monotonic()
                        time.sleep(args.idle_seconds)
                        after = process_stats(process.pid)
                        elapsed = time.monotonic() - measured
                        resources = {"before": before, "after": after, "elapsed_seconds": elapsed,
                                     "cpu_percent_one_core": 100 * (after["cpu_ticks"] - before["cpu_ticks"]) /
                                     os.sysconf("SC_CLK_TCK") / elapsed}
                        samples.append(dict(cpu_s=(after["cpu_ticks"]-before["cpu_ticks"]) /
                                            os.sysconf("SC_CLK_TCK"), rss_kb=after["VmRSS"]))
                    stopping = time.monotonic()
                    os.write(master, b"q")
                    process.wait(timeout=6)
                    stops.append((time.monotonic() - stopping) * 1000)
                    assert process.returncode == 0 and not endpoint.exists()
                    assert not error_path.read_text(), error_path.read_text()
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    reader.join(timeout=1)
                    os.close(master)
                    assert not reader.is_alive(), "isolated terminal reader did not finish"
            print(f"resource run {run + 1}/{args.runs} passed", flush=True)
        step.ok(f"Startup {starts[-1]:.1f} ms; shutdown {stops[-1]:.1f} ms; sample {samples[-1] if samples else 'latency only'}.")
    result = {"scenario": "empty native TUI, 120x30, fresh isolated state, " + (f"required interface {args.vpn}" if args.vpn else "normal discovery"),
              "network_interface": args.vpn,
              "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "platform": platform.platform(), "startup_to_engine_socket_ms": distribution(starts),
              "shutdown_ms": distribution(stops), "status_cli_ms": distribution(queries),
              "idle": resources, "tail_limit": "p99 equals maximum with fewer than 100 samples",
              "limits": "Same host, warm executable cache. No allocation/stack sampler installed; procfs measures total process and thread waits.",
              "idle_samples": samples}
    if args.baseline or args.write_baseline:
        result["scenario"] = "isolated loopback-bound native TUI; idle plus private 32 MiB download at 4 MiB/s"
    return result


def measure_active(args, case):
    # Reuse the existing real BEP 3 peer and verified-payload benchmark. It binds
    # both tracker and peers to loopback and never starts qBittorrent here.
    from engine_benchmark import App, Seed, verify_completed_receipt
    samples = []
    for run in range(args.runs):
        step = case.step(f"measure active run {run + 1}", "private download has exact bytes, measured CPU/RSS and clean shutdown")
        with tempfile.TemporaryDirectory(prefix="fbt-resource-active-") as temporary:
            root = Path(temporary)
            ratings = root / "ratings.tsv"
            ratings.write_text("tconst\taverageRating\tnumVotes\n")
            app_root = root / "app"
            app_root.mkdir()
            app = App(app_root, "native", args, ratings, binary=args.binary)
            state = app_root / "state"
            state.mkdir(mode=0o700)
            app.env["XDG_STATE_HOME"] = str(state)
            seed = None
            try:
                seed = Seed(root, 32, 4)
                seed.label = f"resources-{run + 1}"
                app.start()
                download = app.transfer(seed)
                app.close()
                receipt = verify_completed_receipt(seed, seed.label, download)
                samples.append(dict(cpu_s=download["cpu_seconds"], rss_kb=download["peak_rss_mib"] * 1024,
                                    download=download, seed_receipt=receipt))
                step.ok(f"CPU {download['cpu_seconds']} s; peak RSS {download['peak_rss_mib']} MiB; "
                        f"SHA256 {download['verified_sha256']}; seed sent exactly {receipt['bytes_sent']} bytes.")
            finally:
                app.cleanup()
                errors = (app_root / "tui-stderr").read_bytes()
                with case.suite.stderr_path.open("ab") as output:
                    output.write(errors)
                if seed is not None:
                    seed.close()
            assert not errors, errors.decode(errors="replace")
    return samples


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--daemon", action="store_true", help="measure the isolated daemon subscription bus")
    parser.add_argument("--matrix", action="store_true", help="daemon: 10/100 torrents x 0/1/8 subscribers")
    parser.add_argument("--torrents", type=int, default=10)
    parser.add_argument("--subscribers", type=int, default=1)
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--runs", type=int, help="default 3 for a gate/baseline, 20 for latency measurement")
    parser.add_argument("--idle-seconds", type=float, default=10)
    parser.add_argument("--vpn", help="require this interface in a non-gate measurement")
    parser.add_argument("--output", type=Path, required=True)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--baseline", type=Path, help="compare idle/active CPU and RSS against the reference baseline")
    group.add_argument("--write-baseline", type=Path, help="write a first baseline from exactly three measured runs")
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.runs = args.runs if args.runs is not None else 3 if args.daemon or args.baseline or args.write_baseline else 20
    if args.daemon:
        from resources_daemon import main as daemon_main
        return daemon_main(args)
    if args.runs < 1 or args.idle_seconds <= 0 or (args.write_baseline and args.runs != 3):
        parser.error("positive runs/idle window required; a baseline requires exactly three runs")
    if args.vpn and (args.baseline or args.write_baseline):
        parser.error("resource gates use the fixed loopback fixture; omit --vpn")
    with Case("resources", "baseline" if args.write_baseline else "compare" if args.baseline else "measure") as case:
        baseline = json.loads(args.baseline.read_text()) if args.baseline else None
        if baseline is not None:
            compare(baseline, baseline["configs"], reference(args))
        result = measure_idle(args, case)
        if args.baseline or args.write_baseline:
            active = measure_active(args, case)
            samples = {mode: {metric: [row[metric] for row in rows] for metric in ("cpu_s", "rss_kb")}
                       for mode, rows in (("idle", result["idle_samples"]), ("active", active))}
            result.update(configs=samples, reference=reference(args), active_samples=active)
            if args.baseline:
                comparisons = compare(baseline, samples, result["reference"])
                result["comparisons"] = comparisons
                for comparison in comparisons:
                    case.check(comparison["ok"], f"{comparison['mode']} {comparison['metric']} stays within baseline margin",
                               json.dumps(comparison))
            else:
                args.write_baseline.parent.mkdir(parents=True, exist_ok=True)
                args.write_baseline.write_text(json.dumps(dict(version=1, reference=result["reference"], configs=samples), indent=2) + "\n")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        case.step("retain measurements", "measurement JSON is available for review").ok(
            str(args.output), artifacts=[args.output])
    return suite_status("resources")


if __name__ == "__main__":
    sys.exit(main())
