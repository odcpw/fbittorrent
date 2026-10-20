#!/usr/bin/env python3
"""Private loopback engine benchmark; timings never include fixture generation.

Uses the regression harness's Sandbox, NDJSON client and process ownership scan.
Downloads live in a fresh disk directory; HOME and every XDG path stay private.
Normal measurements use /proc only. Optional preload instrumentation is a
separate diagnostic run, never a throughput comparison.
"""
import argparse
import base64
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import time
import traceback
from types import SimpleNamespace
import urllib.parse
import urllib.request

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "agent_ergonomics_audit/audit/regression_tests"))
from daemon import ApiConn, DaemonContract, private_environment
from behavior_checks import expected_daemon_stderr
from fixtures import Sandbox
from process_cleanup import ProcessRun
from perf_sample import Sampler

MIB = 1024**2


class Check:
    def step(self, *_):
        return self

    def close(self, ok, detail=""):
        if not ok:
            raise AssertionError(detail)
        return ok

    def fail(self, detail):
        raise AssertionError(detail)


def proc(pid):
    root = Path(f"/proc/{pid}")
    fields = (root / "stat").read_text().rsplit(")", 1)[1].split()
    status = dict(line.split(":", 1) for line in (root / "status").read_text().splitlines())
    io = dict(line.split(":", 1) for line in (root / "io").read_text().splitlines())
    return dict(cpu_s=(int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK"),
                rss_kb=int(status["VmRSS"].split()[0]), peak_rss_kb=int(status["VmHWM"].split()[0]),
                read_calls=int(io["syscr"]), write_calls=int(io["syscw"]),
                read_bytes=int(io["read_bytes"]), write_bytes=int(io["write_bytes"]))


def delta(before, after, seconds, size=0):
    result = {key: after[key] - before[key] for key in before if "rss" not in key}
    result.update(seconds=seconds, rss_kb=after["rss_kb"], peak_rss_kb=after["peak_rss_kb"])
    if size:
        result.update(mb_s=size / 1e6 / seconds, cpu_s_gb=result["cpu_s"] / (size / 1e9),
                      rw_syscalls_mb=(result["read_calls"] + result["write_calls"]) / (size / 1e6))
    return result


def wait(check, seconds=60):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.02)
    raise TimeoutError(f"condition did not hold in {seconds}s")


