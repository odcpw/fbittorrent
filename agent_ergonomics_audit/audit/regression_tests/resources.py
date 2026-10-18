#!/usr/bin/env python3
"""Measure isolated empty-app startup, status, idle resources and shutdown.

No user config, services or downloads are touched. This measures the actual
TUI, including its normal discovery setup. It is not a throughput benchmark or
an allocation profiler. Run with a release-perf binary after behavior tests.
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
import struct
import subprocess
import tempfile
import termios
import threading
import time


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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--runs", type=int, default=20)
    parser.add_argument("--idle-seconds", type=float, default=10)
    parser.add_argument("--vpn", help="require this interface in the isolated app config")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    starts, stops, queries = [], [], []
    resources = None
    for run in range(args.runs):
        with tempfile.TemporaryDirectory(prefix="fbittorrent-resources-") as temporary:
            root = Path(temporary)
            env = dict(os.environ, TERM="xterm-256color")
            for kind in ("CONFIG", "DATA", "RUNTIME", "CACHE"):
                key = f"XDG_{kind}_HOME" if kind != "RUNTIME" else "XDG_RUNTIME_DIR"
                env[key] = str(root / kind.lower())
                Path(env[key]).mkdir(mode=0o700)
            config_dir = Path(env["XDG_CONFIG_HOME"]) / "fbittorrent"
            config_dir.mkdir()
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                port = probe.getsockname()[1]
            (config_dir / "config.toml").write_text(
                f'engine = "native"\nautostart = false\nnotify = false\n'
                f'url = "http://127.0.0.1:9"\nbackend_unit = "fbittorrent-fixture-never.service"\n'
                f'peer_port = {port}\nsave_path = {json.dumps(str(root / "downloads"))}\n'
                + (f'network_interface = {json.dumps(args.vpn)}\n' if args.vpn else ''))
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
            error_path = root / "stderr.txt"
            with error_path.open("wb") as errors:
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
                    if run == args.runs - 1:
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
    result = {"scenario": "empty native TUI, 120x30, fresh isolated state, " + (f"required interface {args.vpn}" if args.vpn else "normal discovery"),
              "network_interface": args.vpn,
              "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "platform": platform.platform(), "startup_to_engine_socket_ms": distribution(starts),
              "shutdown_ms": distribution(stops), "status_cli_ms": distribution(queries),
              "idle": resources, "tail_limit": "p99 equals maximum with fewer than 100 samples",
              "limits": "Same host, warm executable cache. No allocation/stack sampler installed; procfs measures total process and thread waits."}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items()
                      if key not in ("idle",) and not isinstance(value, dict)}, indent=2))


if __name__ == "__main__":
    main()