class Run:
    def __init__(self, args, owners=None):
        self.args = args
        self.owners = owners if owners is not None else ProcessRun()
        self.children = []
        self.handles = []

    def spawn(self, argv, env=None, log="process"):
        output = (self.output / (log + ".log")).open("wb")
        self.handles.append(output)
        child = subprocess.Popen(list(map(str, argv)), env=env or self.sandbox.env,
                                 cwd=self.sandbox.root, stdout=output, stderr=output, start_new_session=True)
        self.children.append(child)
        return child

    def tool(self, command, **options):
        argv = [str(self.args.seed), command]
        for key, value in options.items():
            argv += ["--" + key.replace("_", "-"), str(value)]
        output = subprocess.check_output(argv, env=self.sandbox.env, cwd=self.sandbox.root, timeout=120)
        return json.loads(output) if output else None

    @contextmanager
    def setup(self, name, count=1, size=None, peers=1, rate=0, streaming=False):
        self.output = self.args.output / name
        self.output.mkdir(parents=True, exist_ok=False)
        self.load_begin = os.getloadavg()
        self.pressure_begin = self.pressure()
        self.sandbox = Sandbox(directory=self.args.disk_root)
        self.owners.register(self.sandbox.root)
        # The filesystem is intentional. /tmp is tmpfs on the benchmark host.
        self.disk = self.sandbox.root
        self.metadata = self.disk / "metainfo"
        self.metadata.mkdir()
        cfg = Path(self.sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        text = cfg.read_text().replace(str(self.sandbox.root / "downloads"), str(self.sandbox.downloads))
        cfg.write_text(text + f"max_active_downloads = {count}\nupload = \"off\"\n")
        private_environment(self.sandbox)
        self.size = size or self.args.mib * MIB
        self.items = []
        try:
            self.prefix_options = {}
            if streaming:
                prefix = self.disk / "prefix.mkv"
                subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=256x144:rate=24",
                                "-t", "4", "-c:v", "mpeg4", "-q:v", "5", "-threads", "1", "-fflags", "+bitexact",
                                "-flags:v", "+bitexact", "-map_metadata", "-1", str(prefix)],
                               env=self.sandbox.env, cwd=self.disk, check=True, timeout=30)
                self.prefix_options["prefix"] = prefix
            # Start with no announce URL; inject the server's actually bound port
            # into the outer metainfo dict afterwards (the info hash is unchanged).
            for index in range(count):
                name = f"bench-{index}." + ("mkv" if streaming else "bin")
                path = self.metadata / f"{index:04}.torrent"
                info = self.tool("make", out=path, name=name, size=self.size, piece_len=256*1024,
                                 key=42, tracker="http://127.0.0.1:1/announce", cache=self.args.output / "piece-cache", **self.prefix_options)
                self.items.append(dict(hash=info["info_hash"], path=path, name=name))
            ready = self.sandbox.root / "seed-ready.json"
            self.seed = self.spawn([self.args.seed, "serve", "--torrents", self.metadata, "--peers", peers,
                                    "--tracker-port", 0, "--ready", ready, "--rate", rate, *(["--probe-upload"] if self.args.probe_upload else [])], log="seed")
            wait(ready.exists)
            binding = json.loads(ready.read_text())
            self.tracker = f"http://127.0.0.1:{binding['tracker_port']}"
            for item in self.items:
                old = b"http://127.0.0.1:1/announce"
                new = (self.tracker + "/announce").encode()
                data = item["path"].read_bytes()
                item["path"].write_bytes(data.replace(str(len(old)).encode()+b":"+old,
                                                     str(len(new)).encode()+b":"+new, 1))
            env = dict(self.sandbox.env)
            if self.args.preload:
                env.update(LD_PRELOAD=str(self.args.preload), FBT_BENCH_COUNTERS=str(self.output / "allocations.json"))
            command = DaemonContract.command(self.args.binary, 0, True)
            if self.args.strace:
                command = [self.args.strace, "-f", "-c", "-w", "-o", self.output / "syscalls.txt", *command]
            self.daemon = self.spawn(command, env, "daemon")
            path = DaemonContract.socket_path(self.sandbox)
            wait(lambda: path.exists() or self.daemon.poll() is not None)
            if self.daemon.poll() is not None:
                raise RuntimeError((self.output / "daemon.log").read_text())
            self.client = ApiConn(SimpleNamespace(sandbox=self.sandbox, case=Check()), timeout=60)
            self.pid = struct.unpack("3i", self.client.socket.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]
            self.client.hello()
            wait(lambda: self.api("app.info")["state"] == "running")
            if self.args.profile:
                self.profiler = Sampler(self.pid, self.output / "profile.json")
                self.profiler.start()
            yield self
        finally:
            workload_error = sys.exception()
            errors = []
            def attempt(action):
                try:
                    action()
                except BaseException as error:
                    errors.append(error)
            def stop_daemon():
                if hasattr(self, "daemon") and self.daemon.poll() is None:
                    try:
                        os.kill(getattr(self, "pid", self.daemon.pid), signal.SIGINT)
                    except ProcessLookupError:
                        pass
                    try:
                        self.daemon.wait(10)
                    except subprocess.TimeoutExpired:
                        pass  # The ownership audit below fails and cleans survivors.
            def stop_seed():
                if hasattr(self, "seed") and self.seed.poll() is None:
                    self.seed.terminate()
                    try:
                        self.seed.wait(2)
                    except subprocess.TimeoutExpired:
                        pass
            if hasattr(self, "profiler"):
                attempt(self.profiler.stop)
            attempt(stop_daemon)
            if hasattr(self, "client"):
                attempt(self.client.close)
            attempt(stop_seed)
            survivors = []
            def audit():
                survivors.extend(self.owners.survivors())
            attempt(audit)
            attempt(lambda: self.owners.cleanup(deadline=time.monotonic()+4))
            for child in self.children:
                attempt(lambda child=child: child.wait(2))
            for handle in self.handles:
                attempt(handle.close)
            if survivors:
                errors.append(RuntimeError(f"benchmark leaked processes (cleaned): {survivors}"))
            def check_drain():
                if hasattr(self, "daemon"):
                    if self.daemon.returncode != 0:
                        raise RuntimeError(f"daemon exit {self.daemon.returncode}")
                    if not expected_daemon_stderr((self.output / "daemon.log").read_text()):
                        raise AssertionError("daemon diagnostics or shutdown drain failed; see daemon.log")
            attempt(check_drain)
            attempt(self.sandbox.temporary.cleanup)
            if errors:
                if workload_error:
                    errors.insert(0, workload_error)
                raise BaseExceptionGroup("benchmark teardown failed", errors) from None
            (self.output / "teardown.json").write_text('{"drained":true,"survivors":0}\n')

    def api(self, op, args=None, **kwargs):
        return self.client.call(op, args or {}, **kwargs)["result"]

    def add(self, item, stopped=False, magnet=False):
        source = dict(path=str(item["path"]))
        if magnet:
            source = dict(magnet=f"magnet:?xt=urn:btih:{item['hash']}&tr=" + urllib.parse.quote(self.tracker+"/announce", safe=""))
        return self.api("torrents.add", dict(source=source, save_path=str(self.sandbox.downloads), stopped=stopped))

    def stats(self):
        with urllib.request.urlopen(self.tracker + "/stats", timeout=10) as response:
            return json.load(response)

    @staticmethod
    def pressure():
        return {name: Path(f"/proc/pressure/{name}").read_text().strip()
                for name in ("cpu", "io", "memory")}

    def finish_row(self, row):
        row.update(load_begin=self.load_begin, load_end=os.getloadavg(), pressure_begin=self.pressure_begin, pressure_end=self.pressure(), worktree_source=self.args.worktree_source,
                   binary_source=self.args.binary_source,
                   binary_sha256=self.args.binary_hash, seed_sha256=self.args.seed_hash, instrumented=bool(self.args.preload or self.args.profile or self.args.strace))
        (self.output / "result.json").write_text(json.dumps(row, indent=2)+"\n")
        print(json.dumps(dict(case=self.output.name, **row)), flush=True)
        return row

    def download(self, name, peers, magnet=False, streaming=False):
        with self.setup(name, peers=peers, streaming=streaming):
            item = self.items[0]
            before, seed_before = proc(self.pid), proc(self.seed.pid)
            started = time.monotonic()
            self.add(item, magnet=magnet)
            first, prefix, last, playable = None, None, None, None
            observations = []
            deadline = started + self.args.timeout
            while time.monotonic() < deadline:
                row = self.api("torrents.get", dict(hash=item["hash"]))["torrent"]
                if row["state"] == "error":
                    raise RuntimeError(row)
                # A parked/dormant typed view can have unknown counters. The
                # seed independently checks every observed payload/Unchoke.
                assert row.get("uploaded_bytes") in (None, 0) and not row["upload_enabled"], row
                if playable is None and (row.get("playable") or {}).get("ready"):
                    playable = time.monotonic() - started
                complete = row.get("completed_bytes") == self.size and row["state"] == "done"
                if complete and (not streaming or playable is not None):
                    observed = time.monotonic()-started
                    first, prefix, last = (v if v is not None else observed for v in (first,prefix,last))
                    break
                if row["checked"] and not complete:
                    frame = self.client.response(self.client.send("torrents.pieces", dict(hash=item["hash"])))
                    if not frame.get("ok"):
                        assert frame.get("error",{}).get("code") == "invalid_state", frame
                        continue  # Completion can park the map between these two reads.
                    pieces = frame["result"]
                    assert {0, pieces["count"]-1} <= set(pieces["priority"]), "streaming must prioritize the first and last pieces"
                    bits = base64.b64decode(pieces["have"])
                    have = lambda i: bool(bits[i//8] & (128 >> (i%8)))
                    now = time.monotonic() - started
                    if first is None and any(bits):
                        first = now
                    if last is None and have(pieces["count"]-1):
                        last = now
                    prefix_count = min(pieces["count"], (8*MIB)//pieces["piece_size"])
                    if prefix is None and all(have(i) for i in range(prefix_count)):
                        prefix = now
                    if len(observations) < 4 and first is not None:
                        observations.append(dict(seconds=now, completed_bytes=row["completed_bytes"], priority=pieces["priority"]))
                time.sleep(.02)
            else:
                raise TimeoutError(f"download stalled: {row}")
            seconds = time.monotonic() - started
            measurement = delta(before, proc(self.pid), seconds, self.size)
            measurement["seed_cpu_s"] = proc(self.seed.pid)["cpu_s"]-seed_before["cpu_s"]
            measurement.update(first_piece_s=first, prefix_8mib_s=prefix, last_piece_s=last, playable_s=playable)
            stats = self.stats()["torrents"][0]
            # Keep observations from a failed behaviour check without promoting
            # them to a successful result or including them in comparisons.
            (self.output / "observed.json").write_text(json.dumps(dict(measurement=measurement, seed=stats), indent=2)+"\n")
            assert stats["uploaded_bytes"] == 0 and stats["unchokes"] == 0, stats
            if magnet:
                assert stats["metadata_pieces"] > 0, "magnet must obtain metadata over the peer wire"
            if self.args.probe_upload:
                assert stats["probes"] > 0, stats
            order = stats["first_requests"]
            if peers == 1:
                # Across independent peers, arrival order also reflects OS
                # scheduling. Only one wire establishes picker request order.
                assert {0, (self.size-1)//(256*1024)} <= set(order[:2]), stats
                middle = [piece for piece in order if piece not in (0,(self.size-1)//(256*1024))]
                assert middle == sorted(middle), "single-peer streaming must stay sequential"
            self.tool("verify", file=self.sandbox.downloads/item["name"], size=self.size, key=42, **self.prefix_options)
            if streaming:
                assert playable is not None, "daemon never reported a playable prefix"
                prefix_file = self.disk / "verified-prefix.mkv"
                with (self.sandbox.downloads/item["name"]).open("rb") as source:
                    prefix_file.write_bytes(source.read(8*MIB))
                subprocess.run(["ffmpeg", "-v", "error", "-i", str(prefix_file), "-frames:v", "1", "-f", "null", "-"],
                               env=self.sandbox.env, cwd=self.disk, check=True, timeout=30, stdout=subprocess.DEVNULL)
            with (self.sandbox.downloads/item["name"]).open("rb") as payload:
                digest = hashlib.file_digest(payload, "sha256").hexdigest()
            measurement.update(seed=stats, observations=observations, payload_sha256=digest,
                               bytes=self.size, peers=peers, magnet=magnet)
            return self.finish_row(measurement)

    def recheck(self, name):
        with self.setup(name):
            item = self.items[0]
            self.tool("write", file=self.sandbox.downloads/item["name"], size=self.size, key=42)
            before = proc(self.pid)
            started = time.monotonic()
            self.add(item, stopped=True)
            row = wait(lambda: (r if (r:=self.api("torrents.get", dict(hash=item["hash"]))["torrent"])["checked"] else None), self.args.timeout)
            initial = delta(before, proc(self.pid), time.monotonic()-started, self.size)
            before = proc(self.pid)
            started = time.monotonic()
            response = self.api("torrents.recheck", dict(hashes=[item["hash"]]), ref="bench-recheck")
            assert response["state"] == "pending", response
            while time.monotonic() < started + self.args.timeout:
                event = self.client.pushed.pop(0) if self.client.pushed else self.client.receive()
                if event.get("ev") == "mutation.result" and event.get("ref") == "bench-recheck":
                    assert event["outcome"] == "done" and event["result"]["have_bytes"][item["hash"]] == self.size, event
                    break
            else:
                raise TimeoutError("explicit recheck did not finish")
            measured = delta(before, proc(self.pid), time.monotonic()-started, self.size)
            assert self.api("torrents.get", dict(hash=item["hash"]))["torrent"]["state"] == "paused"
            return self.finish_row(dict(initial=initial, recheck=measured, bytes=self.size))

    def many(self, name, count, active):
        with self.setup(name, count=count, size=64*MIB, rate=32*1024):
            for item in self.items:
                self.add(item, stopped=not active)
            def settled():
                rows = self.api("torrents.list")["torrents"]
                return rows if len(rows) == count and all(r["checked"] and r["state"] in (("downloading", "stalled") if active else ("paused",)) and (not active or r["completed_bytes"] > 0) for r in rows) else None
            rows = wait(settled, self.args.timeout)
            before = proc(self.pid)
            started = time.monotonic()
            time.sleep(self.args.seconds)
            after = proc(self.pid)
            measured = delta(before, after, time.monotonic()-started)
            end = self.api("torrents.list")["torrents"]
            assert len(end) == count and all(r["checked"] and r["state"] in (("downloading", "stalled") if active else ("paused",)) for r in end), "workload must remain active or paused throughout the window"
            if active:
                assert all(r["completed_bytes"] > old["completed_bytes"] for r,old in zip(sorted(end,key=lambda r:r["hash"]),sorted(rows,key=lambda r:r["hash"]))), "every active torrent must make verified progress"
            stats = self.stats()
            assert all(t["uploaded_bytes"] == 0 and t["unchokes"] == 0 for t in stats["torrents"])
            measured.update(torrents=count, active=active, cpu_s_per_torrent=measured["cpu_s"]/count,
                            rss_kb_per_torrent=measured["rss_kb"]/count)
            return self.finish_row(measured)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--binary-source", required=True, help="source revision identified by the measured binary's build")
    p.add_argument("--seed", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--disk-root", type=Path, default=Path("/home/oliver/Projects/odcpw"))
    p.add_argument("--mib", type=int, default=1024)
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--seconds", type=float, default=20)
    p.add_argument("--timeout", type=float, default=300)
    p.add_argument("--cases", nargs="+", default=["download1", "download4", "download16", "magnet", "streaming", "recheck", "idle100", "active100", "idle500", "active500"])
    p.add_argument("--preload", type=Path)
    p.add_argument("--profile", action="store_true", help="separate per-thread kernel CPU-clock PC sampling")
    p.add_argument("--strace", type=Path, help="separate syscall count run, including threads and owned helper processes")
    p.add_argument("--probe-upload", action="store_true", help="separate hostile-peer proof; intentionally disrupts that peer")
    args = p.parse_args()
    for key in ("binary", "seed", "output", "disk_root", "preload", "strace"):
        if getattr(args, key) is not None:
            setattr(args, key, getattr(args, key).resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    args.worktree_source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    with args.binary.open("rb") as source:
        args.binary_hash = hashlib.file_digest(source, "sha256").hexdigest()
    with args.seed.open("rb") as source:
        args.seed_hash = hashlib.file_digest(source, "sha256").hexdigest()
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    failures = []
    owners = ProcessRun()
    for index in range(args.runs):
        for case in args.cases:
            run, name = Run(args, owners), f"{case}-{index+1}"
            try:
                if case.startswith("download"):
                    run.download(name, int(case.removeprefix("download")))
                elif case == "magnet":
                    run.download(name, 1, magnet=True)
                elif case == "streaming":
                    run.download(name, 1, streaming=True)
                elif case == "recheck":
                    run.recheck(name)
                elif case.startswith(("idle", "active")):
                    active = case.startswith("active")
                    run.many(name, int(case.removeprefix("active" if active else "idle")), active)
                else:
                    raise ValueError(case)
            except Exception as error:
                # Retain failed observations and continue independent cases ONLY
                # after proving cleanup; an unsuccessful run is never averaged in.
                if run.owners.survivors():
                    raise
                failure = dict(case=name, error=str(error), type=type(error).__name__, traceback=traceback.format_exception(error))
                failures.append(failure)
                if hasattr(run, "output"):
                    (run.output / "failure.json").write_text(json.dumps(failure, indent=2)+"\n")
                print(json.dumps(failure), flush=True)
    survivors = owners.survivors()
    if survivors:
        owners.cleanup()
        raise RuntimeError(f"processes survived this benchmark suite (cleaned): {survivors}")
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
