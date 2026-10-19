#!/usr/bin/env python3
"""Shared daemon integration harness, using real serve processes and private XDG.

Run locally or in CI with --no-systemd (default), or use --systemd for disposable
transient units and disposable static-unit links. Static-unit fixtures verify
all private XDG paths and working directory before launch and in the child.
P1-06 adds ApiConn. --serve-fixture provides owned real-engine UI preconditions
through managed recipes; stopping the launcher or closing an initially open
stdin tears down every child. There is no --self-test mode.
"""
import argparse
import base64
import ctypes
import fcntl
from contextlib import ExitStack, contextmanager
import hashlib
import json
import math
import os
import pwd
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
from typing import NamedTuple
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

from fbtlog import Case, REPO, slug, suite_status
import fixtures



PRIVATE_PATHS = ("HOME", "TMPDIR", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
                 "XDG_STATE_HOME", "XDG_CACHE_HOME")
OWNER_UNIT_DIRECTORY = Path(pwd.getpwuid(os.geteuid()).pw_dir) / ".config/systemd/user"


class Sandbox(fixtures.Sandbox):
    def retain(self):
        # If owned-unit cleanup cannot be proved, never delete a live daemon's
        # environment. Explicit cleanup and the GC finalizer must both be inert.
        self.temporary._finalizer.detach()
        self.temporary.cleanup = lambda: None


def private_environment(sandbox):
    root = sandbox.root.resolve()
    for key in PRIVATE_PATHS:
        path = Path(sandbox.env[key])
        if not path.is_absolute() or not path.resolve().is_relative_to(root) or not path.is_dir():
            raise RuntimeError(f"refusing process launch: {key}={path} escapes {root}")
    config = (Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml").read_text()
    save = re.search(r'^save_path = (.+)$', config, re.M)
    if not save or not Path(json.loads(save[1])).resolve().is_relative_to(root):
        raise RuntimeError("refusing process launch: download path escapes sandbox")
    return f"HOME, TMPDIR, five XDG paths and downloads verified under {root}"


def manager_environment(sandbox):
    private_environment(sandbox)
    environment = dict(sandbox.env)
    environment.pop("NOTIFY_SOCKET", None)
    # Only manager transport uses the real runtime; all filesystem state and
    # every launched service's environment stay private. Controls are runtime-only.
    environment["XDG_RUNTIME_DIR"] = f"/run/user/{os.geteuid()}"
    environment.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path=/run/user/{os.geteuid()}/bus")
    return environment



def allow_manager_reads(harness):
    """Foreground fixtures must establish absence instead of hiding query errors."""
    systemctl = shutil.which("systemctl", path=os.defpath)
    if systemctl is None:
        return  # The product verifies local manager absence without a controller.
    fixtures.path_shim("systemctl", 'case " $* " in *" show "*) exec ' +
                       shlex.quote(systemctl) + ' "$@";; *) exit 99;; esac', harness.sandbox.root)
    def retain_calls():
        log = harness.sandbox.root / "shims/systemctl.log"
        if log.exists():
            shutil.copyfile(log, harness.case.path.with_suffix(".systemctl.log"))
    harness.resources.callback(retain_calls)


class ControlResult(NamedTuple):
    code: int | None
    output: str


def process_state(pid):
    try:
        return (Path("/proc") / str(pid) / "stat").read_text().rsplit(") ", 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return None


def process_alive(pid):
    state = process_state(pid)
    return state is not None and state[0] not in ("Z", "X")


class DaemonContract:
    """Process/fixture compatibility contract. Future API assumptions stay here.

    P5-04 global transfer.set_limits controls the managed fixture seed.
    """
    unit_variable = "FBITTORRENT_UNIT"

    @staticmethod
    def command(binary, idle_exit_secs=300, no_notify=False):
        return [str(binary), "serve", "--idle-exit-secs", str(idle_exit_secs), *(["--no-notify"] if no_notify else [])]

    @staticmethod
    def throttle(client, hash_, bps):
        frame=client.call("transfer.set_limits", dict(upload_bps=bps))
        if frame.get("ok") is not True:raise ValueError("fixture throttle failed: "+json.dumps(frame))
        return frame

    @staticmethod
    def socket_path(sandbox):
        return Path(sandbox.env["XDG_RUNTIME_DIR"]) / "fbittorrent/api.sock"


class NotifySocket:
    """A real AF_UNIX datagram receiver for sd_notify messages from a child."""
    def __init__(self, sandbox):
        self.path = Path(sandbox.env["XDG_RUNTIME_DIR"]) / "notify.sock"
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        self.socket.bind(str(self.path))
        self.path.chmod(0o600)

    def receive(self, timeout=5):
        self.socket.settimeout(timeout)
        value=self.socket.recv(65536).decode()
        self.received_at=time.monotonic()
        return value

    def close(self):
        self.socket.close()
        self.path.unlink(missing_ok=True)


class ManagedProcess:
    def __init__(self, case, sandbox, argv, name, systemd=False):
        self.case, self.sandbox, self.systemd = case, sandbox, systemd
        self.unit = sandbox.unit if name == "process" else sandbox.unit.removesuffix(".service") + "-" + slug(name) + ".service"
        self.stdout = case.path.with_suffix("." + slug(name) + ".stdout.log")
        private_environment(sandbox)
        command = list(map(str, argv))
        environment = dict(sandbox.env, **{DaemonContract.unit_variable: self.unit})
        launcher_env = environment.copy()
        self.closed = False
        # systemd's client must discover the real user bus even when no explicit
        # DBUS_SESSION_BUS_ADDRESS exists. Only the service gets sandbox XDG paths.
        self.control_env = manager_environment(sandbox)
        if systemd:
            self.systemctl = shutil.which("systemctl", path=os.defpath)
            runner = shutil.which("systemd-run", path=os.defpath)
            if not runner or not self.systemctl:
                raise RuntimeError("--systemd requires systemd-run and systemctl")
            command = [runner, "--user", "--quiet", "--wait", "--pipe", "--collect",
                       "--service-type=exec", "--unit=" + self.unit, "--property=TimeoutStopSec=10s",
                       "--property=WorkingDirectory=" + str(sandbox.root),
                       "--property=KillMode=control-group",
                       "--property=UnsetEnvironment=FBITTORRENT_NO_START WATCHDOG_PID WATCHDOG_USEC LISTEN_PID LISTEN_FDS LISTEN_FDNAMES TMUX",
                       *["--setenv=" + key + "=" + environment[key] for key in sandbox.keys], "--", *command]
            # systemd-run emits its own READY/RUN_UNIT if it inherits this value.
            # Only the actual service child should see our fake notify endpoint.
            launcher_env = self.control_env
        step = case.step("spawn " + shlex.join(list(map(str, argv))), "owned child starts with isolated XDG paths and captured stderr")
        with self.stdout.open("w") as output:
            self.child = case.popen(command, env=launcher_env, cwd=sandbox.root,
                                    stdin=subprocess.DEVNULL, stdout=output, start_new_session=True)
        step.ok(f"PID {self.child.pid}; mode={'systemd' if systemd else 'foreground'}; unit={self.unit}; "
                f"XDG root={sandbox.root}.", artifacts=[self.stdout, case.daemon_path])

    def wait(self, timeout=10, code=0):
        step = self.case.step("wait for owned process", f"exit {code} within {timeout} seconds")
        try:
            observed = self.child.wait(timeout=timeout)
            return step.close(observed == code, f"PID {self.child.pid} exited {observed}.", [self.stdout])
        except subprocess.TimeoutExpired:
            return step.fail(f"PID {self.child.pid} still running at deadline.", [self.stdout])

    def control(self, *args, timeout=3):
        child = self.case.popen([self.systemctl, "--user", *args], env=self.control_env,
                               stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=timeout)
            return child.returncode, stdout.strip()
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate(timeout=2)
            return None, "systemctl timed out"

    def unit_state(self):
        code, output = self.control("show", self.unit, "--property=LoadState", "--property=ActiveState")
        return code, dict(line.split("=", 1) for line in output.splitlines() if "=" in line)

    def group_members(self):
        members = []
        for path in Path("/proc").iterdir():
            if path.name.isdigit():
                state = process_state(int(path.name))
                if state is not None and state[0] not in ("Z", "X") and int(state[2]) == self.child.pid:
                    members.append(int(path.name))
        return members

    def signal_group(self, number):
        try:
            os.killpg(self.child.pid, number)
        except ProcessLookupError:
            pass

    def close(self):
        if self.closed:
            return
        step = self.case.step("stop owned process", "owned transient unit, live process group and launcher are all gone")
        observations, failures = [], []
        if self.systemd:
            code, state = self.unit_state()
            observations.append(f"unit before cleanup: exit {code}, {state}")
            if code != 0 or state.get("LoadState") != "not-found":
                stopped, output = self.control("stop", self.unit, timeout=12)
                observations.append(f"stop unit: exit {stopped}, {output!r}")
                if stopped != 0:
                    # Even a dead systemd-run launcher can leave its service
                    # alive. Escalate against that owned unit, never just its client.
                    killed, output = self.control("kill", "--signal=KILL", "--kill-whom=all", self.unit)
                    observations.append(f"kill unit: exit {killed}, {output!r}")
                    code, state = self.unit_state()
                    if code != 0 or state.get("LoadState") != "not-found":
                        failures.append("unit stop failed")

        # A reaped launcher is not evidence that its descendants have exited.
        members = self.group_members()
        observations.append(f"live group members before cleanup: {members}")
        if members:
            self.signal_group(signal.SIGTERM)
            deadline = time.monotonic() + 3
            while self.group_members() and time.monotonic() < deadline:
                time.sleep(.02)
            if self.group_members():
                self.signal_group(signal.SIGKILL)
                failures.append("process group required SIGKILL")
                deadline = time.monotonic() + 2
                while self.group_members() and time.monotonic() < deadline:
                    time.sleep(.02)
        try:
            code = self.child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.signal_group(signal.SIGKILL)
            code = self.child.wait(timeout=3)
            failures.append("launcher required SIGKILL")
        remaining = self.group_members()
        observations.append(f"launcher exit {code}; live group members after cleanup: {remaining}")
        if remaining:
            failures.append("live process group remains")
        unit_gone = True
        if self.systemd:
            deadline = time.monotonic() + 3
            while True:
                code, state = self.unit_state()
                unit_gone = code == 0 and state.get("LoadState") == "not-found"
                if unit_gone or time.monotonic() >= deadline:
                    break
                time.sleep(.02)
            observations.append(f"unit after cleanup: exit {code}, {state}")
            if not unit_gone:
                failures.append("transient unit was not collected")
        self.closed = not remaining and unit_gone
        if not self.closed:
            self.sandbox.retain()
        step.close(not failures, "; ".join(observations + failures) + ".")


class Harness:
    def __init__(self, case, binary, sandbox, systemd=False):
        self.case, self.binary, self.sandbox, self.systemd = case, str(binary), sandbox, systemd
        case.check(True, "private process environment verified before any launch", private_environment(sandbox))
        self.resources = ExitStack()
        self.names = {}

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.resources.close()
        failures = re.findall(r"thread .*panicked|obligation.*leak|closing native (?:torrent session|runtime):|native torrent session did not drain", self.stderr())
        self.case.check(not failures, "owned process stderr contains no worker panic or failed engine drain",
                        f"Unexpected runtime failures={failures!r}.")

    def spawn(self, argv, name="process"):
        self.names[name] = self.names.get(name, 0) + 1
        if self.names[name] > 1:
            name += "_" + str(self.names[name])
        process = ManagedProcess(self.case, self.sandbox, argv, name, self.systemd)
        self.resources.callback(process.close)
        return process

    def notify_socket(self):
        receiver = NotifySocket(self.sandbox)
        self.resources.callback(receiver.close)
        self.sandbox.env["NOTIFY_SOCKET"] = str(receiver.path)
        if "NOTIFY_SOCKET" not in self.sandbox.keys:
            self.sandbox.keys.append("NOTIFY_SOCKET")
        return receiver

    def start_serve(self, idle_exit_secs=300, no_notify=False):
        self.notify = self.notify_socket()
        return self.spawn(DaemonContract.command(self.binary, idle_exit_secs, no_notify))

    @property
    def engine_path(self):
        return Path(self.sandbox.env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"

    def engine_call(self, path, method="GET", args=(), timeout=2):
        """Legacy framing retained for compatibility regressions; fixtures/probes use API v1."""
        request = json.dumps(dict(method=method, path=path, args=list(args), file=None)).encode()
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(timeout)
            connection.connect(str(self.engine_path))
            connection.sendall(struct.pack(">I", len(request)) + request)

            def read(size):
                value = bytearray()
                while len(value) < size:
                    part = connection.recv(size - len(value))
                    if not part:
                        raise EOFError("engine socket closed during frame")
                    value.extend(part)
                return value

            size = struct.unpack(">I", read(4))[0]
            if size > 32 * 1024 * 1024:
                raise ValueError(f"legacy frame too large: {size}")
            return json.loads(read(size))

    def receive_ready(self):
        step = self.case.step("receive daemon readiness", "READY=1 datagram from the owned serve process within 10 seconds")
        try:
            observed = self.notify.receive(timeout=10)
            return step.close(observed == "READY=1", f"Datagram={observed!r}.")
        except OSError as error:
            return step.fail(f"No readiness: {error}.")

    def daemon_pid(self):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.connect(str(self.engine_path))
            return struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]

    def stderr(self):
        return self.case.suite.stderr_path.read_bytes()[self.case.stderr_offset:].decode(errors="replace")

    def wait_engine(self):
        def observed():
            with ApiConn(self) as client:
                client.hello()
                response=client.call("app.info")
                return response.get("result",{}).get("state") in ("running","parked"),json.dumps(response)
        return self.wait_for(observed,"daemon API reports a ready engine slot")

    def stop_serve(self, process, number=signal.SIGTERM, code=0):
        step = self.case.step("signal owned daemon", f"signal {number} reaches the serve PID (not the systemd-run launcher)")
        pid = self.daemon_pid()
        began = time.monotonic()
        os.kill(pid, number)
        step.ok(f"Delivered signal {number} to owned PID {pid}.")
        process.wait(timeout=10, code=code)
        elapsed = time.monotonic() - began
        self.case.check(elapsed < 10 and not self.engine_path.exists(),
                        "daemon exits and removes its socket within 10 seconds", f"elapsed={elapsed:.3f}s; socket={self.engine_path.exists()}.")

    def lock_free(self):
        path = self.engine_path.with_name("engine.lock")
        step = self.case.step("probe instance lock after exit", "stable engine.lock exists and flock succeeds without blocking")
        try:
            with path.open("r+") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return step.ok(f"Acquired and released {path}; lock inode remains {path.stat().st_ino}.")
        except OSError as error:
            return step.fail(f"Lock unavailable: {error}.")

    def cli(self, *args, code=0, timeout=10, start=False, on_exit=None):
        private_environment(self.sandbox)
        command = [self.binary, *([] if start else ["--no-start"]), *args, "--json"]
        step = self.case.step("CLI " + shlex.join(command), f"exit {code} with one JSON object")
        child = self.case.popen(command, env=self.sandbox.env, cwd=self.sandbox.root, stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=timeout)
            if on_exit is not None:on_exit(time.monotonic())
        except subprocess.TimeoutExpired:
            child.kill()
            stdout, _ = child.communicate(timeout=2)
            step.fail(f"CLI timed out; stdout={stdout!r}.")
            return {}
        try:
            value = json.loads(stdout)
        except ValueError:
            value = None
        step.close(child.returncode == code and isinstance(value, dict),
                   f"Exit {child.returncode}; stdout={stdout!r}.")
        if getattr(self, "installed_unit", None) is not None:
            self.installed_unit.verify_child()
        return value if isinstance(value, dict) else {}

    def wait_for(self, check, label, timeout=10):
        step = self.case.step("wait for " + label, f"{label} within {timeout} seconds")
        deadline = time.monotonic() + timeout
        observed = "No observation."
        while time.monotonic() < deadline:
            try:
                ready, observed = check()
            except (OSError, ValueError, EOFError) as error:
                ready, observed = False, f"{type(error).__name__}: {error}"
            if ready:
                return step.ok(str(observed))
            time.sleep(min(.05, max(0, deadline - time.monotonic())))
        return step.fail(f"Timed out; last observation: {observed}.")


class ApiEngine:
    """One serialized v1 test connection to a real managed serve process."""
    def __init__(self, harness):
        self.harness = harness
        self.path = DaemonContract.socket_path(harness.sandbox)
        self.process = harness.start_serve(idle_exit_secs=0)
        if not (harness.receive_ready() and harness.wait_engine()):
            raise RuntimeError("owned cap daemon did not become ready")
        self.client = ApiConn(harness)
        self.client.hello()
        self.lock = threading.RLock()
        harness.resources.callback(self.client.close)

    def call(self, op, args=None):
        # The observation thread and cap edits share no unsynchronized reader.
        with self.lock:
            return self.client.call(op, {} if args is None else args)


def cap_engine(harness):
    return ApiEngine(harness)


def engine_value(engine, op, args=None):
    response = engine.call(op, args)
    if response.get("ok") is not True:
        raise ValueError(f"{op}: {response!r}")
    return response["result"]


def engine_row(engine, hash_):
    return next(row for row in engine_value(engine, "torrents.list")["torrents"] if row["hash"] == hash_)


@contextmanager
def cap_observations(case, engines, hash_):
    """Poll actual engine states and assert the cached metainfo was not rebuilt."""
    stop, observations, errors = threading.Event(), [], []
    began = time.monotonic()
    metadata = Path(engines["seed"].harness.sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine" / (hash_ + ".torrent")
    fingerprint = lambda: (metadata.stat().st_ino, metadata.stat().st_mtime_ns)
    before = fingerprint()

    def sample_once():
        try:
            for name, engine in engines.items():
                row = engine_row(engine, hash_)
                observations.append(dict(after_s=time.monotonic()-began, engine=name,
                                         state=row["state"], completed=row["completed_bytes"],
                                         dl_bps=row.get("dl_bps"), ul_bps=row.get("ul_bps"),
                                         metadata_token=fingerprint() if name == "seed" else None))
        except (OSError, ValueError, EOFError, StopIteration) as error:
            errors.append(str(error))

    def sample():
        while not stop.is_set():
            sample_once()
            stop.wait(.02)

    thread = threading.Thread(target=sample, daemon=True)
    thread.start()
    try:
        yield sample_once
    finally:
        stop.set()
        thread.join(timeout=2)
        artifact = case.path.with_suffix(".states.json")
        artifact.write_text(json.dumps(observations, indent=2) + "\n")
        sequences = {name: [row["state"] for row in observations if row["engine"] == name]
                     for name in engines}
        step = case.step("observe state sequences during cap edits",
                         "every real engine stays out of checking, with no observer errors or metadata replacement")
        step.started = began
        step.close(
            not thread.is_alive() and not errors and all(sequences.values()) and
            not any(state.startswith("checking") for seq in sequences.values() for state in seq) and
            all(row["metadata_token"] == before for row in observations if row["engine"] == "seed") and
            fingerprint() == before,
            f"Per-engine states={sequences}; errors={errors}; metainfo before={before}, after={fingerprint()}.",
            artifacts=[artifact])


def cap_change_no_check(binary, systemd):
    with Case("daemon", "cap_change_no_check") as case, Sandbox() as seedbox, \
            Sandbox() as leechbox, Harness(case, binary, seedbox, systemd) as seedh, \
            Harness(case, binary, leechbox, systemd) as leechh:
        # A real local tracker advertises only the isolated seed's port. Payload
        # then travels between two real fbittorrent engines, never through HTTP.
        config = tomllib.loads((Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml").read_text())
        peer = socket.inet_aton("127.0.0.1") + struct.pack(">H", config["peer_port"])
        tracker_body = fixtures.bencode(dict(interval=1, peers=peer))
        with fixtures.HttpFixture(seedbox.root, {"/announce": dict(base64=base64.b64encode(tracker_body).decode())}) as tracker:
            payload = seedbox.downloads / "daemon-fixture.bin"
            payload.write_bytes(os.urandom(8 * 1024 * 1024))
            fixture = fixtures.torrent_fixture(seedbox.root, [payload], tracker=tracker.base + "/announce")
            seed, leech = cap_engine(seedh), cap_engine(leechh)
            seedh.cli("add", str(fixture.torrent), "--seed", "--quiet")
            if not seedh.wait_for(lambda: (engine_row(seed, fixture.info_hash)["state"] == "seeding",
                                          str(engine_row(seed, fixture.info_hash))), "verified seed"):
                return

            def set_cap(bps):
                step = case.step(f"set live upload cap to {bps}", "synchronous success from the real engine")
                reply = seed.call("torrents.set_upload_limit", dict(hashes=[fixture.info_hash], bps=bps))
                step.close(reply.get("ok") is True and reply["result"]["state"] == "done", str(reply))

            set_cap(32768)
            leechh.cli("add", str(fixture.torrent), "--quiet")
            if not leechh.wait_for(lambda: ((row := engine_row(leech, fixture.info_hash))["completed_bytes"] >= 32768 and
                                           not row["state"].startswith("checking"), str(row)), "real leecher payload", timeout=15):
                return
            with cap_observations(case, dict(seed=seed, leech=leech), fixture.info_hash):
                for bps, lower, upper in [(131072, 65536, 262144), (32768, 1, 65536)]:
                    began = time.monotonic()
                    set_cap(bps)
                    samples, matched = [], False
                    while time.monotonic()-began < 5:
                        start, before = time.monotonic(), engine_row(leech, fixture.info_hash)["completed_bytes"]
                        time.sleep(1)
                        row = engine_row(leech, fixture.info_hash)
                        elapsed = time.monotonic()-start
                        rate = (row["completed_bytes"]-before)/elapsed
                        samples.append(dict(after_s=time.monotonic()-began, measured_bps=rate,
                                            reported_dl_bps=row["dl_bps"], completed=row["completed_bytes"]))
                        if lower <= rate <= upper and .75*bps <= row["dl_bps"] <= 1.25*bps and time.monotonic()-began < 5:
                            matched = True
                            break
                    step = case.step(f"measure leecher after cap {bps}",
                                     f"measured rate in [{lower}, {upper}] B/s and reported rate within 25% of {bps} B/s within 5 seconds")
                    step.started = began
                    step.close(
                        matched, f"Samples={samples}; tracker requests={tracker.requests}.")


def seed_toggle_no_check(binary, systemd):
    with Case("daemon", "seed_toggle_no_check") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        payload = sandbox.downloads / "daemon-fixture.bin"
        payload.write_bytes(os.urandom(1024 * 1024))
        fixture = fixtures.torrent_fixture(sandbox.root, [payload])
        engine = cap_engine(harness)
        harness.cli("add", str(fixture.torrent), "--quiet")
        if not harness.wait_for(lambda: ((row := engine_row(engine, fixture.info_hash)).get("completed_at", 0) > 0 and
                                         row["state"] == "done", str(row)), "verified stop-on-complete fixture"):
            return
        policies = []
        with cap_observations(case, dict(seed=engine), fixture.info_hash) as sample:
            for verb, expected in [(None, "default_cap"), ("seed", "unlimited"), ("noseed", "default_cap")]:
                if verb:
                    harness.cli(verb, fixture.info_hash)
                row = engine_row(engine, fixture.info_hash)
                policy = "unlimited" if row.get("upload_limit_bps") is None and row["seed"] else \
                    "default_cap" if row.get("upload_limit_bps") == 32768 and not row["seed"] else "custom"
                policies.append(policy)
                sample()
                case.step(f"observe upload policy after {verb or 'initial completion'}", expected).close(
                    policy == expected and row["state"] in ("done", "seeding"),
                    f"Policy={policy}; torrent={row}; transfer.info={engine_value(engine, 'transfer.info')}.")
        case.check(policies == ["default_cap", "unlimited", "default_cap"],
                   "seed then noseed restores the default policy", str(policies))


def fixture_payloads(binary, systemd):
    with Case("daemon", "fixture_payloads") as case, Sandbox() as sandbox, \
            fixtures.magnet_fixture() as fixture:
        data = fixture.files[0].read_bytes()
        case.check(len(data) == 256 * 1024 and fixture.info["private"] == 1,
                   "magnet fixture has exactly 256 KiB and private metadata", f"Bytes={len(data)}; private={fixture.info['private']}.")
        expected = b"".join(hashlib.sha1(data[i:i + 16384]).digest() for i in range(0, len(data), 16384))
        case.check(expected == fixture.info["pieces"] and hashlib.sha1(fixtures.bencode(fixture.info)).hexdigest() == fixture.info_hash,
                   "payload pieces and magnet info hash match the torrent bytes", fixture.info_hash)
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(fixture.magnet).query)
        with urllib.request.urlopen(query["xs"][0], timeout=2) as response:
            case.check(response.read() == fixture.torrent.read_bytes(), "magnet exact source serves the real torrent on loopback")
        with urllib.request.urlopen(query["tr"][0] + "?info_hash=" + fixture.info_hash, timeout=2) as response:
            case.check(response.read() == fixtures.bencode(dict(interval=30, complete=0, incomplete=0, peers=b"")),
                       "local tracker returns a valid empty peer list; no daemon peer pair is implied")
        multi = fixtures.two_file_fixture()
        first, second = [path.read_bytes() for path in multi.files]
        crossing = first[16384:] + second[:16384 - 123]
        case.check(len(first) % 16384 == 123 and multi.info["pieces"][20:40] == hashlib.sha1(crossing).digest(),
                   "second file starts 123 bytes into a piece and its crossing hash covers both files")
        before = multi.files[1].read_bytes()
        fixtures.corrupt_byte(multi.files[1], 37)
        after = multi.files[1].read_bytes()
        case.check(len(before) == len(after) and sum(a != b for a, b in zip(before, after)) == 1 and before[37] != after[37],
                   "corrupt_byte changes exactly the requested byte without changing length")
        mkv = fixtures.mkv_fixture(64 * fixtures.MIB)
        with mkv.open("rb") as source:
            header = source.read(128)
        duration_offset = header.index(bytes.fromhex("448988")) + 3
        duration = struct.unpack(">d", header[duration_offset:duration_offset + 8])[0] / 1000
        case.check(mkv.stat().st_size == 64 * fixtures.MIB and b"matroska" in header and duration == 120,
                   "64 MiB Matroska header carries a 120-second duration; random body is not playable video",
                   f"Bytes={mkv.stat().st_size}; duration={duration}s.")
        iso = fixtures.iso_fixture()
        case.check(iso.suffix == ".iso" and iso.stat().st_size == 4 * fixtures.MIB,
                   "non-video ISO-named payload has the requested size")


def fixture_servers(binary, systemd):
    with Case("daemon", "fixture_servers") as case, ExitStack() as stack:
        eztv, imdb, torznab = [stack.enter_context(factory()) for factory in
                              (fixtures.eztv_server, fixtures.imdb_server, fixtures.torznab_server)]
        with urllib.request.urlopen(eztv.base + "/api/get-torrents?imdb_id=1234001", timeout=2) as response:
            value = json.load(response)
            case.check(value["torrents_count"] == 1 and value["torrents"][0]["episode"] == "1",
                       "EZTV canned JSON is served over real loopback HTTP", f"Response={value!r}.")
        with urllib.request.urlopen(imdb.base + "/suggestion/x/fixture.json", timeout=2) as response:
            value = json.load(response)
            poster = value["d"][0]["i"]["imageUrl"]
        with urllib.request.urlopen(poster, timeout=2) as response:
            case.check(poster.startswith(imdb.base + "/") and response.read().startswith(b"\x89PNG\r\n\x1a\n"),
                       "IMDb canned suggestion resolves to this fixture's PNG poster", poster)
        with urllib.request.urlopen(torznab.base + "/redirect", timeout=2) as response:
            value = ET.fromstring(response.read())
            case.check(value.findtext("channel/item/title") == "Local.Fixture.1080p" and response.url == torznab.base + "/api",
                       "Torznab follows a loopback redirect to well-formed RSS", f"Requests={torznab.requests!r}.")
        try:
            urllib.request.urlopen(torznab.base + "/loop", timeout=2)
        except urllib.error.HTTPError as error:
            case.check(error.code == 302, "canned redirect loop terminates as HTTP 302", str(error))
        else:
            case.check(False, "canned redirect loop must fail")
        for suffix, expected in (("/missing", 404), ("/%2e%2e/routes.json", 403)):
            try:
                urllib.request.urlopen(eztv.base + suffix, timeout=2)
            except urllib.error.HTTPError as error:
                case.check(error.code == expected, f"HTTP fixture rejects {suffix} with {expected}", f"HTTP {error.code}.")
            else:
                case.check(False, f"HTTP fixture must reject {suffix}")
        stack.close()
        case.check(all(not server.thread.is_alive() for server in (eztv, imdb, torznab)),
                   "all local server owner threads are joined before the case ends")


def path_shims(binary, systemd):
    with Case("daemon", "path_shims") as case, Sandbox() as sandbox:
        shim = fixtures.path_shim("wl-paste", "printf '%s' 'local clipboard'")
        child = case.popen(["wl-paste", "--type", "text with spaces", "literal $()"], env=sandbox.env,
                           stdout=subprocess.PIPE, text=True)
        stdout, _ = child.communicate(timeout=5)
        argv = json.loads(shim.with_suffix(".log").read_text())
        case.check(child.returncode == 0 and stdout == "local clipboard" and argv == ["--type", "text with spaces", "literal $()"],
                   "PATH shim returns scripted stdout and records argv boundaries literally", f"stdout={stdout!r}; argv={argv!r}.")
        paths = [Path(sandbox.env[key]) for key in sandbox.keys if key.startswith("XDG_")]
        case.check(len(paths) == 5 and all(path.parent == sandbox.root and path.stat().st_mode & 0o777 == 0o700 for path in paths),
                   "all five XDG paths are private siblings under the one temporary root", f"Paths={list(map(str, paths))!r}.")


def notify_and_process(binary, systemd):
    with Case("daemon", "notify_and_process") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        notify = harness.notify_socket()
        program = ("import json,os,socket,sys,time; "
                   "print(json.dumps(dict({k:os.environ[k] for k in os.environ if k.startswith('XDG_') or k=='FBITTORRENT_UNIT'},cwd=os.getcwd())),flush=True); "
                   "sys.stderr.write('owned process diagnostic\\n'); "
                   "s=socket.socket(socket.AF_UNIX,socket.SOCK_DGRAM); s.sendto(b'READY=1\\nSTATUS=fixture child',os.environ['NOTIFY_SOCKET']); "
                   "s.close(); time.sleep(30)")
        process = harness.spawn([sys.executable, "-u", "-c", program])
        message = notify.receive()
        case.check(message == "READY=1\nSTATUS=fixture child", "real child reaches fake NOTIFY_SOCKET with READY and STATUS", message)
        harness.wait_for(lambda: (process.stdout.stat().st_size > 0, process.stdout.read_text()), "child environment report")
        value = json.loads(process.stdout.read_text())
        case.check(all(value.get(key) == sandbox.env[key] for key in sandbox.keys if key.startswith("XDG_"))
                   and value[DaemonContract.unit_variable] == sandbox.unit,
                   "child receives every isolated XDG path and the disposable unit name", f"Environment={value!r}.")
        case.check(value.get("cwd") == str(sandbox.root),
                   "actual child working directory is the sandbox root", f"cwd={value.get('cwd')!r}; root={sandbox.root}.")
        other = harness.spawn([sys.executable, "-c", "import os,sys; print(os.environ['FBITTORRENT_UNIT']); sys.stderr.write('second child diagnostic\\n'); sys.exit(7)"])
        other.wait(code=7)
        case.check(other.unit != process.unit and other.stdout.read_text().strip() == other.unit,
                   "a second owned child has its own unit and stdout; expected nonzero exits are observable",
                   f"First={process.unit}; second={other.unit}.")
        process.close()
        case.check(process.child.poll() is not None, "cleanup reaps the child before its XDG tree is removed")
        case.check("owned process diagnostic" in case.suite.stderr_path.read_text()
                   and "second child diagnostic" in case.suite.stderr_path.read_text(),
                   "stderr from both real children is retained for the case's daemon.log slice")


def launcher_exit_cleanup(binary, systemd):
    with Case("daemon", "launcher_exit_cleanup") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        # Adopt and reap the foreground orphan in the regression itself. This
        # avoids depending on PID 1's reaping policy in CI/container environments.
        libc, prior = ctypes.CDLL(None, use_errno=True), ctypes.c_int()
        if libc.prctl(37, ctypes.byref(prior), 0, 0, 0) or libc.prctl(36, 1, 0, 0, 0):
            raise OSError(ctypes.get_errno(), "cannot establish regression child subreaper")
        process, descendant, service_pid = None, None, None
        control_env = manager_environment(sandbox)
        systemctl = shutil.which("systemctl", path=os.defpath)
        try:
            program = ("import json,os,subprocess,sys,time; "
                       "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(90)']); "
                       "print(json.dumps(dict(pid=os.getpid(),descendant=p.pid)),flush=True); time.sleep(90)")
            process = harness.spawn([sys.executable, "-u", "-c", program])
            if not harness.wait_for(lambda: (process.stdout.stat().st_size > 0, process.stdout.read_text()),
                                    "launcher reports a live descendant"):
                return
            value = json.loads(process.stdout.read_text())
            descendant = value["descendant"]
            service_pid = value["pid"]
            process.child.kill()
            process.child.wait(timeout=5)
            case.check(process_alive(descendant), "descendant is still live after the launcher is killed",
                       f"launcher exit={process.child.returncode}; service PID={value['pid']}; descendant={descendant}.")
            process.close()

            def gone():
                try:
                    os.waitpid(descendant, os.WNOHANG)
                except ChildProcessError:
                    pass
                return not (Path("/proc") / str(descendant)).exists(), f"descendant state={process_state(descendant)!r}"

            harness.wait_for(gone, "descendant is killed and reaped after launcher exit", timeout=5)
            if systemd:
                child = case.popen([systemctl, "--user", "show", process.unit, "--property=LoadState", "--value"],
                                   env=control_env, stdout=subprocess.PIPE, text=True)
                stdout, _ = child.communicate(timeout=5)
                case.check(child.returncode == 0 and stdout.strip() == "not-found",
                           "owned transient unit is collected even after its launcher was killed",
                           f"exit={child.returncode}; LoadState={stdout!r}.")
        finally:
            # Contain the pre-fix regression too: never leave a failing test's
            # known orphan/unit alive while it reports the original defect.
            if process is not None and (process.child.poll() is None or
                                        descendant is not None and process_alive(descendant) or
                                        service_pid is not None and process_alive(service_pid)):
                if systemd:
                    child = case.popen([systemctl, "--user", "stop", process.unit], env=control_env,
                                       stdout=subprocess.DEVNULL)
                    child.wait(timeout=12)
                else:
                    try:
                        os.killpg(process.child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            if descendant is not None:
                try:
                    os.waitpid(descendant, 0)
                except ChildProcessError:
                    pass
            libc.prctl(36, prior.value, 0, 0, 0)


def systemd_without_bus_address(binary, systemd):
    with Case("daemon", "systemd_without_bus_address") as case:
        address = os.environ.pop("DBUS_SESSION_BUS_ADDRESS", None)
        parent_runtime = os.environ.get("XDG_RUNTIME_DIR")
        try:
            with Sandbox() as sandbox, Harness(case, binary, sandbox, systemd=True) as harness:
                program = "import json,os; print(json.dumps(dict(cwd=os.getcwd(),runtime=os.environ['XDG_RUNTIME_DIR'])))"
                process = harness.spawn([sys.executable, "-c", program])
                if process.wait():
                    value = json.loads(process.stdout.read_text())
                    case.check(value == dict(cwd=str(sandbox.root), runtime=sandbox.env["XDG_RUNTIME_DIR"])
                               and value["runtime"] != parent_runtime,
                               "without a parent bus variable, systemd still launches a child in its isolated cwd/runtime",
                               f"child={value!r}; parent runtime={parent_runtime!r}.")
        finally:
            if address is not None:
                os.environ["DBUS_SESSION_BUS_ADDRESS"] = address


def daemon_api_probe(binary, systemd):
    with Case("daemon", "daemon_api_probe") as case, Sandbox() as sandbox, \
            fixtures.magnet_fixture() as fixture, \
            Harness(case, binary, sandbox, systemd) as harness:
        allow_manager_reads(harness)
        status = harness.cli("status")
        case.check(status.get("backend") == "stopped" and status.get("app_open") is False,
                   "fresh isolated engine is closed before launch", f"Status={status!r}.")
        engine = ApiEngine(harness)
        def available():
            value = engine.call("app.info")
            return value.get("result", {}).get("state") == "running", f"Response={value!r}."
        ready = harness.wait_for(available, "managed daemon API answers app.info")
        if ready:
            case.check(engine.path.stat().st_mode & 0o777 == 0o600,
                       "current engine socket is private to this sandbox", str(engine.path))
            status = harness.cli("status")
            case.check(status.get("ok") is True and status.get("running") is True and status.get("app_open") is False,
                       "real CLI reports a live daemon without inventing an open TUI", f"Status={status!r}.")
            added = harness.cli("add", str(fixture.torrent), "--stopped", "--quiet")
            shown = harness.cli("show", fixture.info_hash)
            case.check(added.get("hash") == fixture.info_hash and shown.get("hash") == fixture.info_hash
                       and shown.get("size") == 256 * 1024,
                       "the real current engine parses the private local torrent with its exact hash and payload size",
                       f"Added={added!r}; shown={shown!r}.")
            multi = fixtures.two_file_fixture()
            harness.cli("add", str(multi.torrent), "--stopped", "--quiet")
            shown = harness.cli("show", multi.info_hash)
            case.check(shown.get("hash") == multi.info_hash and
                       [entry.get("size") for entry in shown.get("files", [])] == [16384 + 123, 256 * 1024],
                       "the real current engine accepts the mid-piece two-file torrent and preserves both lengths",
                       f"Shown={shown!r}.")
            engine.call("app.shutdown", {})
            engine.process.wait(timeout=15)
            harness.wait_for(lambda: (not engine.path.exists(), f"socket exists={engine.path.exists()}"), "current engine removes its socket")


def saved_hash_fixture(sandbox, size=256 * fixtures.MIB):
    """Sparse zero payload, real v1 hashes, and durable paused intent. No peers."""
    name, piece = "daemon-hash.bin", fixtures.MIB
    payload = sandbox.downloads / name
    with payload.open("wb") as output:
        output.truncate(size)
    info = dict(name=name, length=size, **{"piece length": piece,
                "pieces": hashlib.sha1(bytes(piece)).digest() * (size // piece), "private": 1})
    digest = hashlib.sha1(fixtures.bencode(info)).hexdigest()
    directory = Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine"
    directory.mkdir(parents=True)
    (directory / (digest + ".torrent")).write_bytes(fixtures.bencode(dict(info=info)))
    record = dict(hash=digest, name=name, source="", output_folder=str(sandbox.downloads),
                  only_files=None, upload_limit=32768, ratio_limit=0.0, category="fixture", tags="",
                  added_on=1, completion_on=0, paused=True, resume_after_migration=False)
    (directory / "torrents.json").write_text(json.dumps(dict(migrated_legacy=True, records=[record])))
    return digest, size


def ready_notify(binary, systemd):
    with Case("daemon", "ready_notify") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        digest, size = saved_hash_fixture(sandbox)
        process = harness.start_serve(no_notify=True)
        if harness.receive_ready() and harness.wait_engine():
            text = harness.stderr()
            case.check(0 <= text.find("INFO daemon ready state=starting") < text.find("INFO engine starting session"),
                       "READY is sent before session creation, hence before any saved-payload hash check can finish",
                       f"Startup ordering in stderr: ready at {text.find('INFO daemon ready')}; session at {text.find('INFO engine starting')}.")
            rows = []
            def hashed():
                response = harness.engine_call("torrents/info")
                rows[:] = json.loads(response.get("value") or "[]")
                row = next((row for row in rows if row.get("hash") == digest), {})
                return row.get("completed") == size and row.get("progress_known", True) and row.get("state") != "checkingDL", f"Restored payload row={row!r}."
            harness.wait_for(hashed, "real saved payload finishes hashing after READY", timeout=30)
            mode = harness.engine_path.stat().st_mode & 0o777
            case.check(mode == 0o600 and harness.engine_path.parent.stat().st_mode & 0o777 == 0o700,
                       "daemon listener and runtime directory are private", f"socket mode={mode:o}.")
            harness.stop_serve(process)
            harness.lock_free()


def parked_start(binary, systemd):
    with Case("daemon", "parked_start") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        digest, _ = saved_hash_fixture(sandbox, 4 * fixtures.MIB)
        ledger = Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine/torrents.json"
        saved = json.loads(ledger.read_text())
        saved["records"][0]["name"] = "ProtonVPN is part of this saved title"
        ledger.write_text(json.dumps(saved))
        config = Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        config.write_text(config.read_text().replace('network_interface = "lo"', 'network_interface = "fbmissing0"'))
        process = harness.start_serve()
        if harness.receive_ready() and harness.wait_engine():
            network = json.loads(harness.engine_call("app/network")["value"])
            rows = json.loads(harness.engine_call("torrents/info")["value"])
            case.check(network["ready"] is False and network["interface"] == "fbmissing0" and
                       network["message"] == "Waiting for fbmissing0" and len(rows) == 1 and
                       rows[0]["hash"] == digest and rows[0]["progress_known"] is False,
                       "missing interface starts parked and serves the saved ledger without claiming progress",
                       f"network={network!r}; rows={rows!r}.")
            case.check(rows[0].get("name") == "ProtonVPN is part of this saved title",
                       "interface wording replacement never changes saved torrent data", f"Saved title={rows[0].get('name')!r}.")
            harness.cli("start", digest)
            harness.cli("stop", digest)
            # Typed parsing validates add input before slot availability. Send a
            # valid disposable magnet so this check exercises the parked engine.
            response = harness.engine_call("torrents/add", method="POST",
                                           args=[("urls", "magnet:?xt=urn:btih:" + digest)])
            case.check("engine_unavailable" in str(response.get("error")) and "fbmissing0" in str(response.get("error")),
                       "add while parked fails with engine_unavailable and the configured interface", f"Response={response!r}.")
            response = harness.engine_call("torrents/delete", method="POST", args=[("hashes", digest), ("deleteFiles", "false")])
            case.check(response.get("error") is None and json.loads(harness.engine_call("torrents/info")["value"]) == [],
                       "parked remove keeps payload and durably removes the ledger record", f"Response={response!r}.")
            case.check((sandbox.downloads / "daemon-hash.bin").exists(), "parked removal preserves payload")
            harness.stop_serve(process)
            harness.lock_free()


def sigterm_drains(binary, systemd):
    with Case("daemon", "sigterm_drains") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        process = harness.start_serve()
        if harness.receive_ready() and harness.wait_engine():
            # One client has only supplied half a frame. It cannot stretch the
            # drain to the transport's normal read timeout.
            with socket.socket(socket.AF_UNIX) as partial:
                partial.connect(str(harness.engine_path))
                partial.sendall(b"\x00\x00")
                harness.stop_serve(process)
            text = harness.stderr()
            case.check(re.search(r"engine drained in \d+ ms", text) is not None and "sockets unlinked" in text,
                       "SIGTERM logs the bounded drain and socket unlink", text[-1800:])
            harness.lock_free()


def panic_exits_70(binary, systemd):
    with Case("daemon", "panic_exits_70") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        sandbox.env["FBT_TEST_PANIC_AFTER_READY"] = "1"
        sandbox.keys.append("FBT_TEST_PANIC_AFTER_READY")
        process = harness.start_serve()
        harness.receive_ready()
        process.wait(timeout=10, code=70)
        text = harness.stderr()
        case.check("ERROR daemon panic" in text and "FBT_TEST_PANIC_AFTER_READY" in text and "sockets unlinked" in text
                   and not harness.engine_path.exists(),
                   "injected panic is logged, drains, unlinks the socket and exits 70", text[-1800:])
        harness.lock_free()


def second_serve_refused(binary, systemd):
    with Case("daemon", "second_serve_refused") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        first = harness.start_serve()
        if harness.receive_ready() and harness.wait_engine():
            inode = harness.engine_path.stat().st_ino
            second = harness.spawn([*DaemonContract.command(binary), "--json"], "second")
            second.wait(timeout=8, code=1)
            value = json.loads(second.stdout.read_text())
            case.check("already_running" in value.get("error", "") and value.get("code") == 1 and
                       value.get("hint") == "fbittorrent status --json" and harness.engine_path.stat().st_ino == inode,
                       "second serve waits at most five seconds then fails without replacing the owner's socket", f"Response={value!r}; socket inode={inode}.")
            # A plain invocation must carry the same actionable hint as JSON.
            offset = len(harness.stderr())
            plain = harness.spawn(DaemonContract.command(binary), "second_plain")
            plain.wait(timeout=8, code=1)
            diagnostic = harness.stderr()[offset:]
            case.check("already_running" in diagnostic and "fbittorrent status --json" in diagnostic
                       and plain.stdout.read_text() == "" and harness.engine_path.stat().st_ino == inode,
                       "plain second serve exits 1 with already_running and the status hint, retaining the owner's socket",
                       f"Plain startup stderr={diagnostic!r}; socket inode={inode}.")
            harness.wait_engine()
            harness.stop_serve(first)
            harness.lock_free()


def engine_restart_retains_owner(binary, systemd):
    with Case("daemon", "engine_restart_retains_owner") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        process = harness.start_serve()
        if harness.receive_ready() and harness.wait_engine():
            inode, pid = harness.engine_path.stat().st_ino, harness.daemon_pid()
            lockpath = harness.engine_path.with_name("engine.lock")
            before = harness.stderr()
            response = harness.engine_call("app/restart", method="POST")
            case.check(response.get("error") is None, "legacy restart queues an owner lifecycle request", str(response))
            harness.wait_for(lambda: ("state=running engine_epoch=2" in harness.stderr(), harness.stderr()[-1400:]),
                             "new engine reaches epoch 2")
            with lockpath.open("r+") as lock:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    held = False
                except BlockingIOError:
                    held = True
            case.check(held and harness.engine_path.stat().st_ino == inode and harness.daemon_pid() == pid,
                       "restart retains the instance lock, listener inode and daemon PID", f"held={held}; inode={inode}; PID={pid}.")
            case.check(harness.stderr().count('"instance":') == before.count('"instance":') == 1,
                       "engine restart keeps the same process instance identity")
            harness.stop_serve(process, signal.SIGHUP)
            harness.lock_free()


def shutdown_inflight_add(binary, systemd):
    with Case("daemon", "shutdown_inflight_add") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        process = harness.start_serve()
        if harness.receive_ready() and harness.wait_engine():
            with socket.socket(socket.AF_UNIX) as pending:
                pending.settimeout(2)
                pending.connect(str(harness.engine_path))
                value = dict(method="POST", path="torrents/add", file=None,
                             args=[("urls", "magnet:?xt=urn:btih:" + "ab" * 20), ("paused", "true")])
                packet = json.dumps(value).encode()
                pending.sendall(struct.pack(">I", len(packet)) + packet)
                entered = harness.wait_for(lambda: ('legacy dispatch path="torrents/add"' in harness.stderr(), harness.stderr()[-600:]),
                                           "unresolved add enters live engine dispatch")
                if entered:
                    harness.stop_serve(process)
                    def read(size):
                        data = b""
                        while len(data) < size:
                            part = pending.recv(size - len(data))
                            if not part:
                                raise EOFError("shutdown discarded an already-read request's reply")
                            data += part
                        return data
                    response = json.loads(read(struct.unpack(">I", read(4))[0]))
                    case.check(response.get("error") is not None and
                               ("closing" in response["error"] or "stopping" in response["error"]),
                               "an already-read unresolved add gets a shutdown error instead of delaying drain for 90 seconds",
                               f"Reply={response!r}.")
                    harness.lock_free()

class ApiConn:
    """Real bounded NDJSON connection; retains unsolicited frames separately."""
    def __init__(self, harness, timeout=8, trace=None):
        self.harness, self.case, self.trace = harness, harness.case, trace
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(timeout)
        self.socket.connect(str(DaemonContract.socket_path(harness.sandbox)))
        self.buffer, self.pushed, self.next_id = b"", [], 1

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def close(self):
        self.socket.close()

    def send(self, op, args=None, ref=None, request_id=None):
        identifier = self.next_id if request_id is None else request_id
        self.next_id = max(self.next_id, identifier + 1)
        frame = dict(id=identifier, op=op)
        if args is not None:
            frame["args"] = args
        if ref is not None:
            frame["ref"] = ref
        self.send_frame(frame)
        return identifier

    def send_frame(self, frame, typed=True):
        self.socket.sendall(json.dumps(frame, separators=(",", ":")).encode() + b"\n")
        if self.trace is not None and typed:
            self.trace.append(dict(dir="c2s", frame=frame))
        elif self.trace is not None:
            artifact = self.case.path.with_suffix(".raw-inputs.ndjson")
            with artifact.open("a") as output:
                output.write(json.dumps(frame, sort_keys=True) + "\n")
            step = self.case.step("send deliberately invalid typed input", "the exact input is retained separately from the typed transcript")
            step.ok(json.dumps(frame, sort_keys=True), [artifact])

    def response(self, identifier):
        while (frame := self.receive()) is not None:
            if frame.get("id") == identifier:
                return frame
        raise EOFError("EOF before response " + str(identifier))

    def receive(self):
        while b"\n" not in self.buffer:
            chunk = self.socket.recv(8192)
            if not chunk:
                if self.buffer:
                    raise EOFError("API closed during a JSON line")
                return None
            self.buffer += chunk
            if len(self.buffer.split(b"\n", 1)[0]) + 1 > 1048576:
                raise ValueError("server frame exceeds 1 MiB")
        line, self.buffer = self.buffer.split(b"\n", 1)
        if len(line) + 1 > 1048576:
            raise ValueError("server frame exceeds 1 MiB")
        frame = json.loads(line)
        if self.trace is not None:
            self.trace.append(dict(dir="s2c", frame=frame))
        return frame

    def call(self, op, args=None, ref=None, error=None):
        step = self.case.step("API " + op + " " + json.dumps(args, sort_keys=True),
                              "matching success reply" if error is None else "matching " + error + " failure")
        identifier = self.send(op, args, ref)
        while True:
            frame = self.receive()
            if frame is None:
                step.fail("EOF before reply")
                raise EOFError(op)
            if "ev" in frame:
                self.pushed.append(frame)
                continue
            if frame.get("id") != identifier:
                step.fail(f"Unexpected response ID: {frame!r}")
                raise ValueError("response ID mismatch")
            ok = frame.get("ok") is True if error is None else frame.get("error", {}).get("code") == error
            step.close(ok, json.dumps(frame, sort_keys=True))
            return frame

    def hello(self, kind="test"):
        return self.call("hello", dict(proto=1, client=dict(name="daemon-api-test", version="0.2.0", kind=kind)))

    def events(self):
        while self.pushed:
            yield self.pushed.pop(0)
        while (frame := self.receive()) is not None:
            yield frame


@contextmanager
def api_owned(name, binary, systemd):
    with Case("daemon", name) as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        process = harness.start_serve()
        if not (harness.receive_ready() and harness.wait_engine()):
            raise RuntimeError("owned API fixture did not start")
        try:
            yield case, sandbox, harness, process
        finally:
            if process.child.poll() is None:
                harness.stop_serve(process)
            harness.lock_free()


@contextmanager
def stalled_api_source(case):
    """A real loopback HTTP source held until cleanup; no synthetic daemon delay."""
    import http.server
    release, entered = threading.Event(), threading.Event()
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            entered.set()
            release.wait()
            try:
                self.send_error(404)
            except OSError:
                pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    thread = threading.Thread(target=server.serve_forever, kwargs=dict(poll_interval=.05))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/held.torrent", entered
    finally:
        release.set()
        server.shutdown()
        server.server_close()
        thread.join(2)
        case.check(not thread.is_alive(), "owned stalled HTTP source joins", f"HTTP thread alive={thread.is_alive()}.")


def connection_limit(binary, systemd):
    with api_owned("connection_limit", binary, systemd) as (case, sandbox, harness, process), ExitStack() as stack:
        clients = [stack.enter_context(ApiConn(harness)) for _ in range(32)]
        for client in clients:
            client.hello()
        with ApiConn(harness) as rejected:
            failure, final, eof = rejected.receive(), rejected.receive(), rejected.receive()
            case.check(failure.get("ev") == "error" and failure["error"]["code"] == "busy"
                       and failure["error"]["details"]["limit"] == "connections" and final["ev"] == "bye" and eof is None,
                       "33rd connection receives request-less busy, bye, then EOF", f"{failure!r}; {final!r}; EOF={eof!r}.")
        clients[0].close()
        def replaced():
            with ApiConn(harness) as replacement:
                replacement.send("hello", dict(proto=1, client=dict(name="replacement", version="0", kind="test")))
                value = replacement.receive()
                return value.get("ok") is True, str(value)
        harness.wait_for(replaced, "disconnected client releases admission", timeout=2)
        path = DaemonContract.socket_path(sandbox)
        case.check(path.stat().st_mode & 0o777 == 0o600 and path.parent.stat().st_mode & 0o777 == 0o700,
                   "API socket and runtime directory are private", str(path))


def api_protocol_failure(name, binary, systemd, payload):
    with api_owned(name, binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        client.socket.sendall(payload)
        failure, final, eof = client.receive(), client.receive(), client.receive()
        case.check(failure.get("ev") == "error" and failure["error"]["code"] == "invalid_message"
                   and final == dict(ev="bye", reason="invalid_message") and eof is None,
                   "invalid_message always sends error, bye and closes", f"{failure!r}; {final!r}; EOF={eof!r}.")


def line_cap(binary, systemd):
    api_protocol_failure("line_cap", binary, systemd, b"x" * (70 * 1024) + b"\n")


def non_object_line(binary, systemd):
    api_protocol_failure("non_object_line", binary, systemd, b"[]\n")


def hello_twice(binary, systemd):
    with api_owned("hello_twice", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        first = client.hello()
        second = client.call("hello", dict(proto=1, client=dict(name="test", version="0", kind="test")), error="invalid_state")
        client.call("app.info")
        case.check(len(first["result"]["instance"]) == 32 and second["error"]["retryable"] is False,
                   "second hello is non-retryable and leaves connection usable")
        client.socket.sendall(b'{"id":0,"op":"app.info"}\n')
        invalid = client.receive()
        case.check(invalid.get("id", "missing") is None and invalid["error"]["code"] == "invalid_args",
                   "bad ID receives id:null and leaves connection open", str(invalid))
        client.call("app.info")


def hello_deadline_case(name, binary, systemd, trickle):
    with api_owned(name, binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        began = time.monotonic()
        if trickle:
            for _ in range(8):
                client.socket.sendall(b" ")
                time.sleep(.5)
        failure, final, eof = client.receive(), client.receive(), client.receive()
        elapsed = time.monotonic() - began
        case.check(4.7 <= elapsed <= 7 and failure["error"]["code"] == "hello_required"
                   and final == dict(ev="bye", reason="hello_required") and eof is None,
                   "five-second hello deadline is anchored to accept and not reset by bytes",
                   f"elapsed={elapsed:.3f}s; {failure!r}; {final!r}; EOF={eof!r}.")


def hello_deadline(binary, systemd):
    hello_deadline_case("hello_deadline", binary, systemd, False)


def slow_hello_bytes(binary, systemd):
    hello_deadline_case("slow_hello_bytes", binary, systemd, True)


def ninth_inflight_busy(binary, systemd):
    with api_owned("ninth_inflight_busy", binary, systemd) as (case, sandbox, harness, process), \
            stalled_api_source(case) as (source, entered), ApiConn(harness) as client:
        client.hello()
        batch = [dict(id=identifier, op="torrents.add", args=dict(source=dict(url=source), stopped=True))
                 for identifier in range(2, 11)]
        batch.append(dict(id=2, op="app.info"))
        client.next_id = 11
        client.socket.sendall(b"".join(json.dumps(frame).encode() + b"\n" for frame in batch))
        frame = client.receive()
        case.check(frame.get("id") == 10 and frame.get("error", {}).get("code") == "busy"
                   and frame["error"]["details"]["limit"] == "inflight",
                   "ninth pipelined in-flight operation is retryable busy before dispatch", str(frame))
        duplicate = client.receive()
        case.check(duplicate.get("id", "missing") is None and duplicate["error"]["code"] == "invalid_args",
                   "reused in-flight ID is id:null invalid_args", str(duplicate))
        case.check(entered.wait(2), "dispatch entered the owned HTTP source", f"entered={entered.is_set()}.")
        # Signal cancellation before the source is released: pending requests
        # must receive stopping errors, and bye is the final frame.
        harness.stop_serve(process)
        frames = list(client.events())
        replies = [frame for frame in frames if "id" in frame]
        outcomes = [frame for frame in frames if frame.get("ev") == "mutation.result"]
        pending = [frame for frame in replies if frame.get("ok") is True and frame["result"]["state"] == "pending"]
        case.check(len(replies) == 8 and all(frame in pending or frame.get("error", {}).get("code") == "engine_unavailable" for frame in replies)
                   and len(outcomes) == len(pending) and all(frame["outcome"] == "failed" for frame in outcomes)
                   and frames[-1] == dict(ev="bye", reason="shutdown"),
                   "all eight accepted requests get pending plus failed outcome, or stopping rejection, before final bye", json.dumps(frames))


def api_quiesce_bye(binary, systemd):
    with api_owned("api_quiesce_bye", binary, systemd) as (case, sandbox, harness, process), \
            ApiConn(harness) as first, ApiConn(harness) as second:
        first.hello("bar")
        second.hello("agent")
        second.call("app.client", dict(interactive=True, glance=True))
        before = second.call("app.info")["result"]
        epoch, inode, instance = before["engine_epoch"], DaemonContract.socket_path(sandbox).stat().st_ino, before["instance"]
        first.call("app.restart", dict(scope="engine"))
        def restarted():
            info = second.call("app.info")["result"]
            return info["state"] == "running" and info["engine_epoch"] > epoch, json.dumps(info)
        harness.wait_for(restarted, "same API connections observe replacement engine")
        case.check(DaemonContract.socket_path(sandbox).stat().st_ino == inode and second.call("app.info")["result"]["instance"] == instance,
                   "API listener and process instance survive engine restart")
        result = first.call("app.shutdown", dict(reason="test"))["result"]
        case.check(result["state"] == "pending" and isinstance(result["ref"], str), "shutdown acknowledges pending with correlation ref", str(result))
        for connection in (first, second):
            frames = list(connection.events())
            case.check(frames and frames[-1] == dict(ev="bye", reason="shutdown") and
                       sum(frame.get("ev") == "bye" for frame in frames) == 1,
                       "each connection receives bye last and then EOF", str(frames))
        process.wait()
        case.check(not DaemonContract.socket_path(sandbox).exists(), "API socket unlinked after drain")

def api_torrent_operations(binary, systemd):
    with api_owned("api_torrent_operations", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        client.hello()
        fixture = fixtures.two_file_fixture()
        fixture.torrent.write_bytes(fixtures.bencode(dict(info=fixture.info, comment="API fixture", **{"created by": "P1-06"})))
        fixtures.corrupt_byte(fixture.files[0], 0)
        args = dict(source=dict(path=str(fixture.torrent)), save_path=str(sandbox.root), stopped=True, origin="agent")
        added = client.call("torrents.add", args)["result"]
        duplicate = client.call("torrents.add", dict(args, seed=True))["result"]
        case.check(added["hash"] == fixture.info_hash and added["duplicate"] is False and duplicate["duplicate"] is True,
                   "typed add returns the exact hash and serial duplicate outcome", f"added={added!r}; duplicate={duplicate!r}.")
        with fixtures.HttpFixture(sandbox.root, {"/metadata.torrent": dict(file=fixture.torrent.name)}) as http:
            pending = client.call("torrents.add", dict(source=dict(url=http.base + "/metadata.torrent"), stopped=True), ref="url-fixture")["result"]
            case.check(pending == dict(state="pending", ref="url-fixture"), "URL add acknowledges pending with its ref", str(pending))
            outcome = client.receive()
            case.check(outcome.get("ev") == "mutation.result" and outcome["ref"] == "url-fixture" and outcome["outcome"] == "done"
                       and outcome["result"]["hash"] == fixture.info_hash and outcome["result"]["duplicate"] is True
                       and outcome["seq"] > 0 and outcome["ts"] > 0,
                       "URL completion event carries the atomic real engine add result", str(outcome))

        def checked():
            detail = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
            return detail["checked"], json.dumps(detail)
        harness.wait_for(checked, "API fixture completes real hash check")
        detail = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
        case.check(detail["private"] is True and detail["file_count"] == 2 and detail["piece_size"] == 16384
                   and detail["piece_count"] > 1 and detail["comment"] == "API fixture" and detail["created_by"] == "P1-06"
                   and detail["seed"] is False and detail["origin"] == "agent" and detail["uploaded_bytes"] == 0,
                   "detail contains real cached metadata and duplicate add preserves existing policy", str(detail))
        rows = client.call("torrents.list", dict(states=["paused"], origin=["agent"]))["result"]["torrents"]
        case.check([row["hash"] for row in rows] == [fixture.info_hash], "list filters states and accepted origin", str(rows))
        entries = client.call("torrents.files", dict(hash=fixture.info_hash))["result"]["files"]
        case.check([entry["size"] for entry in entries] == [16384 + 123, 256 * 1024]
                   and all(isinstance(entry["sort_key"], str) for entry in entries), "files carry exact sizes, selection and sort keys", str(entries))
        client.call("torrents.start", dict(hashes=[fixture.info_hash]))
        client.call("torrents.stop", dict(hashes=[fixture.info_hash]))
        client.call("torrents.set_seed", dict(hashes=[fixture.info_hash], on=True))
        client.call("torrents.set_upload_limit", dict(hashes=[fixture.info_hash], bps=32768))
        client.call("torrents.set_files", dict(hash=fixture.info_hash, include=[0], skip=[1]))
        selected = client.call("torrents.files", dict(hash=fixture.info_hash))["result"]["files"]
        case.check([file["included"] for file in selected] == [True, False], "include/skip changes are applied before done", str(selected))
        target = client.call("torrents.open_target", dict(hash=fixture.info_hash, file=0))["result"]
        case.check(target == dict(path=str(fixture.files[0]), kind="file"), "explicit open-target file index resolves the real path", str(target))
        client.call("transfer.info")
        client.call("app.network")
        client.call("torrents.get", dict(hash="f" * 40), error="not_found")
        client.call("torrents.start", dict(hashes=[fixture.info_hash], force=False), error="invalid_args")
        client.call("torrents.add", dict(source=dict(restore=fixture.info_hash)), error="invalid_args")
        client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), stage=False), error="invalid_args")
        link = sandbox.root / "symlink.torrent"
        link.symlink_to(fixture.torrent)
        client.call("torrents.add", dict(source=dict(path=str(link))), error="invalid_args")
        fifo = sandbox.root / "fifo.torrent"
        os.mkfifo(fifo)
        client.call("torrents.add", dict(source=dict(path=str(fifo))), error="invalid_args")
        large = sandbox.root / "large.torrent"
        with large.open("wb") as output:
            output.truncate(16 * fixtures.MIB + 1)
        client.call("torrents.add", dict(source=dict(path=str(large))), error="invalid_args")
        client.call("torrents.remove", dict(hashes=[fixture.info_hash], delete_files=False))
        case.check(all(path.exists() for path in fixture.files), "remove keeping files retains owned payloads")
        case.check(client.call("torrents.list", {})["result"]["torrents"] == [], "durable remove is visible before response")
        client.call("torrents.add", dict(args, files=[1]))
        selected = client.call("torrents.files", dict(hash=fixture.info_hash))["result"]["files"]
        case.check([file["included"] for file in selected] == [False, True], "initial file selection is admitted and saved atomically", str(selected))
        client.call("torrents.remove", dict(hashes=[fixture.info_hash], delete_files=False))

        magnet = "magnet:?xt=urn:btih:" + "a1" * 20 + "&dn=unresolved"
        began = time.monotonic()
        unresolved = client.call("torrents.add", dict(source=dict(magnet=magnet), stopped=True))["result"]
        elapsed = time.monotonic() - began
        case.check(unresolved["hash"] == "a1" * 20 and elapsed < 2,
                   "magnet returns its durable hash without waiting for metadata", f"elapsed={elapsed:.3f}s; {unresolved!r}.")
        client.call("torrents.stop", dict(hashes=["a1" * 20]))
        client.call("torrents.remove", dict(hashes=["a1" * 20], delete_files=False))

def dispatch_queue_busy(binary, systemd):
    with api_owned("dispatch_queue_busy", binary, systemd) as (case, sandbox, harness, process), \
            stalled_api_source(case) as (source, entered), ExitStack() as stack:
        clients = [stack.enter_context(ApiConn(harness)) for _ in range(10)]
        for client in clients:
            client.hello()
        for client in clients:
            for _ in range(8):
                client.send("torrents.add", dict(source=dict(url=source), stopped=True))
        case.check(entered.wait(2), "eight-worker pool enters the held real source", str(entered.is_set()))
        for client in clients:
            client.socket.settimeout(.15)
        refusals, acknowledgements = [], []
        for client in clients:
            while True:
                try:
                    value = client.receive()
                except socket.timeout:
                    break
                if value is None:
                    break
                if value.get("ok") is True and value.get("result", {}).get("state") == "pending":
                    acknowledgements.append(value)
                else:
                    refusals.append(value)
        case.check(len(acknowledgements) == 8, "exactly eight dispatch workers start pending URL tasks", str(acknowledgements))
        case.check(refusals and all(frame.get("error", {}).get("code") == "busy"
                   and frame["error"]["details"]["limit"] == "dispatch" for frame in refusals),
                   "eight active workers plus queue 64 refuse additional admitted work with dispatch busy", str(refusals))
        # Owner control is independent of the saturated dispatch pool.
        with ApiConn(harness) as control:
            control.hello()
            control.call("app.shutdown", {})
            case.check(list(control.events())[-1] == dict(ev="bye", reason="shutdown"),
                       "shutdown remains responsive when all dispatch workers and queue are occupied")
        process.wait()


def api_concurrent_adds(binary, systemd):
    import concurrent.futures
    with api_owned("api_concurrent_adds", binary, systemd) as (case, sandbox, harness, process):
        inputs = []
        for name in ("one", "two"):
            directory = sandbox.root / name
            directory.mkdir()
            fixture = fixtures.two_file_fixture(directory)
            inputs.append((dict(source=dict(path=str(fixture.torrent)), save_path=str(directory), stopped=True), fixture.info_hash))
        def add(value):
            with ApiConn(harness) as client:
                client.send("hello", dict(proto=1, client=dict(name="concurrent", version="0", kind="test")))
                assert client.receive()["ok"] is True
                client.send("torrents.add", value)
                return client.receive()
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            requests = [(args, digest) for args, digest in inputs for _ in range(2)]
            futures = [pool.submit(add, args) for args, _ in requests]
            responses = [future.result(timeout=10) for future in futures]
        for (_, digest), response in zip(requests, responses):
            case.check(response.get("ok") is True and response["result"]["hash"] == digest,
                       "concurrent add reply carries its own exact hash", json.dumps(response))
        for _, digest in inputs:
            outcomes = [response["result"]["duplicate"] for response in responses if response["result"]["hash"] == digest]
            case.check(sorted(outcomes) == [False, True], "same-hash concurrent additions have exactly one durable first add", str(outcomes))
        with ApiConn(harness) as client:
            client.hello()
            rows = client.call("torrents.list", {})["result"]["torrents"]
            case.check(sorted(row["hash"] for row in rows) == sorted(digest for _, digest in inputs),
                       "concurrent add ledger has exactly two records", str(rows))


def api_collection_cap(binary, systemd):
    with Case("daemon", "api_collection_cap") as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        # A parked owner exposes real durable records/cached metadata without
        # spawning thousands of metadata resolvers just to fill a response.
        config = Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        config.write_text(config.read_text().replace('network_interface = "lo"', 'network_interface = "fbmissing0"'))
        directory = Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine"
        directory.mkdir(parents=True, exist_ok=True)
        info = dict(name="large-files", **{"piece length": 16384, "pieces": b"x" * 20,
                    "files": [dict(length=1, path=["x" * 512, str(index)]) for index in range(2000)]})
        digest = hashlib.sha1(fixtures.bencode(info)).hexdigest()
        (directory / (digest + ".torrent")).write_bytes(fixtures.bencode(dict(info=info)))
        records = [dict(hash=(digest if index == 0 else f"{index:040x}"), name="x" * 1024,
                        source="magnet:?xt=urn:btih:" + (digest if index == 0 else f"{index:040x}"),
                        output_folder=str(sandbox.downloads), only_files=None, upload_limit=32768,
                        ratio_limit=0.0, category="", tags="", added_on=1, completion_on=0, paused=True)
                   for index in range(2000)]
        (directory / "torrents.json").write_text(json.dumps(dict(records=records)))
        process = harness.start_serve()
        try:
            if harness.receive_ready() and harness.wait_engine():
                with ApiConn(harness) as client:
                    client.hello()
                    for op, args in [("torrents.list", {}), ("torrents.files", dict(hash=digest))]:
                        frame = client.call(op, args, error="invalid_message")
                        case.check(frame["id"] > 0 and frame["error"]["retryable"] is False,
                                   "oversized collection has an ID-bearing typed error", str(frame))
                        client.call("app.info")
                    case.check(not client.pushed, "collection cap preserves the connection without error/bye events", str(client.pushed))
        finally:
            if process.child.poll() is None:
                harness.stop_serve(process)
            harness.lock_free()


def api_start_results(binary, systemd):
    with api_owned("api_start_results", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        client.hello()
        complete_dir = sandbox.root / "complete"
        complete_dir.mkdir()
        payload = complete_dir / "complete.bin"
        payload.write_bytes(os.urandom(4096))
        complete = fixtures.torrent_fixture(complete_dir, [payload])
        client.call("torrents.add", dict(source=dict(path=str(complete.torrent)), save_path=str(complete_dir), seed=True))
        def seeded():
            row = client.call("torrents.get", dict(hash=complete.info_hash))["result"]["torrent"]
            return row["state"] == "seeding", json.dumps(row)
        harness.wait_for(seeded, "real completed start fixture finishes hashing")
        client.call("torrents.set_seed", dict(hashes=[complete.info_hash], on=False))
        refusal = client.call("torrents.start", dict(hashes=[complete.info_hash]), error="invalid_state")
        case.check(refusal["error"]["details"] == dict(state="done", reason="complete_no_seed", hash=complete.info_hash) and
                   refusal["error"]["hint"] == "inspect fbittorrent log" and refusal["error"]["message"] == "invalid state",
                   "completed start preserves the catalogue text and the hash for fbittorrent seed <hash>", str(refusal))
        partial_dir = sandbox.root / "partial"
        partial_dir.mkdir()
        partial = fixtures.two_file_fixture(partial_dir)
        fixtures.corrupt_byte(partial.files[0], 0)
        client.call("torrents.add", dict(source=dict(path=str(partial.torrent)), save_path=str(partial_dir), stopped=True))
        harness.wait_for(lambda: ((row := client.call("torrents.get", dict(hash=partial.info_hash))["result"]["torrent"])["checked"], json.dumps(row)), "partial start fixture finishes hashing")
        first = client.call("torrents.start", dict(hashes=[partial.info_hash]))["result"]
        second = client.call("torrents.start", dict(hashes=[partial.info_hash]))["result"]
        case.check(first == dict(state="done", started=[partial.info_hash], skipped=[]) and
                   second == dict(state="done", started=[], skipped=[dict(hash=partial.info_hash, reason="already_running")]),
                   "first start applies intent and repeated start reports already_running", f"first={first}; second={second}")
        bulk = client.call("torrents.start", dict(hashes=[complete.info_hash, partial.info_hash]))["result"]
        case.check(bulk == dict(state="done", started=[], skipped=[dict(hash=complete.info_hash, reason="complete_no_seed"), dict(hash=partial.info_hash, reason="already_running")]),
                   "bulk start preserves completed and already-running skip reasons", str(bulk))


def offline_ledger_case(binary, systemd, name, remove=False, draining=False):
    with Case("daemon", name) as case, Sandbox() as sandbox, Harness(case,binary,sandbox,systemd) as h:
        select_replay_interface(Path(sandbox.env["XDG_CONFIG_HOME"])/"fbittorrent/config.toml","fbmissing0")
        if not systemd: allow_manager_reads(h)
        elif draining:
            controller = shutil.which("systemctl",path=os.defpath)
            if controller is None: raise RuntimeError("systemd fixture controller missing")
            # This concurrent-down case must permit exactly its owned unit's
            # manager shutdown; the default refusal shim remains for every
            # unrelated unit/action. Retain the calls in the case evidence.
            allowed = "--user:--no-ask-password:show:"+sandbox.unit+"|--user:--no-ask-password:stop:"+sandbox.unit
            fixtures.path_shim("systemctl", 'case "$1:$2:$3:$4" in '+allowed+') exec '+shlex.quote(controller)+' "$@";; *) exit 99;; esac',sandbox.root)
            h.resources.callback(lambda:shutil.copyfile(sandbox.root/"shims/systemctl.log",case.path.with_suffix(".systemctl.log")))
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            fixture=fixtures.two_file_fixture(sandbox.root)
            h.cli("add",str(fixture.torrent))
            ledger=Path(sandbox.env["XDG_DATA_HOME"])/"fbittorrent/engine/torrents.json"
            before=json.loads(ledger.read_text())["records"]
            if draining:
                commands=[[str(binary),"down","--json"],[str(binary),"--no-start","stop",fixture.info_hash,"--json"]]
                children=[case.popen(argv,env=sandbox.env,stdout=subprocess.PIPE,text=True) for argv in commands]
                for argv,child in zip(commands,children):
                    stdout,stderr=child.communicate(timeout=15)
                    case.check(child.returncode==0 and json.loads(stdout)["ok"],"concurrent down/offline stop succeeds without a lost final save",json.dumps(dict(argv=argv,code=child.returncode,stdout=stdout,stderr_log=str(case.daemon_path))))
                process.wait()
            else:
                h.stop_serve(process)
                result=h.cli("delete" if remove else "stop",fixture.info_hash)
                case.check(result.get("running") is False and result.get("count")==1,"offline intent reports its durable result without starting an owner",json.dumps(result))
            saved=json.loads(ledger.read_text())["records"]
            if remove:
                case.check(saved==[] and not (ledger.parent/(fixture.info_hash+".torrent")).exists() and all(p.exists() for p in fixture.files),"offline keep-files deletion removes record/metainfo and retains payload",json.dumps(saved))
            else:
                expected=[dict(row,paused=True) for row in before]
                case.check(saved==expected,"offline stop preserves every record field/order and durably changes only pause intent",json.dumps(saved))
            h.lock_free()
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def offline_stop_ledger(binary,systemd):
    offline_ledger_case(binary,systemd,"offline_stop_ledger")


def offline_delete_keep_files_ledger(binary,systemd):
    offline_ledger_case(binary,systemd,"offline_delete_keep_files_ledger",remove=True)


def offline_write_waits_for_draining_daemon(binary,systemd):
    offline_ledger_case(binary,systemd,"offline_write_waits_for_draining_daemon",draining=True)

def deferred_add_saved_record(binary, systemd):
    with Case("daemon", "deferred_add_saved_record") as case, Sandbox() as sandbox, Harness(case, binary, sandbox, systemd) as h:
        config = Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        select_replay_interface(config, "fbmissing0")
        process = h.start_serve()
        try:
            h.receive_ready(); h.wait_engine()
            with ApiConn(h) as client:
                hello = client.hello()["result"]
                case.check(hello["state"] == "parked" and "deferred_add" in hello["caps"], "missing binding parks the daemon and advertises deferred_add", json.dumps(hello))
                magnet = "magnet:?xt=urn:btih:" + "d1" * 20 + "&dn=Deferred%20fixture&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce"
                reply = client.call("torrents.add", dict(source=dict(magnet=magnet)), ref="deferred-magnet")["result"]
                case.check(reply == dict(state="done", hash="d1" * 20, name="Deferred fixture", duplicate=False, deferred=True), "magnet add acknowledges its durable deferred intent", json.dumps(reply))
                ledger = Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine/torrents.json"
                record = json.loads(ledger.read_text())["records"][-1]
                case.check(record["deferred"] and not record["paused"] and record["source"] == magnet, "physical transfer remains parked while the saved record retains run intent and trackers", json.dumps(record))
                row = client.call("torrents.get", dict(hash=reply["hash"]))["result"]["torrent"]
                case.check(row["state"] == "waiting_vpn" and row["intent"] == "run" and not row["checked"] and "progress" not in row, "deferred row reports waiting_vpn with unknown progress", json.dumps(row))
                fixture = fixtures.two_file_fixture(sandbox.root)
                added = client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), stopped=True, files=[1]))["result"]
                records = json.loads(ledger.read_text())["records"]
                cached = ledger.parent / (fixture.info_hash + ".torrent")
                case.check(added["deferred"] and added["state"] == "done" and records[-1]["paused"] and records[-1]["only_files"] == [1] and cached.read_bytes() == fixture.torrent.read_bytes(), "local metadata and explicit pause/selection are saved at the end of the ledger", json.dumps(dict(reply=added, records=records)))
                case.check(cached.stat().st_mode & 0o777 == 0o600 and ledger.parent.stat().st_mode & 0o777 == 0o700, "deferred store and metainfo stay private", str(cached))
                before = ledger.read_bytes()
                duplicate = client.call("torrents.add", dict(source=dict(magnet=magnet), stopped=True, seed=True))["result"]
                case.check(duplicate["duplicate"] and duplicate["deferred"] and ledger.read_bytes() == before, "duplicate deferred add preserves the original policy and ledger bytes", json.dumps(duplicate))
                with fixtures.HttpFixture(sandbox.root, {"/never.torrent": dict(file=fixture.torrent.name)}) as http:
                    client.call("torrents.add", dict(source=dict(url=http.base + "/never.torrent")), error="engine_unavailable")
                    case.check(not http.requests and ledger.read_bytes() == before, "parked URL add is refused without fetching or saving anything", json.dumps(http.requests))
            h.stop_serve(process)
            h.notify.close()
            process = h.start_serve()
            h.receive_ready(); h.wait_engine()
            with ApiConn(h) as client:
                client.hello()
                row = client.call("torrents.get", dict(hash="d1" * 20))["result"]["torrent"]
                case.check(ledger.read_bytes() == before and row["state"] == "waiting_vpn" and row["intent"] == "run", "daemon restart preserves the deferred records, queue order and saved intent exactly", json.dumps(row))
        finally:
            if process.child.poll() is None: h.stop_serve(process)
            h.lock_free()


def deferred_add(binary, systemd):
    if os.environ.get("FBT_TEST_VPN") != "1":
        raise RuntimeError("deferred_add requires the opt-in VPN namespace fixture (FBT_TEST_VPN=1)")
    with Case("daemon", "deferred_add") as case, Sandbox() as sandbox, Harness(case, binary, sandbox, systemd) as h, Sandbox() as seedbox, Harness(case, binary, seedbox) as seedh:
        select_replay_interface(Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml", "fixture0")
        select_replay_interface(Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml", None)
        activation, activated = sandbox.root / "activate", sandbox.root / "activated.json"
        info = sandbox.root / "namespace.json"
        manifest = sandbox.root / "deferred-namespace.json"
        seed_port = tomllib.loads((Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml").read_text())["peer_port"]
        manifest.write_text(json.dumps(dict(main=DaemonContract.command(str(binary), 300, True), seed=DaemonContract.command(str(binary), 300, True), seed_env=seedbox.env,
            seed_port=seed_port, info=str(info), cleanup=str(sandbox.root / "cleanup.json"), host_netns=os.readlink("/proc/self/ns/net"), activation=str(activation), activated=str(activated))))
        process = h.spawn(["unshare", "-Urn", "--", sys.executable, str(Path(__file__).resolve()), "--replay-namespace-child", str(manifest)])
        try:
            h.wait_engine(); seedh.wait_engine()
            if not h.wait_for(lambda: (info.exists(), str(info)), "owned namespace publishes its tracker"): raise RuntimeError("namespace fixture unavailable")
            network = json.loads(info.read_text())
            payload = seedbox.downloads / "deferred.bin"; payload.write_bytes(os.urandom(8 * fixtures.MIB))
            fixture = fixtures.torrent_fixture(seedbox.root, [payload], tracker=network["tracker"], piece_length=fixtures.MIB)
            # BEP 27 private seeds refuse BEP 9 metadata exchange. This public
            # fixture remains confined to the owned, loopback-only namespace.
            fixture.info["private"] = 0
            fixture.info_hash = hashlib.sha1(fixtures.bencode(fixture.info)).hexdigest()
            fixture.torrent.write_bytes(fixtures.bencode(dict(info=fixture.info, announce=network["tracker"])))
            fixture.magnet = "magnet:?" + urllib.parse.urlencode(dict(xt="urn:btih:" + fixture.info_hash, dn=payload.name, tr=network["tracker"]))
            with ApiConn(seedh) as seed:
                seed.hello(); seed.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), save_path=str(seedbox.downloads), seed=True))
                seedh.wait_for(lambda: ((row := seed.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"])["state"] == "seeding", json.dumps(row)), "real seed finishes checking")
                seed.call("torrents.set_upload_limit", dict(hashes=[fixture.info_hash], bps=32768))
            with ApiConn(h) as client:
                client.hello()
                added = client.call("torrents.add", dict(source=dict(magnet=fixture.magnet)))["result"]
                case.check(added["state"] == "done" and added["deferred"] and added["hash"] == fixture.info_hash, "missing interface saves the real magnet before any engine exists", json.dumps(added))
                before = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
                case.check(before["state"] == "waiting_vpn" and not before["checked"], "record waits without fabricated metadata/progress", json.dumps(before))
                local = fixtures.two_file_fixture(sandbox.root)
                saved = client.call("torrents.add", dict(source=dict(path=str(local.torrent)), stopped=True, files=[1]))["result"]
                case.check(saved["deferred"] and saved["hash"] == local.info_hash, "local metainfo also waits with its explicit paused selection", json.dumps(saved))
                activation.write_text(str(time.time_ns()))
                h.wait_for(lambda: (activated.exists(), str(activated)), "fixture interface appears inside the owned namespace")
                case.check(True, "interface up/down timestamps", activated.read_text())
                def receiving():
                    row = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
                    return row["state"] == "downloading" and row.get("dl_bps", 0) > 0, json.dumps(row)
                h.wait_for(receiving, "deferred metadata resolves and the saved run intent starts receiving", timeout=30)
                records = json.loads((Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine/torrents.json").read_text())["records"]
                record = next(row for row in records if row["hash"] == fixture.info_hash)
                case.check(not record.get("deferred", False) and not record["paused"], "successful activation clears the deferred marker and preserves run intent", json.dumps(record))
                def local_paused():
                    row = client.call("torrents.get", dict(hash=local.info_hash))["result"]["torrent"]
                    return row["state"] == "paused" and row["checked"] and row["intent"] == "pause", json.dumps(row)
                h.wait_for(local_paused, "deferred local metainfo is admitted and checked without starting a paused record")
                files = client.call("torrents.files", dict(hash=local.info_hash))["result"]["files"]
                case.check([row["included"] for row in files] == [False, True], "local selection survives activation", json.dumps(files))
                client.call("app.shutdown", dict(reason="deferred fixture complete")); process.wait()
        finally:
            if process.child.poll() is None: h.stop_serve(process)


def api_missing_completed_payload(binary, systemd):
    with api_owned("api_missing_completed_payload", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        client.hello()
        directory = sandbox.root / "missing-payload-source"
        directory.mkdir()
        payload = directory / "Fixture.bin"
        payload.write_bytes(os.urandom(4096))
        fixture = fixtures.torrent_fixture(directory, [payload])
        client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), save_path=str(sandbox.downloads), stopped=True))
        replay_missing_payload(client, harness, dict(hash=fixture.info_hash, fixture=fixture, pristine=directory), sandbox.downloads)


def api_loopback_seed_receives(binary, systemd):
    with api_owned("api_loopback_seed_receives", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client, \
            Sandbox() as seedbox, Harness(case, binary, seedbox, systemd) as seed_harness:
        seed_process = seed_harness.start_serve()
        if not (seed_harness.receive_ready() and seed_harness.wait_engine()):
            raise RuntimeError("real seed fixture did not start")
        try:
            client.hello()
            directory = sandbox.root / "loopback-payload"
            directory.mkdir()
            payload = directory / "Receive.bin"
            payload.write_bytes(os.urandom(8 * fixtures.MIB))
            fixture = fixtures.torrent_fixture(directory, [payload])
            peer_port = tomllib.loads((Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml").read_text())["peer_port"]
            tracker = fixtures.bencode(dict(interval=1, complete=1, incomplete=0, peers=socket.inet_aton("127.0.0.1") + struct.pack("!H", peer_port)))
            with fixtures.HttpFixture(directory, {"/announce": dict(base64=base64.b64encode(tracker).decode())}) as http:
                fixture.torrent.write_bytes(fixtures.bencode(dict(info=fixture.info, announce=http.base + "/announce")))
                item = dict(hash=fixture.info_hash, fixture=fixture, pristine=directory)
                harness.seed_harness, harness.replay_seedbox = seed_harness, seedbox
                client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), save_path=str(sandbox.downloads), stopped=True))
                start_replay_seed(client, harness, item)
                client.call("torrents.start", dict(hashes=[fixture.info_hash]))
                frame = receive_replay_sample(client, item, dict(id=50000, op="torrents.get", args=dict(hash=fixture.info_hash)))
                row = frame["result"]["torrent"]
                case.check(row["state"] == "downloading" and row.get("dl_bps", 0) > 0 and 0 < row["completed_bytes"] < row["size"],
                           "real 8 MiB loopback payload is receiving at sample time", json.dumps(row))
        finally:
            seed_harness.stop_serve(seed_process)
            seed_harness.lock_free()


def validate_live_contract(case, sandbox, frames, kind="live", optional=()):
    """Use P1-05's exact Rust decoder, not a Python schema approximation."""
    trace = case.path.with_suffix("." + kind + ".ndjson")
    trace.write_text("".join(json.dumps(frame, separators=(",", ":")) + "\n" for frame in frames))
    validator = os.environ.get("FBT_API_CONTRACT_VALIDATOR")
    if not validator or not Path(validator).is_file():
        raise RuntimeError("contract replay requires --contract-validator (cargo test --no-run's fbittorrent test executable)")
    output = case.path.with_suffix("." + kind + ".validation.log")
    environment = dict(sandbox.env, FBT_API_CONTRACT_TRACE=str(trace), FBT_API_CONTRACT_KIND="fixture" if kind == "fixture" else "live")
    if optional:
        optional_path = case.path.with_suffix(".extra-optional.json")
        optional_path.write_text(json.dumps(optional, indent=2, sort_keys=True) + "\n")
        environment["FBT_API_CONTRACT_OPTIONALS"] = str(optional_path)
    with output.open("w") as log:
        child = case.popen([validator, "--exact", "api::server::argument_tests::validate_live_transcript",
                            "--ignored", "--nocapture"], env=environment, stdout=log)
        result = child.wait(timeout=10)
    step = case.step("validate captured live frames with the frozen Rust contract", "all frames pass P1-05 types, bounds, capabilities and canonical round trips")
    step.close(result == 0, f"Rust validator exit={result}; transcript={trace}.", [trace, output])
    return result == 0


# The only live-replay normalization specification. Docs and the receipt copy
# normalization_markdown() verbatim; no fixture file or hidden mask is changed.
REPLAY_NORMALIZATION = (
    ("/id", "identity:request", "positive u64, consistent first-occurrence bijection"),
    ("/result/instance", "identity:instance", "32 lowercase hex, consistent first-occurrence bijection"),
    ("/**/hash; /result/started/*; /result/stopped/*", "identity:hash", "40 lowercase hex; API-generated hashes bound to fixture identities"),
    ("/ref; /result/ref", "identity:ref", "at most 64 ASCII characters, consistent first-occurrence bijection"),
    ("/ts; /result/torrent/added_at; /result/torrent/completed_at; /result/torrents/*/added_at; /result/torrents/*/completed_at", "timestamp", "positive u64; presence preserved"),
    ("/result/dl_bps; /result/ul_bps; /result/torrent/dl_bps; /result/torrent/ul_bps; /result/torrents/*/dl_bps; /result/torrents/*/ul_bps", "u64", "nonnegative u64; presence preserved"),
    ("/result/torrent/eta_s; /result/torrents/*/eta_s; /result/torrent/peers; /result/torrent/seeds; /result/torrents/*/peers; /result/torrents/*/seeds", "u64", "nonnegative u64; presence preserved"),
    ("/result/session_downloaded; /result/session_uploaded; /result/uptime_s; /result/clients/*", "u64", "nonnegative u64; presence preserved"),
    ("/result/server/pid", "timestamp", "positive u64; presence preserved"),
    ("/result/server/build", "text", "nonempty string; presence preserved (declared fixture volatile path)"),
    ("/result/server/version", "version", "semantic version string; presence preserved"),
    ("/result/torrent/completed_bytes; /result/torrent/progress; /result/torrents/*/completed_bytes; /result/torrents/*/progress", "checking only", "live state=checking: u64 completed_bytes <= size, finite progress=completed_bytes/size within 1e-9 absolute; size=0 requires both zero; presence preserved; other states compare exactly. Checking hashes exclude these measurements from live-history agreement, retaining size/file totals"),
    ("object member absent in fixture", "extra optional field", "live value validates against P1-05; removing the member also validates; log every pointer (ruling 4C)"),
    ("all other JSON pointers", "exact", "canonical bytes through the P1-05 types; no extra masks or value substitution"),
)


def normalization_markdown():
    return "| JSON pointer(s) | Comparison | Validity |\n|---|---|---|\n" + "".join(
        f"| `{paths}` | {kind} | {rule} |\n" for paths, kind, rule in REPLAY_NORMALIZATION)


class ReplayMismatch(ValueError):
    pass


class ReplayNormalizer:
    def __init__(self):
        self.identities = {kind: {} for kind in ("request", "instance", "hash", "ref")}
        self.extra_optional = []

    def bind(self, kind, fixture, live, pointer):
        valid = (type(live) is int and 0 < live < 2**64) if kind == "request" else (
            isinstance(live, str) and
            (bool(re.fullmatch(r"[0-9a-f]{32}", live)) if kind == "instance" else
             bool(re.fullmatch(r"[0-9a-f]{40}", live)) if kind == "hash" else
             live.isascii() and len(live) <= 64))
        if not valid:
            raise ReplayMismatch(f"JSON pointer {pointer}: invalid live {kind} identity {live!r}")
        mapping = self.identities[kind]
        if fixture in mapping and mapping[fixture] != live:
            raise ReplayMismatch(f"JSON pointer {pointer}: inconsistent {kind} mapping {fixture!r}: {mapping[fixture]!r} then {live!r}")
        if fixture not in mapping and live in mapping.values():
            raise ReplayMismatch(f"JSON pointer {pointer}: two fixture {kind} identities alias {live!r}")
        mapping[fixture] = live
        return live

    def request(self, frame):
        frame = json.loads(json.dumps(frame))
        fixture_id = frame["id"]
        mapping = self.identities["request"]
        if type(fixture_id) is int and 0 < fixture_id < 2**64:
            frame["id"] = self.bind("request", fixture_id, mapping.get(fixture_id, 1000 + len(mapping)), "/id")
        def rewrite(value, key=""):
            if isinstance(value, dict):
                return {k: rewrite(v, k) for k, v in value.items()}
            if isinstance(value, list):
                return [rewrite(v, key) for v in value]
            if isinstance(value, str):
                if key == "ref":
                    return self.bind("ref", value, self.identities["ref"].get(value, "live-ref-" + str(len(self.identities["ref"]))), "/ref")
                for old, new in self.identities["hash"].items():
                    value = value.replace(old, new)
                return value
            return value
        return rewrite(frame)

    @staticmethod
    def matches(pattern, pointer):
        if pattern == "/**/hash":
            return pointer.endswith("/hash")
        return re.fullmatch(re.escape(pattern).replace(r"\*", "[^/]+"), pointer) is not None

    def compare(self, expected, live):
        def walk(wanted, got, pointer="", checking=False):
            if checking:
                return got  # The complete checking tuple was validated below.
            rule = next((kind for patterns, kind, _ in REPLAY_NORMALIZATION
                         for pattern in patterns.split("; ") if self.matches(pattern, pointer)), "exact")
            if rule == "checking only":
                rule = "exact"
            if rule.startswith("identity:"):
                if wanted is None:  # Failure.id is the single nullable wire field.
                    if got is not None:
                        raise ReplayMismatch(f"JSON pointer {pointer}: expected null, observed {got!r}")
                    return None
                return self.bind(rule.split(":")[1], wanted, got, pointer)
            if rule != "exact":
                valid = ((type(got) is int and 0 <= got < 2**64) if rule == "u64" else
                         (type(got) is int and 0 < got < 2**64) if rule == "timestamp" else
                         (isinstance(got, str) and bool(got)) if rule == "text" else
                         (isinstance(got, str) and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", got)))
                if not valid:
                    raise ReplayMismatch(f"JSON pointer {pointer}: invalid {rule}: {got!r}")
                return got
            if type(wanted) is not type(got):
                raise ReplayMismatch(f"JSON pointer {pointer or '/'}: expected {wanted!r}, observed {got!r}")
            if isinstance(wanted, dict):
                for key in sorted(wanted):
                    here = pointer + "/" + key.replace("~", "~0").replace("/", "~1")
                    if key not in got:
                        raise ReplayMismatch(f"JSON pointer {here}: expected {wanted.get(key, '<missing>')!r}, observed {got.get(key, '<missing>')!r}")
                is_torrent = pointer == "/result/torrent" or re.fullmatch(r"/result/torrents/[0-9]+", pointer)
                active_check = is_torrent and got.get("state") == "checking"
                if active_check and any(key in wanted for key in ("completed_bytes", "progress")):
                    size, completed, progress = (got.get(key) for key in ("size", "completed_bytes", "progress"))
                    if type(size) is not int or not 0 <= size < 2**64:
                        raise ReplayMismatch(f"JSON pointer {pointer}/size: checking requires a nonnegative u64 size")
                    if type(completed) is not int or not 0 <= completed <= size:
                        raise ReplayMismatch(f"JSON pointer {pointer}/completed_bytes: checking requires 0 <= completed_bytes <= size")
                    ratio = completed / size if size else 0.0
                    if type(progress) not in (int, float) or not math.isfinite(progress) or not 0 <= progress <= 1 or abs(progress - ratio) > 1e-9:
                        raise ReplayMismatch(f"JSON pointer {pointer}/progress: checking progress must equal completed_bytes/size within 1e-9")
                for key in sorted(got.keys() - wanted.keys()):
                    here = pointer + "/" + key.replace("~", "~0").replace("/", "~1")
                    self.extra_optional.append((here, got[key]))
                result = {key: walk(value, got[key], pointer + "/" + key.replace("~", "~0").replace("/", "~1"),
                                    active_check and key in ("completed_bytes", "progress")) for key, value in wanted.items()}
                result.update((key, value) for key, value in got.items() if key not in wanted)
                return result
            if isinstance(wanted, list):
                if len(wanted) != len(got):
                    raise ReplayMismatch(f"JSON pointer {pointer}: expected {len(wanted)} items, observed {len(got)}")
                return [walk(a, b, pointer + "/" + str(index)) for index, (a, b) in enumerate(zip(wanted, got))]
            if wanted != got:
                raise ReplayMismatch(f"JSON pointer {pointer}: expected {wanted!r}, observed {got!r}")
            return wanted
        normalized = walk(expected, live)
        # The Rust check_group bridge also requires lossless P1-05 typed
        # canonical round trips of the captured frames; this compares its bytes.
        canonical = lambda value: json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        if canonical(normalized) != canonical(live):
            raise ReplayMismatch("JSON pointer /: canonical byte comparison failed")


def replay_namespace_child(manifest_path):
    """VPN fixture pattern: real seed, tracker and daemon in one owned netns."""
    manifest = json.loads(Path(manifest_path).read_text())
    if os.readlink("/proc/self/ns/net") == manifest["host_netns"]:
        raise RuntimeError("refusing replay interface mutations outside the owned network namespace")
    activation = manifest.get("activation")
    if activation:
        subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    else:
        subprocess.run(["ip", "link", "set", "lo", "name", "fixture0"], check=True)
        subprocess.run(["ip", "link", "set", "fixture0", "up"], check=True)
    compact = socket.inet_aton("127.0.0.1") + struct.pack("!H", manifest["seed_port"])
    tracker = fixtures.HttpFixture(Path(manifest_path).parent, {
        "/announce": dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1, complete=1, incomplete=0, peers=compact))).decode())})
    children = []
    def terminate(*_):
        for child in children:
            if child.poll() is None:
                child.terminate()
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    control_stop, control = threading.Event(), None
    def activate():
        while not control_stop.wait(.05):
            if Path(activation).exists():
                down = time.time_ns()
                subprocess.run(["ip", "link", "set", "lo", "down"], check=True)
                subprocess.run(["ip", "link", "set", "lo", "name", "fixture0"], check=True)
                subprocess.run(["ip", "link", "set", "fixture0", "up"], check=True)
                Path(manifest["activated"]).write_text(json.dumps(dict(down_ns=down, up_ns=time.time_ns(), namespace=os.readlink("/proc/self/ns/net"))))
                return
    try:
        seed = subprocess.Popen(manifest["seed"], env=manifest["seed_env"], stdin=subprocess.DEVNULL)
        children.append(seed)
        main = subprocess.Popen(manifest["main"], stdin=subprocess.DEVNULL)
        children.append(main)
        Path(manifest["info"]).write_text(json.dumps(dict(namespace=os.readlink("/proc/self/ns/net"), tracker=tracker.base + "/announce",
            seed_port=manifest["seed_port"], seed_pid=seed.pid, main_pid=main.pid), sort_keys=True))
        if activation:
            control = threading.Thread(target=activate)
            control.start()
        return main.wait()
    finally:
        control_stop.set()
        if control is not None:
            control.join(timeout=2)
            if control.is_alive(): raise RuntimeError("namespace activation thread did not stop")
        terminate()
        exits = []
        for child in children:
            try:
                exits.append(child.wait(timeout=10))
            except subprocess.TimeoutExpired:
                child.kill(); child.wait()
                raise RuntimeError("owned replay child required SIGKILL")
        tracker.close()
        Path(manifest["cleanup"]).write_text(json.dumps(dict(exits=exits, tracker_closed=tracker.closed), sort_keys=True))


@contextmanager
def replay_owned(group, binary, systemd):
    with Case("daemon", "contract_" + group) as case, Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        payload_root = None
        if group == "torrents":
            lock_path = Path("/tmp/fbt-contract-replay-" + str(os.geteuid()) + ".lock")
            descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
            lock = harness.resources.enter_context(os.fdopen(descriptor, "r+"))
            if os.fstat(lock.fileno()).st_uid != os.geteuid():
                raise RuntimeError("replay lock has a foreign owner")
            def acquire():
                try:
                    fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                    return True, str(lock_path)
                except BlockingIOError:
                    return False, "another literal-path replay owns the lock"
            if not harness.wait_for(acquire, "exclusive literal fixture-path ownership"):
                raise RuntimeError("literal fixture-path replay is busy")
            # Match literal fixture paths, without ever adopting an existing
            # directory. Cleanup is contingent on proving owned children gone.
            payload_root = Path("/tmp/fbt-fixture")
            payload_root.mkdir(mode=0o700)
            owned_inode = payload_root.stat().st_ino
        process = None
        try:
            command = DaemonContract.command(str(binary), 300, True)
            if group == "torrents":
                config = Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
                select_replay_interface(config, "fixture0" if os.environ.get("FBT_TEST_VPN") == "1" else None)
                config.write_text(config.read_text() + '\nmax_active_downloads = 1\n')
                seedbox = harness.resources.enter_context(Sandbox())
                seedconfig = Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
                select_replay_interface(seedconfig, None)
                if tomllib.loads(seedconfig.read_text())["peer_port"] == tomllib.loads(config.read_text())["peer_port"]:
                    with socket.socket() as reservation:
                        reservation.bind(("127.0.0.1", 0))
                        port = reservation.getsockname()[1]
                    seedconfig.write_text(re.sub(r'^peer_port = .*$', 'peer_port = ' + str(port), seedconfig.read_text(), flags=re.M))
                harness.seed_harness = harness.resources.enter_context(Harness(case, binary, seedbox))
                harness.replay_seedbox = seedbox
                manifest = sandbox.root / "replay-namespace.json"
                harness.replay_namespace_info = sandbox.root / "replay-namespace-info.json"
                harness.replay_namespace_cleanup = sandbox.root / "replay-namespace-cleanup.json"
                manifest.write_text(json.dumps(dict(main=command, seed=DaemonContract.command(str(binary), 300, True),
                    seed_env=seedbox.env, seed_port=tomllib.loads(seedconfig.read_text())["peer_port"],
                    info=str(harness.replay_namespace_info), cleanup=str(harness.replay_namespace_cleanup), host_netns=os.readlink("/proc/self/ns/net"))))
                command = ["unshare", "-Urn", "--", sys.executable, str(Path(__file__).resolve()), "--replay-namespace-child", str(manifest)]
            # No NOTIFY_SOCKET is injected: the hello fixture presupposes the
            # documented foreground identity. All launches still use Harness.
            process = harness.spawn(command)
            if not harness.wait_engine():
                raise RuntimeError("replay daemon did not answer")
            if group == "torrents":
                if not harness.wait_for(lambda: (harness.replay_namespace_info.exists(), str(harness.replay_namespace_info)), "owned namespace seed/tracker manifest exists"):
                    raise RuntimeError("replay namespace did not publish fixture endpoints")
                harness.replay_network = json.loads(harness.replay_namespace_info.read_text())
                if not harness.seed_harness.wait_engine():
                    raise RuntimeError("real loopback seed daemon did not answer")
                shutil.copyfile(harness.replay_namespace_info, case.path.with_suffix(".namespace.json"))
                case.check(True, "VPN namespace fixture availability", json.dumps(dict(opt_in=os.environ.get("FBT_TEST_VPN") == "1", **harness.replay_network), sort_keys=True))
            with ApiConn(harness) as ready:
                ready.hello()
                harness.wait_for(lambda: ((info := ready.call("app.info")["result"])["state"] == "running", json.dumps(info)), "replay owner is running before transcript")
            yield case, sandbox, harness, process, payload_root
        finally:
            if process is not None and process.child.poll() is None:
                if harness.engine_path.exists():
                    harness.stop_serve(process)
                else:
                    process.wait()
            if process is not None:
                harness.lock_free()
            if group == "torrents" and hasattr(harness, "replay_namespace_cleanup") and harness.replay_namespace_cleanup.exists():
                cleanup = json.loads(harness.replay_namespace_cleanup.read_text())
                case.check(cleanup["exits"] == [0, 0] and cleanup["tracker_closed"], "real seed, tracker and main daemon exit with owned namespace", str(cleanup))
                harness.seed_harness.lock_free()
                shutil.copyfile(harness.replay_namespace_cleanup, case.path.with_suffix(".namespace-cleanup.json"))
            if payload_root is not None:
                live = process is not None and (process.child.poll() is None or process.group_members())
                if live:
                    sandbox.retain()
                    case.check(False, "retain payloads if owned process cleanup is unproved", str(payload_root))
                elif payload_root.is_dir() and not payload_root.is_symlink() and payload_root.stat().st_ino == owned_inode:
                    shutil.rmtree(payload_root)


def replay_preconditions(lines, client, normalizer, root):
    rows, details, file_lists = {}, {}, {}
    for item in lines:
        result = item.get("frame", {}).get("result", {})
        for row in result.get("torrents", []):
            rows.setdefault(row["hash"], row)
        if "torrent" in result:
            row = result["torrent"]
            rows.setdefault(row["hash"], row)
            details.setdefault(row["hash"], row)
    previous = None
    for item in lines:
        if item["dir"] == "c2s": previous = item["frame"]
        elif item["dir"] == "s2c" and previous and previous["op"] == "torrents.files":
            file_lists[previous["args"]["hash"]] = item["frame"].get("result", {}).get("files", [])
    root.mkdir(exist_ok=True)
    prepared = {}
    for fixture_hash, row in rows.items():
        detail = details.get(fixture_hash, row)
        directory = root / fixture_hash
        directory.mkdir()
        files = file_lists.get(fixture_hash)
        if files:
            specs = [(file["path"], file["size"], file["progress"]) for file in files]
        else:
            count = detail.get("file_count", 1)
            total = detail.get("size", row.get("size", 4096))
            specs = [(row["name"] if count == 1 else row["name"] + f" {index + 1}.bin",
                      total // count + (index < total % count), row.get("progress", 0.0)) for index in range(count)]
        if files and "size" in row:
            client.case.check(row["size"] == sum(size for _, size, _ in specs), "fixture total matches its real file lengths",
                              f"JSON pointer /preconditions/{fixture_hash}/size: torrent size={row['size']}; sum of file sizes={sum(size for _, size, _ in specs)}")
        payloads = []
        for name, size, _ in specs:
            path = directory / name
            if not path.resolve().is_relative_to(directory.resolve()):
                raise RuntimeError("fixture payload path escapes its owned root")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(os.urandom(size))
            payloads.append(path)
        pristine = directory / "pristine"
        pristine.mkdir()
        for payload in payloads:
            shutil.copyfile(payload, pristine / payload.name)
        # A receiving frame can still have zero verified bytes: use a larger
        # piece when its geometry is unspecified, so real incoming chunks raise
        # the measured rate before the first piece finishes verification.
        default_piece = fixtures.MIB if row["state"] == "downloading" and row.get("completed_bytes") == 0 else 1024
        fixture = fixtures.torrent_fixture(directory, payloads, name=row["name"], piece_length=detail.get("piece_size", default_piece))
        client.case.check(True, "record generated transcript payload geometry",
                          json.dumps(dict(fixture_hash=fixture_hash, bytes=sum(size for _, size, _ in specs),
                                          piece_size=fixture.info["piece length"]), sort_keys=True))
        fixture.info["private"] = int(detail.get("private", False))
        document = dict(info=fixture.info)
        if row["state"] == "downloading":
            document["announce"] = client.harness.replay_network["tracker"]
        for key, wire_key in (("comment", "comment"), ("created_by", "created by")):
            if key in detail: document[wire_key] = detail[key]
        fixture.torrent.write_bytes(fixtures.bencode(document))
        fixture.info_hash = hashlib.sha1(fixtures.bencode(fixture.info)).hexdigest()
        fixture.magnet = "magnet:?" + urllib.parse.urlencode(dict(xt="urn:btih:" + fixture.info_hash, dn=row["name"]))
        for payload, (_, size, progress) in zip(payloads, specs):
            keep = int(size * progress)
            with payload.open("r+b") as output:
                for offset in range(keep, size, fixture.info["piece length"]):
                    output.seek(offset); value = output.read(1)
                    output.seek(offset); output.write(bytes([value[0] ^ 1]))
        source = dict(magnet=fixture.magnet) if row["state"] == "metadata" else dict(path=str(fixture.torrent))
        reply = client.call("torrents.add", dict(source=source, save_path="/tmp/fbt-fixture/downloads", stopped=True, origin="manual", seed=row["seed"]))
        if not reply.get("ok"):
            raise RuntimeError("precondition add failed")
        actual_hash = reply["result"]["hash"]
        client.case.check(actual_hash == fixture.info_hash, "API add identity matches the generator's actual metainfo hash", f"expected={fixture.info_hash}; observed={actual_hash}")
        normalizer.bind("hash", fixture_hash, actual_hash, "/preconditions/hash")
        prepared[fixture_hash] = dict(row=row, fixture=fixture, hash=actual_hash, specs=specs, pristine=pristine)
        # Establish files using the API's save_path, separate from the pristine
        # source/generator directory. These are real payloads, never ledger edits.
        for payload in payloads:
            destination = root / "downloads" / payload.name
            destination.parent.mkdir(exist_ok=True)
            shutil.copyfile(payload, destination)
        if row["state"] != "metadata":
            # The add may already have verified missing payloads; remove and
            # re-add after copying so the actual hash check sees these bytes.
            client.call("torrents.remove", dict(hashes=[actual_hash], delete_files=False))
            client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), save_path="/tmp/fbt-fixture/downloads", stopped=True, origin="manual", seed=row["seed"]))
            def checked():
                result = client.call("torrents.get", dict(hash=actual_hash))["result"]["torrent"]
                return result["checked"], json.dumps(result)
            client.harness.wait_for(checked, "real precondition payload finishes hashing")
        if row["state"] in ("done", "seeding"):
            client.call("torrents.set_seed", dict(hashes=[actual_hash], on=True))
            client.call("torrents.start", dict(hashes=[actual_hash]))
            if row["state"] == "done": client.call("torrents.set_seed", dict(hashes=[actual_hash], on=False))
        elif row["intent"] == "run":
            client.call("torrents.start", dict(hashes=[actual_hash]))
        observed = client.call("torrents.get", dict(hash=actual_hash))["result"]["torrent"]
        client.case.check(True, "record real transcript preconditions and observed state",
                          json.dumps(dict(fixture_hash=fixture_hash, live_hash=actual_hash, expected_state=row["state"], observed=observed), sort_keys=True))
    return prepared


def replay_missing_payload(client, harness, item, save_path):
    """Complete through the engine, unlink real payloads, restart, then activate."""
    digest, fixture = item["hash"], item["fixture"]
    client.call("torrents.remove", dict(hashes=[digest], delete_files=False))
    for payload in fixture.files:
        shutil.copyfile(item["pristine"] / payload.name, save_path / payload.name)
    client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), save_path=str(save_path), seed=True))
    harness.wait_for(lambda: ((row := client.call("torrents.get", dict(hash=digest))["result"]["torrent"])["state"] == "seeding", json.dumps(row)), "missing-payload fixture is actually completed and seeding")
    client.call("torrents.set_seed", dict(hashes=[digest], on=False))
    for payload in fixture.files:
        (save_path / payload.name).unlink()
    epoch = client.call("app.info")["result"]["engine_epoch"]
    client.call("app.restart", dict(scope="engine"))
    harness.wait_for(lambda: ((info := client.call("app.info")["result"])["engine_epoch"] > epoch and info["state"] == "running", json.dumps(info)), "completed missing-payload record restarts without admitting data")
    dormant = client.call("torrents.get", dict(hash=digest))["result"]["torrent"]
    client.case.check(dormant["state"] == "done" and not dormant["checked"], "restart retains the unverified dormant completion claim before explicit activation", json.dumps(dormant))
    client.call("torrents.set_seed", dict(hashes=[digest], on=True), error="invalid_state")
    row = client.call("torrents.get", dict(hash=digest))["result"]["torrent"]
    client.case.check(row["state"] == "error" and row["reason"] == "missing_files" and
                      row["error"] == row["name"] + " restore failed: missing files" and "completed_at" not in row,
                      "missing payload activation has canonical text/reason and clears stale completion", json.dumps(row))


def start_replay_seed(client, harness, item):
    seedbox = harness.replay_seedbox
    for payload in item["fixture"].files:
        shutil.copyfile(item["pristine"] / payload.name, seedbox.downloads / payload.name)
    with ApiConn(harness.seed_harness) as seed:
        seed.hello()
        added = seed.call("torrents.add", dict(source=dict(path=str(item["fixture"].torrent)), save_path=str(seedbox.downloads), seed=True))
        client.case.check(added["result"]["hash"] == item["hash"], "real loopback seed and client use identical metainfo", str(added))
        harness.wait_for(lambda: ((row := seed.call("torrents.get", dict(hash=item["hash"]))["result"]["torrent"])["state"] == "seeding", json.dumps(row)), "loopback seed finishes its real checksum check")
        seed.call("torrents.set_upload_limit", dict(hashes=[item["hash"]], bps=32768))


def receive_replay_sample(client, item, query, timeout=8):
    began, attempt, frame = time.monotonic(), 0, None
    while time.monotonic() - began < timeout:
        attempt += 1
        client.send_frame(query)
        frame = client.response(query["id"])
        row = frame["result"]["torrent"]
        client.case.check(True, "bounded real loopback receive/query attempt", json.dumps(dict(attempt=attempt, elapsed_ms=round((time.monotonic()-began)*1000), torrent=row), sort_keys=True))
        if (row["state"] == "downloading" and row.get("dl_bps", 0) > 0) or row["state"] == "done":
            break
        time.sleep(.05)
    return frame


def drive_replay_state(client, prepared, fixture_hash, harness, query):
    """Establish each detail scenario using real intent/restart operations."""
    item = prepared[fixture_hash]
    row, digest = item["row"], item["hash"]
    desired = row["state"]
    available = client.call("torrents.list", {})["result"]["torrents"]
    live_hashes = {value["hash"] for value in available}
    if digest not in live_hashes:
        client.case.check(True, "ruling 4E defers the removed-record detail precondition to P6-08", f"JSON pointer /result/torrent/hash: {fixture_hash} was removed earlier; no silent re-add")
        return None
    if desired in ("queued", "stalled", "downloading"):
        for other in prepared.values():
            if other["hash"] != digest and other["hash"] in live_hashes:
                client.call("torrents.stop", dict(hashes=[other["hash"]]))
        if desired == "queued":
            # Prior checking scenarios may have re-added the earlier blocker.
            # Put this target behind it in the real FIFO via remove/add, never
            # by changing ledger order or fabricating queue_pos.
            client.call("torrents.remove", dict(hashes=[digest], delete_files=False))
            client.call("torrents.add", dict(source=dict(path=str(item["fixture"].torrent)), save_path="/tmp/fbt-fixture/downloads", stopped=True))
            harness.wait_for(lambda: ((value := client.call("torrents.get", dict(hash=digest))["result"]["torrent"])["checked"], json.dumps(value)), "real queued target completes its initial check")
            blocker = next(other for other in prepared.values() if other["hash"] != digest and other["hash"] in live_hashes and 0 < other["row"].get("progress", 0) < 1)
            client.call("torrents.start", dict(hashes=[blocker["hash"]]))
        if desired == "downloading":
            start_replay_seed(client, harness, item)
            # Re-admit so the real tracker/seed is available on the first dial.
            client.call("torrents.set_files", dict(hash=digest, include=list(range(len(item["specs"])))))
        client.call("torrents.start", dict(hashes=[digest]))
        if desired == "downloading":
            return receive_replay_sample(client, item, query)
        if desired == "queued":
            harness.wait_for(lambda: ((value := client.call("torrents.get", dict(hash=digest))["result"]["torrent"])["state"] == "queued", json.dumps(value)), "FIFO queue precondition is actually queued")
    elif desired == "paused":
        client.call("torrents.stop", dict(hashes=[digest]))
    elif desired == "metadata":
        client.call("torrents.start", dict(hashes=[digest]))
    elif desired == "checking":
        # SetFiles admission performs a real recheck; the frozen v1 table has
        # no recheck operation. Retain the immediate response, never re-query it.
        frame = None
        for attempt in range(1, 4):
            client.call("torrents.set_files", dict(hash=digest, include=list(range(len(item["specs"])))))
            client.send_frame(query)
            frame = client.response(query["id"])
            sample = frame["result"]["torrent"]
            client.case.check(True, "bounded real recheck/query attempt", json.dumps(dict(attempt=attempt, size=sample.get("size"), torrent=sample), sort_keys=True))
            if sample["state"] == "checking":
                break
        return frame
    elif desired == "error":
        replay_missing_payload(client, harness, item, Path("/tmp/fbt-fixture/downloads"))
    elif desired == "waiting_vpn":
        client.call("torrents.start", dict(hashes=[digest]))
        config = Path(harness.sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        harness.replay_network_config = config.read_text()
        select_replay_interface(config, "fbmissing0")
        client.call("app.restart", dict(scope="engine"))
        harness.wait_for(lambda: ((info := client.call("app.info")["result"])["state"] == "parked", json.dumps(info)), "missing private fixture interface parks the real owner")
    return None


def select_replay_interface(config, interface):
    value = re.sub(r'^network_interface = .*\n?', '', config.read_text(), flags=re.M)
    config.write_text(value + ('\nnetwork_interface = ' + json.dumps(interface) + '\n' if interface else ''))


def restore_replay_interface(client, harness):
    config = Path(harness.sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
    config.write_text(harness.replay_network_config)
    client.call("app.restart", dict(scope="engine"))
    harness.wait_for(lambda: ((info := client.call("app.info")["result"])["state"] == "running", json.dumps(info)), "private fixture interface selection restarts the real owner")


def api_replay_normalization(binary, systemd):
    with Case("daemon", "api_replay_normalization") as case:
        normalizer = ReplayNormalizer()
        normalizer.compare(dict(id=1, result=dict(instance="0" * 32, server=dict(version="0.2.0"))),
                           dict(id=123, result=dict(instance="a" * 32, server=dict(version="0.1.0"))))
        case.check(True, "identity and version normalization accepts valid first occurrences", json.dumps(normalizer.identities, sort_keys=True))
        for label, expected, actual, pointer in [
            ("inconsistent identity", dict(id=1), dict(id=124), "/id"),
            ("invalid volatile", dict(result=dict(dl_bps=1)), dict(result=dict(dl_bps=-1)), "/result/dl_bps"),
            ("changed nonvolatile", dict(result=dict(checked=True)), dict(result=dict(checked=False)), "/result/checked"),
            ("missing volatile", dict(result=dict(dl_bps=1)), dict(result={}), "/result/dl_bps"),
        ]:
            try:
                normalizer.compare(expected, actual)
                case.check(False, label + " fails comparison", "unexpected match")
            except ReplayMismatch as error:
                case.check(pointer in str(error), label + " fails at its exact pointer", str(error))
        watch_frame = dict(result=dict(torrent=dict(playable=dict(ready=True))))
        pending_frame = dict(result=dict(state="pending"))
        undo_frame = dict(result=dict(torrent=dict(undo_until=123)))
        for frame, op, cap in ((watch_frame, "torrents.get", "watch"),
                               (pending_frame, "torrents.remove", "mutations"),
                               (undo_frame, "torrents.get", "origin")):
            absent = replay_frame_deferrals(frame, op)
            present = replay_frame_deferrals(frame, op, {cap})
            case.check(bool(absent) and not present, "advertised capability keeps the frame under active comparison",
                       json.dumps(dict(cap=cap, absent=absent, present=present)))
        checking = dict(state="checking", checked=False, size=100, completed_bytes=50, progress=.5)
        sample = dict(checking, completed_bytes=25, progress=.25)
        for wrap in (lambda row: dict(result=dict(torrent=row)), lambda row: dict(result=dict(torrents=[row]))):
            normalizer.compare(wrap(checking), wrap(sample))
            case.check(True, "checking-only measurement normalization accepts a valid live tuple", json.dumps(wrap(sample)))
        zero = dict(checking, size=0, completed_bytes=0, progress=0.0)
        normalizer.compare(dict(result=dict(torrent=zero)), dict(result=dict(torrent=zero)))
        normalizer.compare(dict(result=dict(torrent=checking)), dict(result=dict(torrent=dict(sample, progress=.25 + 1e-10))))
        case.check(True, "zero size and the declared 1e-9 tolerance are accepted", "zero/zero=0; in-tolerance tuple passes")
        for label, wanted, got, pointer in [
            ("over-size checked bytes", checking, dict(sample, completed_bytes=101), "/completed_bytes"),
            ("negative checked bytes", checking, dict(sample, completed_bytes=-1), "/completed_bytes"),
            ("boolean checked bytes", checking, dict(sample, completed_bytes=True), "/completed_bytes"),
            ("nonfinite progress", checking, dict(sample, progress=float("nan")), "/progress"),
            ("inconsistent progress", checking, dict(sample, progress=.5), "/progress"),
            ("missing checked bytes", checking, {k: v for k, v in sample.items() if k != "completed_bytes"}, "/completed_bytes"),
            ("changed static size", checking, dict(sample, size=200, progress=.125), "/size"),
            ("changed checked flag", checking, dict(sample, checked=True), "/checked"),
            ("paused is exact", dict(checking, state="paused"), dict(sample, state="paused"), "/completed_bytes"),
            ("downloading is exact", dict(checking, state="downloading"), dict(sample, state="downloading"), "/completed_bytes"),
        ]:
            try:
                normalizer.compare(dict(result=dict(torrent=wanted)), dict(result=dict(torrent=got)))
                case.check(False, label + " fails comparison", "unexpected match")
            except ReplayMismatch as error:
                case.check(pointer in str(error), label + " fails at its exact pointer", str(error))


def replay_starting_window(client, harness, identifier, frames):
    """Bounded real READY-window probes; never delay or forge engine startup."""
    attempts = []
    actual = None
    evidence = client.case.path.with_suffix(".starting-retries.json")
    for attempt in range(1, 4):
        with Sandbox() as sandbox, Harness(client.case, harness.binary, sandbox, harness.systemd) as probe:
            probe.notify = probe.notify_socket()
            process = probe.spawn(DaemonContract.command(harness.binary, 300, True), name="starting_attempt_" + str(attempt))
            try:
                if not probe.receive_ready():
                    raise RuntimeError("starting-window probe did not receive READY")
                ready_at = time.monotonic()
                with ApiConn(probe, timeout=65, trace=frames) as fresh:
                    hello = fresh.hello()["result"]
                    sent_after_ms = round((time.monotonic() - ready_at) * 1000, 3)
                    fresh.send_frame(dict(id=identifier, op="torrents.list", args={}))
                    actual = fresh.response(identifier)
                    attempts.append(dict(attempt=attempt, hello_state=hello["state"], after_ready_ms=sent_after_ms,
                                         response_ms=round((time.monotonic() - ready_at) * 1000, 3), response=actual))
                    client.case.check(True, "bounded retry immediately after real READY=1", json.dumps(attempts[-1], sort_keys=True))
            finally:
                if process.child.poll() is None:
                    probe.stop_serve(process)
                probe.lock_free()
        if actual.get("error", {}).get("code") == "engine_unavailable" and actual["error"].get("details", {}).get("state") == "starting":
            break
    evidence.write_text(json.dumps(attempts, indent=2, sort_keys=True) + "\n")
    return actual, evidence


def replay_error(expected, fixture_request, client, normalizer, harness, number, frames):
    """Drive the actual input/resource condition, without manufacturing output."""
    request = normalizer.request(fixture_request)
    identifier = request["id"]
    code = expected["error"]["code"]
    deferred = []
    def send(frame, typed=True):
        client.send_frame(frame, typed)
        return client.response(identifier)
    if code in ("invalid_message", "hello_required", "proto_unsupported"):
        with ApiConn(harness, timeout=8, trace=frames) as fresh:
            if code == "invalid_message":
                fresh.socket.sendall(b"x" * expected["error"]["details"]["bytes"])
                deferred.append(("/id", "P1-18", "fatal malformed input has a request-less error event, not the fixture's ID-bearing failure"))
            elif code == "proto_unsupported":
                fresh.send_frame(dict(id=identifier, op="hello", args=dict(proto=2, client=dict(name="fixture-test", version="0.2.0", kind="test"))))
            else:
                deferred.append(("/id", "P1-18", "silent hello deadline has a request-less error event, not the fixture's ID-bearing failure"))
            frame = fresh.receive()
            tail = list(fresh.events())
            client.case.check(frame.get("error", {}).get("code") == code and tail[-1].get("ev") == "bye",
                              "drive the real malformed/protocol/deadline condition", json.dumps(dict(code=code, actual=frame, tail=tail)))
            return frame, deferred
    if code == "unknown_op":
        return send(dict(id=identifier, op="torrents.stpo"), False), deferred
    if code == "invalid_args":
        return send(dict(id=identifier, op="torrents.remove", args=dict(hashes=["f" * 40], delete_files="bad")), False), deferred
    if code == "not_found":
        fixture_hash = expected["error"]["details"]["hash"]
        digest = hashlib.sha1(b"P1-06 isolated unknown hash").hexdigest()
        normalizer.bind("hash", fixture_hash, digest, "/error/details/hash")
        return send(dict(id=identifier, op="torrents.get", args=dict(hash=digest))), deferred
    if code == "invalid_state":
        directory = harness.sandbox.root / "completed-refusal"
        directory.mkdir()
        payload = directory / "completed.bin"
        payload.write_bytes(os.urandom(4096))
        fixture = fixtures.torrent_fixture(directory, [payload])
        client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), save_path=str(directory), seed=True))
        def seeded():
            value = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
            return value["state"] == "seeding", json.dumps(value)
        harness.wait_for(seeded, "completed error fixture is actually checked and seeding")
        client.call("torrents.set_seed", dict(hashes=[fixture.info_hash], on=False))
        return send(dict(id=identifier, op="torrents.start", args=dict(hashes=[fixture.info_hash]))), deferred
    if code == "engine_unavailable":
        frame, evidence = replay_starting_window(client, harness, identifier, frames)
        if frame.get("error", {}).get("code") != "engine_unavailable" or frame["error"].get("details", {}).get("state") != "starting":
            deferred.append(("/error/details/state", "P1-18", "ruling 4A: three READY-window retries did not produce starting refusal; engine dispatch waits for session; retry evidence " + str(evidence.relative_to(REPO))))
        return frame, deferred
    if code in ("busy", "timeout"):
        with stalled_api_source(client.case) as (source, entered):
            jobs = [dict(id=90000 + number * 10 + index, op="torrents.add", args=dict(source=dict(url=source), stopped=True)) for index in range(8)]
            if code == "busy":
                batch = jobs + [dict(id=identifier, op="app.info")]
                client.socket.sendall(b"".join(json.dumps(frame).encode() + b"\n" for frame in batch))
                frames.extend(dict(dir="c2s", frame=frame) for frame in batch)
            else:
                for job in jobs: client.send_frame(job)
                seen = set()
                while len(seen) < 8:
                    frame = client.receive()
                    if frame.get("id") in {job["id"] for job in jobs}: seen.add(frame["id"])
                client.case.check(entered.wait(3), "eight real URL dispatch jobs hold the pool", str(entered.is_set()))
                client.send_frame(dict(id=identifier, op="app.info"))
            began = time.monotonic()
            frame = client.response(identifier)
            client.case.check(frame.get("error", {}).get("code") == code,
                              "drive bounded dispatch/in-flight pressure rather than inject a server error",
                              f"elapsed={time.monotonic()-began:.3f}s; frame={frame!r}")
            expected_frame = frame
        # Releasing the HTTP source completes the auxiliary requests. Drain
        # their actual replies and mutation outcomes before the next fixture
        # input; response(identifier) may already have consumed some of them.
        identifiers = {job["id"] for job in jobs}
        def remaining():
            replies = [line["frame"] for line in frames if line["dir"] == "s2c" and line["frame"].get("id") in identifiers]
            pending = {reply["result"]["ref"] for reply in replies if reply.get("result", {}).get("state") == "pending"}
            completed = {line["frame"].get("ref") for line in frames if line["dir"] == "s2c" and line["frame"].get("ev") == "mutation.result"}
            return identifiers - {reply["id"] for reply in replies}, pending - completed
        deadline = time.monotonic() + 20
        while any(remaining()) and time.monotonic() < deadline:
            if client.receive() is None: break
        outstanding = remaining()
        client.case.check(not any(outstanding), "drain every real pressure-fixture reply/outcome before protocol validation",
                          json.dumps(dict(unanswered=sorted(outstanding[0]), pending=sorted(outstanding[1]))))
        return expected_frame, deferred
    # These complete expected error frames have no P1-06 RPC producer. Do not
    # mistake the old catalogue's app.info requests for a way to trigger them.
    owners = {"ambiguous": "P1-11", "safety": "P1-11", "unsupported": "P1-18",
              "already_running": "P1-18", "upstream": "P1-18", "internal": "P1-14"}
    reasons = {
        "ambiguous": "selector ambiguity is raised by the CLI before RPC",
        "safety": "destructive-confirmation safety is raised by the CLI before RPC",
        "unsupported": "the catalogue's unsupported legacy-only controls have no P1-06 RPC",
        "already_running": "already_running belongs to starting a second process, not an RPC on this owner",
        "upstream": "the fixture's tracker-source upstream failure requires later tracker/provider operations",
        "internal": "the fixture's diagnostic entries require the unadvertised log capability",
    }
    deferred.append(("/error/details/entries" if code == "internal" else "/error", owners[code], reasons[code]))
    return send(request), deferred


def replay_frame_deferrals(expected, op, caps=()):
    found = []
    def visit(value, pointer=""):
        if isinstance(value, dict):
            for key, child in value.items():
                here = pointer + "/" + key.replace("~", "~0").replace("/", "~1")
                if (key == "playable" or (key == "ready" and pointer.startswith("/result/files/"))) and "watch" not in caps:
                    found.append((here, "P5-05", "expected frame depends on the unadvertised watch capability"))
                elif key == "undo_until" and "origin" not in caps:
                    found.append((here, "P6-08", "expected frame depends on undo/origin restoration"))
                visit(child, here)
        elif isinstance(value, list):
            for index, child in enumerate(value): visit(child, pointer + "/" + str(index))
    visit(expected)
    if op in ("torrents.remove", "torrents.set_upload_limit") and expected.get("result", {}).get("state") == "pending" and "mutations" not in caps:
        found.append(("/result/state", "P1-09", "expected asynchronous mutation state requires the unadvertised mutations capability"))
    return found


CAP_BEADS = dict(subscribe="P1-08", mutations="P1-09", peers="P5-01", watch="P5-05",
                 queue="P6-05", origin="P6-08")


def replay_deferral_class(bead, reason):
    if bead == "ENVIRONMENT-CONDITIONAL":
        return "environment-conditional"
    if "proto-2" in reason:
        return "protocol"
    if "operation after preceding remove" in reason:
        return "post-remove"
    if "capability" in reason or "unadvertised" in reason or "undo/origin" in reason:
        return "capability-conditioned"
    return "undrivable-catalogue-frame"


def contract_group(group, binary, systemd):
    path = REPO / "contracts/ipc/v1" / (group + ".ndjson")
    lines = [json.loads(line) for line in path.read_text().splitlines()]
    normalizer = ReplayNormalizer()
    deferred, frames, request, prepared, optional, driven = [], [], None, {}, [], None
    available_caps = set()
    with replay_owned(group, binary, systemd) as (case, sandbox, harness, process, root):
        if group == "torrents":
            with ApiConn(harness) as setup:
                setup.hello()
                prepared = replay_preconditions(lines, setup, normalizer, root)
        with ApiConn(harness, timeout=20, trace=frames) as client:
            def defer(number, pointer, bead, reason):
                record = dict(fixture=str(path.relative_to(REPO)), line=number, pointer=pointer, bead=bead,
                              reason=reason, **{"class": replay_deferral_class(bead, reason)})
                deferred.append(record)
                case.check(True, "record the exact deferred fixture field and owner", json.dumps(record, sort_keys=True))
            for number, line in enumerate(lines, 1):
                if line["dir"] == "c2s":
                    request = line["frame"]
                    driven = None
                    live_request = normalizer.request(request) if group != "errors" or request["op"] == "hello" else None
                    next_frame = lines[number].get("frame", {}) if number < len(lines) and lines[number]["dir"] == "s2c" else {}
                    if group == "torrents" and request["op"] == "torrents.get" and request["args"]["hash"] in prepared and not replay_frame_deferrals(next_frame, request["op"], available_caps):
                        driven = drive_replay_state(client, prepared, request["args"]["hash"], harness, live_request)
                    if live_request is not None and driven is None:
                        client.send_frame(live_request)
                elif line["dir"] == "s2c":
                    expected = line["frame"]
                    if group == "torrents" and number in (43, 63):
                        minimum = (64 if number == 43 else 8) * fixtures.MIB
                        size = expected.get("result", {}).get("torrent", {}).get("size", 0)
                        case.check(size >= minimum, "fixture payload supports the required live state",
                                   json.dumps(dict(fixture=str(path.relative_to(REPO)), line=number, pointer="/result/torrent/size", size=size, required_minimum=minimum), sort_keys=True))
                    if request is None and group == "errors":
                        # Dedicated input validation scenarios have no c2s line.
                        raw = b'{"id":0,"op":"app.info"}\n' if number == 36 else b'x' * 65537 if number == 37 else None
                        if raw:
                            client.socket.sendall(raw)
                            case.check(True, "drive the server-only fixture input validation scenario", f"line={number}; input bytes={len(raw)}")
                    current, request = request, None
                    condition_deferrals = []
                    if group == "errors" and current and "error" in expected:
                        actual, condition_deferrals = replay_error(expected, current, client, normalizer, harness, number, frames)
                    elif driven is not None:
                        actual, driven = driven, None
                    elif "id" in expected and expected["id"] is not None:
                        actual = client.response(normalizer.identities["request"][expected["id"]])
                    else:
                        actual = client.receive()
                        while actual and (actual.get("ev") in ("heartbeat", "mutation.result")):
                            actual = client.receive()
                    if actual is None:
                        case.check(False, "fixture receives a live response", f"EOF at {path.name}:{number}")
                        break
                    if group == "torrents" and current and current["op"] == "torrents.get" and current["args"]["hash"] in prepared and prepared[current["args"]["hash"]]["row"]["state"] == "waiting_vpn":
                        restore_replay_interface(client, harness)
                    # Frame-level deferral: its raw expected and live frame are
                    # both run through frozen P1-05 validation, never synthesized.
                    if "caps" in actual.get("result", {}):
                        available_caps = set(actual["result"]["caps"])
                    owners = condition_deferrals + replay_frame_deferrals(expected, current["op"] if current else None, available_caps)
                    if group == "torrents" and number == 9 and os.environ.get("FBT_TEST_VPN") != "1":
                        owners.append(("/result/state", "ENVIRONMENT-CONDITIONAL", "ruling 6: explicit fixture0 binding requires the harness VPN namespace fixture (vpn.py, opt-in FBT_TEST_VPN=1)"))
                    if group == "torrents" and number in (27, 29, 31, 35, 37, 39, 61) and "origin" not in available_caps:
                        if number == 61:
                            case.check(actual.get("error", {}).get("code") == "not_found",
                                       "post-remove upload-limit fixture reaches the actual missing-record condition",
                                       json.dumps(actual, sort_keys=True))
                        pointer = "/result/torrent/hash" if number == 39 else "/result"
                        owners.append((pointer, "P6-08", "ruling 4E: operation after preceding remove requires a surviving/undo record"))
                    if "instance" in expected.get("result", {}) and "instance" in actual.get("result", {}):
                        normalizer.bind("instance", expected["result"]["instance"], actual["result"]["instance"], "/result/instance")
                    for index, cap in enumerate(expected.get("result", {}).get("caps", [])):
                        if cap in CAP_BEADS and cap not in actual.get("result", {}).get("caps", []): owners.append((f"/result/caps/{index}", CAP_BEADS[cap], "unadvertised capability " + cap))
                    if group == "hello" and expected.get("result", {}).get("proto") == 2:

                        if cap in CAP_BEADS and cap not in available_caps: owners.append((f"/result/caps/{index}", CAP_BEADS[cap], "unadvertised capability " + cap))
                    if group == "hello" and expected.get("result", {}).get("proto") == 2 and actual.get("result", {}).get("proto") != 2:
                        owners.append(("/result/proto", "P1-18", "proto-2 acceptance is not implemented"))
                    if owners:
                        for pointer, bead, reason in owners: defer(number, pointer, bead, reason)
                        continue
                    step = case.step(f"replay {path.relative_to(REPO)}:{number}", "canonical bytes match under the single declared normalization table")
                    try:
                        first_extra = len(normalizer.extra_optional)
                        normalizer.compare(expected, actual)
                        step.ok("Canonical bytes matched; identity bindings and volatile validity checked.")
                    except ReplayMismatch as failure:
                        step.fail(str(failure))
                    for pointer, value in normalizer.extra_optional[first_extra:]:
                        live_line = next(index for index in range(len(frames), 0, -1) if frames[index - 1].get("frame") is actual)
                        optional.append(dict(fixture=str(path.relative_to(REPO)), line=number, live_line=live_line, pointer=pointer, value=value))
            case.path.with_suffix(".deferred.json").write_text(json.dumps(deferred, indent=2, sort_keys=True) + "\n")
            case.path.with_suffix(".identities.json").write_text(json.dumps(normalizer.identities, indent=2, sort_keys=True) + "\n")
            case.path.with_suffix(".normalization.md").write_text(normalization_markdown())
            valid = validate_live_contract(case, sandbox, frames, optional=optional)
            for record in optional:
                case.check(valid, "extra optional field", json.dumps(record, sort_keys=True))
            validate_live_contract(case, sandbox, lines, "fixture")
            if group == "torrents": process.wait()


def contract_hello(binary, systemd):
    contract_group("hello", binary, systemd)


def contract_errors(binary, systemd):
    contract_group("errors", binary, systemd)


def contract_torrents(binary, systemd):
    contract_group("torrents", binary, systemd)


class InstalledUnit:
    """Real static notify unit linked only into the manager's runtime search path."""
    def __init__(self, harness):
        self.h, self.case, self.sandbox = harness, harness.case, harness.sandbox
        self.systemctl = shutil.which("systemctl", path=os.defpath)
        self.control_env = manager_environment(self.sandbox)
        self.sandbox.env["DBUS_SESSION_BUS_ADDRESS"] = os.environ.get(
            "DBUS_SESSION_BUS_ADDRESS", f"unix:path=/run/user/{os.geteuid()}/bus")
        self.path = Path(self.sandbox.env["XDG_CONFIG_HOME"]) / "systemd/user" / self.sandbox.unit
        self.launcher = fixtures.path_shim("omarchy-launch-or-focus-tui", "exit 0", self.sandbox.root)
        fixtures.path_shim("systemctl", "exec " + shlex.quote(self.systemctl) + ' "$@"', self.sandbox.root)
        self.linked = False

    def persistent_links_absent(self):
        # Metadata-only check required by the regression. Never remove a
        # persistent link, even if an earlier test mistakenly created it.
        paths = [OWNER_UNIT_DIRECTORY / self.sandbox.unit,
                 *OWNER_UNIT_DIRECTORY.glob("*.wants/" + self.sandbox.unit),
                 *OWNER_UNIT_DIRECTORY.glob("*.requires/" + self.sandbox.unit)]
        found = [str(path) for path in paths if path.exists() or path.is_symlink()]
        if not self.case.check(not found, "no persistent fixture link exists (refuse if present)", f"links={found}"):
            raise RuntimeError("persistent fixture link found; refusing manager operations")

    def control(self, *args, codes=(0,), timeout=25, strict=True):
        private_environment(self.sandbox)
        if args[0] in ("enable", "disable", "link") and "--runtime" not in args:
            raise RuntimeError("refusing persistent unit operation")
        step = self.case.step("systemctl --user " + shlex.join(args), f"exit in {codes} within {timeout}s on the disposable unit")
        child = self.case.popen([self.systemctl, "--user", *args], env=self.control_env,
                               cwd=self.sandbox.root, stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=timeout)
            result = ControlResult(child.returncode, stdout.strip())
        except subprocess.TimeoutExpired:
            child.kill(); child.communicate(timeout=2)
            result = ControlResult(None, "Owned systemctl command timed out")
        ok = step.close(result.code in codes, f"Exit {result.code}; stdout={result.output!r}.")
        if strict and not ok:
            raise RuntimeError(f"owned systemctl {args} failed: {result}")
        return result

    def __enter__(self):
        try:
            self.persistent_links_absent()
            self.h.cli("setup")
            self.case.check(self.path.exists(), "setup writes the selected private static unit", str(self.path))
            self.drop = Path(f"/run/user/{os.geteuid()}/systemd/user") / (self.sandbox.unit + ".d")
            self.drop.mkdir()
            drop = self.drop
            def quote(value):
                return json.dumps(str(value).replace("%", "%%"), ensure_ascii=False)
            environment = "\n".join("Environment=" + quote(key + "=" + self.sandbox.env[key])
                                     for key in self.sandbox.keys if key in self.sandbox.env)
            (drop / "fixture.conf").write_text("[Service]\n" + environment +
                "\nEnvironment=" + quote("DBUS_SESSION_BUS_ADDRESS=" + self.sandbox.env["DBUS_SESSION_BUS_ADDRESS"]) +
                "\nWorkingDirectory=" + str(self.sandbox.root) +
                "\nStandardOutput=append:" + str(self.case.path.with_suffix(".unit.stdout.log")) +
                "\nStandardError=append:" + str(self.case.suite.stderr_path) + "\n")
            self.control("link", "--runtime", str(self.path))
            self.linked = True
            self.control("daemon-reload")
            loaded = self.control("show", self.sandbox.unit, "--property=DropInPaths", "--value").output
            environment = self.control("show", self.sandbox.unit, "--property=Environment", "--value").output
            # systemctl's shell-style display escapes literal dollars even
            # inside quotes; shlex retains that escape. The child check below
            # independently compares the raw /proc environment byte strings.
            fields = [field.replace("\\$", "$") for field in shlex.split(environment)]
            working_directory = self.control("show", self.sandbox.unit, "--property=WorkingDirectory", "--value").output
            isolated = working_directory == str(self.sandbox.root) and str(drop / "fixture.conf") in loaded and all(
                key + "=" + self.sandbox.env[key] in fields for key in self.sandbox.keys)
            if not self.case.check(isolated, "manager loaded every private fixture environment value before any start",
                                   f"DropInPaths={loaded}; WorkingDirectory={working_directory}; isolated={isolated}"):
                raise RuntimeError("refusing to start a unit without verified sandbox environment")
            self.h.installed_unit = self
            return self
        except BaseException:
            self.close()
            raise

    def __exit__(self, *_):
        self.close()

    def stopped(self, cgroup):
        result = self.control("show", self.sandbox.unit,
            "--property=ActiveState,SubState,MainPID,Job", strict=False)
        state = dict(line.split("=", 1) for line in result.output.splitlines() if "=" in line)
        members = []
        if cgroup:
            root = Path("/sys/fs/cgroup") / cgroup.lstrip("/")
            for path in [root / "cgroup.procs", *root.glob("**/cgroup.procs")]:
                try:
                    members += [int(pid) for pid in path.read_text().split() if process_alive(int(pid))]
                except FileNotFoundError:
                    pass
        stopped = (result.code == 0 and state.get("ActiveState") in ("inactive", "failed") and
                   state.get("MainPID") == "0" and state.get("Job") in ("", "0") and
                   state.get("SubState") != "auto-restart" and not members)
        return stopped, f"state={state}; cgroup={cgroup}; live members={members}"

    def close(self):
        try:
            self.close_owned()
        except BaseException:
            if self.linked:
                self.sandbox.retain()
                self.case.check(False, "retain isolation when cleanup cannot be proved",
                                f"Sandbox {self.sandbox.root} and runtime drop-in retained")
            raise

    def close_owned(self):
        if self.linked:
            cgroup = self.control("show", self.sandbox.unit, "--property=ControlGroup", "--value").output
            if cgroup and cgroup.rsplit("/", 1)[-1] != self.sandbox.unit:
                self.sandbox.retain()
                raise RuntimeError(f"refusing cleanup of unexpected cgroup {cgroup}; sandbox retained")
            result = self.control("stop", self.sandbox.unit,
                                  codes=getattr(self, "stop_codes", (0, 5)),
                                  timeout=getattr(self, "stop_timeout", 25), strict=False)
            if result.code not in (0, 5) or not self.stopped(cgroup)[0]:
                # Kill only the named owned unit; a second stop cancels any
                # restart that killing an on-failure service might schedule.
                self.control("kill", "--signal=KILL", "--kill-whom=all", self.sandbox.unit,
                             codes=(0, 1, 5), timeout=3, strict=False)
                self.control("stop", self.sandbox.unit, timeout=25, strict=False)
            if not self.h.wait_for(lambda: self.stopped(cgroup), "owned unit and cgroup stop before isolation teardown", timeout=5):
                self.sandbox.retain()
                raise RuntimeError(f"cleanup unproved; retained sandbox {self.sandbox.root} and drop-in {self.drop}")
            self.control("disable", "--runtime", self.sandbox.unit)
            self.control("reset-failed", self.sandbox.unit, codes=(0, 1))
            self.linked = False
        if getattr(self, "drop", None) is not None and self.drop.exists():
            shutil.rmtree(self.drop)
            self.control("daemon-reload")
        self.h.installed_unit = None
        self.persistent_links_absent()
        for name in ("systemctl", "omarchy-launch-or-focus-tui", "omarchy-shell", "xdg-mime"):
            log = self.sandbox.root / "shims" / (name + ".log")
            if log.exists():
                shutil.copyfile(log, self.case.path.with_suffix("." + name + ".log"))

    def verify_child(self):
        pid = self.pid()
        if pid > 1:
            try:
                fields = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
                isolated = all((key + "=" + self.sandbox.env[key]).encode() in fields for key in self.sandbox.keys)
                cwd = Path(f"/proc/{pid}/cwd").resolve()
                if not self.case.check(isolated and cwd == self.sandbox.root,
                                       "actual owned child keeps every private environment path and working directory", f"pid={pid}; isolated={isolated}; cwd={cwd}"):
                    self.control("stop", self.sandbox.unit)
                    raise RuntimeError("stopped owned unit after failed child isolation verification")
            except FileNotFoundError:
                self.case.check(False, "owned child remains available for isolation verification", f"pid={pid} exited during verification")

    def state(self):
        return self.control("show", self.sandbox.unit, "--property=ActiveState", "--value").output

    def pid(self):
        return int(self.control("show", self.sandbox.unit, "--property=MainPID", "--value").output or 0)

    def autostart(self, enabled=True):
        config = Path(self.sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        config.write_text(config.read_text().replace("autostart = false", "autostart = true") if enabled
                          else config.read_text().replace("autostart = true", "autostart = false"))

    def no_launcher(self):
        log = self.launcher.with_name(self.launcher.name + ".log")
        calls = log.read_text().splitlines() if log.exists() else []
        self.case.check(not calls, "headless operations make zero terminal-launcher calls", f"launcher call count={len(calls)}")


def unit_is_static(binary, systemd):
    with Case("daemon", "unit_is_static") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        unit.control("enable", "--runtime", sandbox.unit)
        unit.persistent_links_absent()
        roots = [Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "systemd/user",
                 Path(f"/run/user/{os.geteuid()}/systemd/user"), unit.path.parent]
        links = [str(path) for root in roots for path in root.glob("*.wants/" + sandbox.unit)]
        state = unit.control("show", sandbox.unit, "--property=UnitFileState", "--value").output
        case.check(not links and state in ("static", "linked", "linked-runtime"), "enable creates no wants link for the static unit", f"state={state}; links={links}")
        h.cli("setup", "--uninstall")
        case.check(not unit.path.exists(), "setup --uninstall removes only the selected unit")


def add_starts_unit_no_launcher(binary, systemd):
    with Case("daemon", "add_starts_unit_no_launcher") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        unit.autostart()
        # Resolve the magnet through a real second, isolated librqbit engine.
        # rqbit ignores magnet xs= HTTP sources, so an HTTP-only fixture cannot
        # prove this path. The local tracker advertises the seed's loopback port.
        with fixtures.magnet_fixture(sandbox.root) as fixture, Sandbox() as seedbox, Harness(case, binary, seedbox) as seed:
            # P6-12 deliberately refuses BEP 9 for private torrents. Use a
            # non-private info dictionary for metadata exchange; both engines
            # remain bound to lo and every supplied tracker/peer is loopback.
            info = dict(fixture.info)
            info.pop("private", None)
            tracker_url = urllib.parse.parse_qs(urllib.parse.urlsplit(fixture.magnet).query)["tr"][0]
            fixture.torrent.write_bytes(fixtures.bencode(dict(info=info, announce=tracker_url)))
            fixture.info = info
            fixture.info_hash = hashlib.sha1(fixtures.bencode(info)).hexdigest()
            source = seed.start_serve()
            if not (seed.receive_ready() and seed.wait_engine()):
                return
            seed.cli("add", str(fixture.torrent), "--seed", "--save", str(fixture.files[0].parent))
            seed.wait_for(lambda: ((row := seed.cli("show", fixture.info_hash)).get("complete") is True and row.get("state") == "uploading",
                                  f"Seed state={row.get('state')}; complete={row.get('complete')}"), "seed verifies the real fixture and starts serving")
            config = Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
            port = int(re.search(r"peer_port = (\d+)", config.read_text()).group(1))
            step = case.step("connect real seed peer", "loopback seed accepts a BitTorrent handshake for the fixture hash")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
                    handshake = b"\x13BitTorrent protocol" + b"\0" * 8 + bytes.fromhex(fixture.info_hash) + b"-FT104-" + b"0" * 13
                    connection.sendall(handshake)
                    reply = bytearray()
                    while len(reply) < 68:
                        part = connection.recv(68 - len(reply))
                        if not part: break
                        reply.extend(part)
                    step.close(len(reply) == 68 and reply[28:48] == bytes.fromhex(fixture.info_hash), f"peer port={port}; handshake bytes={len(reply)}")
            except OSError as error:
                step.fail(f"peer port={port}; error={error}")
            tracker = fixtures.bencode(dict(interval=30, complete=1, incomplete=0,
                peers=socket.inet_aton("127.0.0.1") + struct.pack(">H", port)))
            # Supply the tracker route on the magnet fixture's real HTTP server.
            # Its context owns the server, so find it through an explicit local
            # tracker fixture rather than relying on hidden magnet internals.
            with fixtures.HttpFixture(sandbox.root, {"/announce": dict(base64=base64.b64encode(tracker).decode())}) as tracker_server:
                magnet = "magnet:?" + urllib.parse.urlencode(dict(xt="urn:btih:" + fixture.info_hash,
                            tr=tracker_server.base + "/announce", dn=fixture.info["name"]))
                value = h.cli("add", magnet, start=True, timeout=30)
                case.check(bool(tracker_server.requests), "real loopback tracker receives metadata-discovery requests", str(tracker_server.requests))
                case.check(value.get("hash") == fixture.info_hash and unit.state() == "active", "magnet add starts the static unit and resolves real local peer metadata", str(value))
                saved = Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine/torrents.json"
                records = json.loads(saved.read_text())["records"] if saved.exists() else []
                case.check(any(record["hash"] == fixture.info_hash for record in records),
                           "magnet add durably saves the fixture torrent record", str(records))
                unit.no_launcher()
                unit.control("stop", sandbox.unit)
                h.lock_free()
            seed.stop_serve(source)


def status_never_starts(binary, systemd):
    with Case("daemon", "status_never_starts") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        unit.autostart()
        value = h.cli("status", start=True)
        case.check(value.get("app_open") is False and unit.state() == "inactive", "status leaves the disposable unit inactive", str(value))
        for args, code in ((("list",),0), (("show", "abcd"),6), (("stop", "all"),0), (("delete", "all"),0)):
            value = h.cli(*args, start=True, code=code)
            shape = (value.get("ok") is True and value.get("running") is False) if code == 0 else (value.get("code_name") == "not_found" and "fbittorrent list" in value.get("hint", ""))
            case.check(shape and unit.state() == "inactive", "P1-11 offline reads/intents retain canonical exits and never start the daemon", str(value))
        unit.no_launcher()


def down_stops(binary, systemd):
    with Case("daemon", "down_stops") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        h.cli("up", start=True)
        case.check(unit.state() == "active", "explicit up starts despite persistent autostart=false")
        h.cli("down", start=True)
        h.wait_for(lambda: (unit.state() == "inactive", "unit state observed"), "unit becomes inactive")
        case.check(not h.engine_path.exists(), "down removes the owned engine socket")
        h.lock_free(); unit.no_launcher()



def down_cancels_restart(binary, systemd):
    with Case("daemon", "down_cancels_restart") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        h.cli("up", start=True)
        pid = unit.pid()
        if not case.check(pid > 1 and h.daemon_pid() == pid, "restart-window kill target is the verified owned daemon", str(pid)):
            return
        os.kill(pid, signal.SIGKILL)
        h.wait_for(lambda: ((state := unit.control("show", sandbox.unit, "--property=SubState", "--value").output) == "auto-restart",
                            f"SubState={state}"), "unit schedules on-failure restart", timeout=1)
        h.cli("down", start=True)
        # Observe beyond RestartSec, not merely the free-lock interval.
        deadline = time.monotonic() + 2.3
        while time.monotonic() < deadline:
            time.sleep(.05)
        stopped, observed = unit.stopped("")
        try:
            h.engine_call("app/version")
            answers = True
        except (OSError, EOFError):
            answers = False
        case.check(stopped and not answers, "down cancels pending restart and remains stopped beyond RestartSec",
                   observed + f"; engine answers={answers}; crash socket path remains={h.engine_path.exists()}")
        h.lock_free()



def manager_query_restart_case(binary, name, malformed=False, remove_unit=False):
    with Case("daemon", name) as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        h.cli("up", start=True)
        pid = unit.pid()
        if not case.check(pid > 1 and h.daemon_pid() == pid, "query-failure kill target is the verified isolated owner", str(pid)):
            return
        marker = sandbox.root / "initial-show-failed"
        program = sandbox.root / "fail-initial-show.py"
        reply = "LoadState=loaded\nActiveState=inactive\nMainPID=0\nJob=\n" if malformed else ""
        program.write_text(f'''import os,sys
from pathlib import Path
if "show" in sys.argv and not Path({str(marker)!r}).exists():
 Path({str(marker)!r}).write_text("initial show intercepted")
 sys.stdout.write({reply!r})
 if not {malformed!r}: print("injected initial manager query failure", file=sys.stderr)
 sys.exit({0 if malformed else 1})
os.execv({unit.systemctl!r},[{unit.systemctl!r},*sys.argv[1:]])
''')
        compile(program.read_text(), str(program), 'exec')
        os.kill(pid, signal.SIGKILL)
        if not h.wait_for(lambda: ((state := unit.control("show", sandbox.unit, "--property=SubState", "--value").output) == "auto-restart",
                                   f"SubState={state}"), "owned unit schedules restart before failed query", timeout=1):
            return
        fixtures.path_shim("systemctl", "exec /usr/bin/python3 " + shlex.quote(str(program)) + ' "$@"', sandbox.root)
        previous = unit.path.read_bytes()
        if remove_unit:
            unit.path.unlink()
        try:
            result = h.cli("down", code=4, start=True)
        finally:
            if remove_unit:
                unit.path.write_bytes(previous)
        case.check(marker.exists() and result.get("ok") is False and result.get("code_name") == "backend" and
                   result.get("hint") == "systemctl --user stop " + sandbox.unit,
                   "failed or malformed initial manager query returns typed nonzero with the selected-unit stop hint",
                   f"intercepted={marker.exists()}; removed unit file={remove_unit}; result={result}")
        h.wait_for(lambda: ((after := unit.pid()) > 1 and after != pid and h.engine_call("app/version").get("error") is None,
                            f"before={pid}; after={after}; state={unit.state()}"),
                   "restart remains possible so down must not have acknowledged success", timeout=3)
        h.cli("down", start=True)
        case.check(unit.stopped("")[0], "retry with a healthy manager query stops the owned unit")
        h.lock_free()


def down_failed_manager_query(binary, systemd):
    manager_query_restart_case(binary, "down_failed_manager_query")


def down_malformed_manager_query(binary, systemd):
    manager_query_restart_case(binary, "down_malformed_manager_query", malformed=True)


def down_failed_query_without_unit_file(binary, systemd):
    manager_query_restart_case(binary, "down_failed_query_without_unit_file", remove_unit=True)


def cleanup_failed_stop(binary, systemd, timed_out=False):
    name = "cleanup_stop_timeout" if timed_out else "cleanup_failed_stop"
    with Case("daemon", name) as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        h.cli("up", start=True)
        pid = unit.pid()
        if not case.check(pid > 1 and h.daemon_pid() == pid, "cleanup target is the verified owned daemon", str(pid)):
            return
        marker = sandbox.root / "failed-stop"
        real = unit.systemctl
        wrapper = sandbox.root / "systemctl-failed-stop"
        wrapper.write_text(f'''#!/usr/bin/python3
import os,sys,time
from pathlib import Path
if "stop" in sys.argv and not Path({str(marker)!r}).exists():
 assert Path({str(unit.drop)!r}).is_dir() and Path({str(sandbox.root)!r}).is_dir()
 assert Path("/proc/{pid}").exists()
 Path({str(marker)!r}).write_text("isolation retained while daemon is alive")
 {"time.sleep(5)" if timed_out else "sys.exit(1)"}
os.execv({real!r},[{real!r},*sys.argv[1:]])
''')
        wrapper.chmod(0o700)
        unit.systemctl = str(wrapper)
        unit.stop_codes = (None,) if timed_out else (1,)
        unit.stop_timeout = .1 if timed_out else 3
        unit.close()
        case.check(marker.exists() and not process_alive(pid) and sandbox.root.exists() and not unit.drop.exists(),
                   "failed stop status propagates and bounded owned-unit cleanup proves exit before removing drop-in",
                   f"injected {'timeout' if timed_out else 'exit 1'}; pid={pid}; alive={process_alive(pid)}; sandbox={sandbox.root.exists()}; drop-in={unit.drop.exists()}")


def cleanup_stop_timeout(binary, systemd):
    cleanup_failed_stop(binary, systemd, timed_out=True)



def foreground_down(binary, systemd):
    with Case("daemon", "foreground_down") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h:
        allow_manager_reads(h)
        process = h.start_serve()
        if not h.receive_ready():
            return
        h.cli("down", start=True)
        process.wait()
        case.check(not h.engine_path.exists(), "down still shuts down a foreground owner without an installed unit")
        h.lock_free()


def unit_install_unwritable(binary, systemd):
    with Case("daemon", "unit_install_unwritable") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h:
        fixtures.path_shim("systemctl", "exit 0", sandbox.root)
        h.cli("setup")
        unit = Path(sandbox.env["XDG_CONFIG_HOME"]) / "systemd/user" / sandbox.unit
        before, inode = unit.read_bytes(), unit.stat().st_ino
        unit.parent.chmod(0o500)
        try:
            result = h.cli("setup", code=1)
            case.check(result.get("ok") is False and unit.read_bytes() == before and unit.stat().st_ino == inode,
                       "0500 directory refuses install and keeps previous unit bytes and inode", f"error={result.get('error')}; inode={unit.stat().st_ino}")
        finally:
            unit.parent.chmod(0o700)
        case.check(list(unit.parent.iterdir()) == [unit], "failed install leaves no temporary unit")


def kill9_restarts(binary, systemd):
    with Case("daemon", "kill9_restarts") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        h.cli("up", start=True)
        before = unit.pid()
        if not case.check(before > 1 and h.daemon_pid() == before,
                          "kill target is the verified owned daemon PID, never zero or a process group", str(before)):
            return
        began = time.monotonic()
        os.kill(before, signal.SIGKILL)
        h.wait_for(lambda: ((pid := unit.pid()) > 0 and pid != before and h.engine_call("app/version").get("error") is None,
                            f"old pid={before}; new pid={pid}"), "systemd restarts killed daemon", timeout=3)
        case.check(time.monotonic() - began < 3, "replacement daemon answers within three seconds", f"elapsed={time.monotonic()-began:.3f}s")
        unit.no_launcher()


def killmode_mixed(binary, systemd):
    with Case("daemon", "killmode_mixed") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        value = unit.control("show", sandbox.unit, "--property=KillMode").output
        case.check(value == "KillMode=mixed", "manager loads KillMode=mixed", value)


def start_limit_not_hit(binary, systemd):
    with Case("daemon", "start_limit_not_hit") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        began = time.monotonic()
        for _ in range(10):
            h.cli("up", start=True)
            h.cli("down", start=True)
        elapsed = time.monotonic() - began
        h.cli("up", start=True)
        case.check(elapsed < 30 and unit.state() == "active", "ten up/down cycles in thirty seconds leave the unit startable", f"ten cycles={elapsed:.3f}s")
        unit.no_launcher()


def start_while_deactivating(binary, systemd):
    with Case("daemon", "start_while_deactivating") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        h.cli("up", start=True)
        drop = unit.drop / "slow-stop.conf"
        drop.write_text("[Service]\nExecStopPost=/usr/bin/sleep 0.8\n")
        unit.control("daemon-reload")
        first = sandbox.root / "first-start"
        pid = unit.pid()
        if not case.check(pid > 1 and h.daemon_pid() == pid, "race target is the verified owned daemon PID", str(pid)):
            return
        # Force the exact race after the first active-unit start returns. All
        # unit transitions/probes remain real; this seam controls interleaving.
        script = f'''import os,subprocess,sys,time
from pathlib import Path
args=sys.argv[1:]
if "start" in args and not Path({str(first)!r}).exists():
 os.kill({pid}, {int(signal.SIGCONT)})
 Path({str(first)!r}).touch()
 subprocess.run([{unit.systemctl!r},*args],check=True)
 subprocess.run([{unit.systemctl!r},"--user","--no-block","stop",{sandbox.unit!r}],check=True)
 time.sleep(.15)
 sys.exit(0)
os.execv({unit.systemctl!r},[{unit.systemctl!r},*args])
'''
        shim = sandbox.root / "shims/systemctl"
        shim.write_text("#!/usr/bin/python3\n" + script); shim.chmod(0o700)
        # The initial liveness probe times out while the process is stopped.
        os.kill(pid, signal.SIGSTOP)
        try:
            h.cli("up", start=True, timeout=15)
        finally:
            if unit.pid() == pid:
                os.kill(pid, signal.SIGCONT)
        case.check(first.exists() and unit.state() == "active" and unit.pid() != pid,
                   "deactivating start is retried once and a new owner answers", f"old pid={pid}; new pid={unit.pid()}")
        unit.no_launcher()


def startup_failure_hint(binary, systemd):
    with Case("daemon", "startup_failure_hint") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        (unit.drop / "failure.conf").write_text("[Service]\nExecStart=\nExecStart=/usr/bin/false\n")
        unit.control("daemon-reload")
        value = h.cli("up", start=True, code=4)
        expected = "journalctl --user -u " + sandbox.unit.removesuffix(".service") + " -n 50"
        case.check(value.get("hint") == expected and not h.engine_path.exists(),
                   "failed start exits 4 with the selected unit's journal hint", str(value))
        unit.no_launcher()


def no_manager(binary, systemd):
    with Case("daemon", "no_manager") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h:
        # setup can write its real unit using a successful recording reload shim;
        # the same shim then reports that no manager is available to start it.
        fixtures.path_shim("systemctl", 'case "$*" in *daemon-reload*) exit 0;; *) exit 1;; esac', sandbox.root)
        h.cli("setup")
        value = h.cli("up", start=True, code=4)
        case.check(value.get("hint") == "fbittorrent serve" and not h.engine_path.exists(),
                   "missing user manager exits 4 with foreground serve hint and no self-daemonization", str(value))
        value = h.cli("serve", code=4)
        case.check(value.get("hint") == "fbittorrent up" and not h.engine_path.exists(),
                   "explicit --no-start also prevents serve from becoming an owner", str(value))
        process = h.start_serve()
        if h.receive_ready() and h.wait_engine():
            h.cli("up", start=True)
            h.stop_serve(process)
        original = sandbox.env["FBITTORRENT_UNIT"]
        for invalid in ("../fbittorrent.service", "bad\udcff.service"):
            sandbox.env["FBITTORRENT_UNIT"] = invalid
            h.cli("up", start=True, code=2)
        sandbox.env["FBITTORRENT_UNIT"] = original



def dashboard_routes(binary, systemd):
    with Case("daemon", "dashboard_routes") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        shell = fixtures.path_shim("omarchy-shell", "exit 0", sandbox.root)
        manifest = Path(sandbox.env["XDG_CONFIG_HOME"]) / "omarchy/plugins/oliver.torrents/manifest.json"
        manifest.parent.mkdir(parents=True)
        manifest.write_text(json.dumps(dict(kinds=["panel"])))
        query = "literal spaces; $variable 'quotes'"
        h.cli("dashboard", "--view", "search", "--query", query, start=True)
        calls = [json.loads(line) for line in shell.with_name("omarchy-shell.log").read_text().splitlines()]
        case.check(calls[-1][:3] == ["shell", "summon", "oliver.torrents"] and
                   json.loads(calls[-1][3]) == dict(view="search", query=query),
                   "dashboard passes one literal JSON payload to the installed panel", str(calls))
        unit.no_launcher()
        manifest.unlink()
        h.cli("up", "--ui", start=True)
        calls = [json.loads(line) for line in unit.launcher.with_name(unit.launcher.name + ".log").read_text().splitlines()]
        case.check(len(calls) == 1 and calls[0][-1] == "tui", "explicit up --ui falls back to the TUI launcher when no panel is installed", str(calls))


def setup_desktop_handlers(binary, systemd):
    with Case("daemon", "setup_desktop_handlers") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h:
        fixtures.path_shim("systemctl", "exit 0", sandbox.root)
        fixtures.path_shim("xdg-mime", "exit 0", sandbox.root)
        fixtures.path_shim("update-desktop-database", "exit 0", sandbox.root)
        h.cli("setup", "--magnet")
        apps = Path(sandbox.env["XDG_DATA_HOME"]) / "applications"
        app, magnet = [(apps / name).read_text() for name in ("fbittorrent.desktop", "fbittorrent-magnet.desktop")]
        case.check(" dashboard\n" in app and " add --origin handler %u\n" in magnet and "Terminal=false" in magnet,
                   "setup writes dashboard and direct headless magnet desktop entries", app + magnet)
        icon = Path(sandbox.env["XDG_DATA_HOME"]) / "icons/hicolor/scalable/apps/fbittorrent.svg"
        case.check("Icon=fbittorrent" in app and "Icon=fbittorrent" in magnet and
                   icon.read_text() == (REPO / "assets/fbittorrent.svg").read_text(),
                   "both desktop entries and the private hicolor SVG use fbittorrent's download glyph", str(icon))
        calls = [json.loads(line) for line in (sandbox.root / "shims/xdg-mime.log").read_text().splitlines()]
        case.check(len(calls) == 2 and all(call[:2] == ["default", "fbittorrent-magnet.desktop"] for call in calls),
                   "both magnet and torrent-file MIME registrations use the direct add handler", str(calls))


def unit_exec_path(binary, systemd):
    with Case("daemon", "unit_exec_path") as case, Sandbox() as sandbox, Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
        executable = sandbox.root / "binary with % $ spaces"
        # Prefer a hard link; /tmp may be a separate filesystem.
        try:
            os.link(binary, executable)
        except OSError as error:
            if error.errno != 18:
                raise
            shutil.copy2(binary, executable)
        h.binary = str(executable)
        h.cli("setup")
        h.cli("up", start=True)
        case.check(unit.state() == "active", "current_exe path with spaces, percent and dollar executes as one literal path")
        h.cli("down", start=True)
        h.lock_free()


def install_atomic(binary, systemd):
    with Case("daemon", "install_atomic") as case, Sandbox() as sandbox:
        home = sandbox.root / "home with % $ spaces"
        home.mkdir(mode=0o700)
        sandbox.env["HOME"] = str(home)
        with Harness(case, binary, sandbox) as h, InstalledUnit(h) as unit:
            fixtures.path_shim("xdg-mime", "exit 0", sandbox.root)
            fixtures.path_shim("update-desktop-database", "exit 0", sandbox.root)
            installed = home / ".local/bin/fbittorrent"
            installed.parent.mkdir(parents=True)
            shutil.copy2(binary, installed)
            h.binary = str(installed)
            h.cli("setup")
            h.cli("up", start=True)
            pid, inode = unit.pid(), installed.stat().st_ino
            if not case.check(pid > 1 and h.daemon_pid() == pid, "atomic-install target is the verified owned daemon", str(pid)):
                return
            environment = dict(sandbox.env, CARGO_BUILD_JOBS="4",
                               RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path(os.environ["HOME"]) / ".rustup")),
                               CARGO_HOME=os.environ.get("CARGO_HOME", str(Path(os.environ["HOME"]) / ".cargo")))
            step = case.step("bash scripts/install.sh (inside the outer release lock and memory scope)", "real locked release build, atomic replacement and setup --magnet exit 0")
            child = case.popen(["bash", "scripts/install.sh"], cwd=REPO, env=environment, stdout=subprocess.PIPE, text=True)
            try:
                stdout, _ = child.communicate(timeout=1200)
                artifact = case.path.with_suffix(".install.stdout.log")
                artifact.write_text(stdout)
                step.close(child.returncode == 0, f"Exit {child.returncode}; stdout={stdout[-1600:]!r}", [artifact])
            except subprocess.TimeoutExpired:
                child.kill(); child.communicate(timeout=5)
                step.fail("Installer exceeded its deadline.")
            new_inode = installed.stat().st_ino
            running_inode = Path(f"/proc/{pid}/exe").stat().st_ino
            case.check(new_inode != inode and running_inode == inode and unit.pid() == pid and
                       h.engine_call("app/version").get("error") is None,
                       "atomic mv replaces the executable while its old inode continues serving",
                       f"old inode={inode}; installed inode={new_inode}; process inode={running_inode}; pid={pid}")
            case.check(not installed.with_suffix(".tmp").exists(), "installer leaves no temporary executable")
            h.cli("down", start=True)


if __name__ == "__main__":
    sys.modules["daemon"] = sys.modules[__name__]
from daemon_bus import (bus_subscriptions, subscribe_gap_recovery,
                        structural_frame_within_100ms, detail_topic_released,
                        engine_restart_keeps_connection, slow_client_closed_others_live,
                        rev_soak_10min, state_watcher_cancellation_stress, events_since_seq_replay, mutation_bus_sequence)

from daemon_notify import (notify_completed, notify_action_open, notify_action_default,
    notify_action_folder, notify_added_undo, notify_off, notify_failed, notify_queue_bounded,
    notify_vpn_once_per_transition, notify_cli_policy, notify_killed_handler, notify_handler_cli_once)

from daemon_log import log_injected_error, log_follow, hello_counters, log_down_never_starts
from daemon_dormant import lazy_restore_no_checks, dormant_seed_checks, dormant_missing_files, dormant_open_no_check
from daemon_fixture_cases import fixture_serving_states, fixture_launcher_killed, fixture_daemon_killed, fixture_stdin_lifetime, fixture_peer_pair
from daemon_upgrade import upgrade_idle_exit_75, upgrade_pending_while_busy, restart_now, restart_closed_never_starts

from daemon_policy import (dashboard_bar_presence, idle_exit_no_torrents, idle_exit_download_keeps_alive,
    idle_exit_interactive_keeps_alive, idle_exit_bar_does_not, idle_exit_parked_keeps_alive,
    idle_exit_disabled_never_exits, idle_exit_disabled_shutdown_drains,
    idle_exit_glance_pauses, idle_exit_glance_drop_resumes, idle_exit_mutation_keeps_alive)

# P1-09: every mutation records its complete envelope, including resend identity.
def mutation_request(client, op, args, ref=None, instance=None, error=None):
    identifier = client.next_id
    client.next_id += 1
    frame = dict(id=identifier, op=op, args=args)
    if ref is not None:
        frame["ref"] = ref
    if instance is not None:
        frame["instance"] = instance
    step = client.case.step(json.dumps(frame, sort_keys=True),
                            "matching success envelope" if error is None else f"error.code == {error}")
    client.send_frame(frame, typed=error is None)
    while (reply := client.receive()) is not None:
        if "ev" in reply:
            client.pushed.append(reply)
            continue
        step.close(reply.get("id") == identifier and
                   (reply.get("ok") is True if error is None else reply.get("error", {}).get("code") == error),
                   json.dumps(reply, sort_keys=True))
        return reply
    step.fail("EOF before the matching response")
    raise EOFError(op)


def mutation_hello(client, name):
    return mutation_request(client, "hello", dict(proto=1, client=dict(name=name, version="0", kind="test")))["result"]


def mutation_outcome(client, reference):
    for frame in client.events():
        if frame.get("ev") == "mutation.result" and frame.get("ref") == reference:
            client.case.check(frame.get("outcome") == "done", "pending mutation finishes with done", json.dumps(frame, sort_keys=True))
            return frame
    raise EOFError("mutation.result " + reference)


def mutation_add_path(client, sandbox, name="payload"):
    root = sandbox.root / name
    root.mkdir()
    fixture = fixtures.two_file_fixture(root)
    # Keep a real incomplete torrent; completion does not race a stop assertion.
    fixtures.corrupt_byte(fixture.files[0], 0)
    args = dict(source=dict(path=str(fixture.torrent)), stopped=True, save_path=str(root))
    reply = mutation_request(client, "torrents.add", args)
    client.case.check(reply.get("result", {}).get("hash") == fixture.info_hash,
                      "path add returns its exact durable hash", json.dumps(reply))
    return fixture, args


def ref_replay_remove(binary, systemd):
    with api_owned("ref_replay_remove", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        mutation_hello(client, "ref-remove")
        fixture, args = mutation_add_path(client, sandbox)
        remove = dict(hashes=[fixture.info_hash], delete_files=False)
        first = mutation_request(client, "torrents.remove", remove, ref="remove-once")
        replay = mutation_request(client, "torrents.remove", remove, ref="remove-once")
        case.check(first.get("result") == replay.get("result"), "remove replay returns the stored first result", json.dumps(replay))
        mutation_request(client, "torrents.add", args, ref="readd")
        replay = mutation_request(client, "torrents.remove", remove, ref="remove-once")
        rows = client.call("torrents.list", {})["result"]["torrents"]
        case.check(first.get("result") == replay.get("result") and [row["hash"] for row in rows] == [fixture.info_hash],
                   "old remove ref does not remove a later add of the same hash", json.dumps(rows))


def ref_per_client(binary, systemd):
    with api_owned("ref_per_client", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as first, ApiConn(harness) as second:
        mutation_hello(first, "client-one"); mutation_hello(second, "client-two")
        inputs = []
        for client, name in [(first, "one"), (second, "two")]:
            fixture, args = mutation_add_path(client, sandbox, name)
            inputs.append(fixture.info_hash)
            mutation_request(client, "torrents.remove", dict(hashes=[fixture.info_hash], delete_files=False), ref="same-ref")
        case.check(len(set(inputs)) == 2 and first.call("torrents.list", {})["result"]["torrents"] == [],
                   "distinct client names apply both independent removals with the same ref", str(inputs))


def remove_requires_flag(binary, systemd):
    with api_owned("remove_requires_flag", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        mutation_hello(client, "required-flag")
        fixture, _ = mutation_add_path(client, sandbox)
        failure = mutation_request(client, "torrents.remove", dict(hashes=[fixture.info_hash]), ref="missing-flag", error="invalid_args")
        case.check(failure["error"].get("details", {}).get("key") == "delete_files", "missing delete_files names the mandatory argument", json.dumps(failure))
        rows = client.call("torrents.list", {})["result"]["torrents"]
        case.check([row["hash"] for row in rows] == [fixture.info_hash] and all(path.exists() for path in fixture.files),
                   "invalid removal changes neither ledger nor payload", json.dumps(rows))


@contextmanager
def held_torrent_source(case, document):
    import http.server
    release, entered = threading.Event(), threading.Event()
    requests = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            requests.append(self.path)
            entered.set()
            release.wait(10)
            try:
                self.send_response(200)
                self.send_header("Content-Length", str(len(document)))
                self.end_headers()
                self.wfile.write(document)
            except OSError:
                pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    thread = threading.Thread(target=server.serve_forever, kwargs=dict(poll_interval=.05))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/legal.torrent", entered, release, requests
    finally:
        release.set(); server.shutdown(); server.server_close(); thread.join(2)
        case.check(not thread.is_alive(), "held HTTP fixture joins", f"alive={thread.is_alive()}")


def add_url_pending_then_result(binary, systemd):
    with api_owned("add_url_pending_then_result", binary, systemd) as (case, sandbox, harness, process):
        payload = sandbox.root / "Example e2.mkv"
        payload.write_bytes(b"legal local mutation fixture\n" * 16384)
        fixture = fixtures.torrent_fixture(sandbox.root, [payload], name="mutation-url")
        trace = []
        with held_torrent_source(case, fixture.torrent.read_bytes()) as (url, entered, release, requests), \
                ApiConn(harness) as origin, ApiConn(harness, trace=trace) as replay:
            hello = mutation_hello(origin, "url-reconnect")
            mutation_hello(replay, "url-reconnect")
            case.check("mutations" in hello["caps"], "hello advertises the mutation capability", json.dumps(hello))
            args = dict(source=dict(url=url), save_path=str(sandbox.root), stopped=True)
            first = mutation_request(origin, "torrents.add", args, ref="test-add", instance=hello["instance"])
            case.check(entered.wait(2) and first["result"] == dict(state="pending", ref="test-add"),
                       "URL pending is correlated and has no fabricated hash before the fetch", json.dumps(first))
            again = mutation_request(replay, "torrents.add", args, ref="test-add", instance=hello["instance"])
            case.check(again["result"] == first["result"], "in-flight replay receives the stored pending response", json.dumps(again))
            # The worker is daemon-owned; losing its original connection does not
            # discard the result or prevent delivery to a replaying connection.
            origin.close()
            release.set()
            outcome = mutation_outcome(replay, "test-add")
            case.check(outcome.get("result", {}).get("hash") == fixture.info_hash and outcome["result"]["duplicate"] is False
                       and requests == ["/legal.torrent"], "one real URL fetch completes on the replaying connection after original disconnect", json.dumps(outcome))
            frozen = [json.loads(line) for line in (REPO / "contracts/ipc/v1/mutations.ndjson").read_text().splitlines()]
            normalizer = ReplayNormalizer()
            normalizer.compare(frozen[4]["frame"], again)
            normalizer.compare(dict(result=frozen[12]["frame"]["result"]), dict(result=outcome["result"]))
            case.check(True, "pending and add result match the frozen mutation fixture using P1-06 normalization", json.dumps(outcome))
            if os.environ.get("FBT_API_CONTRACT_VALIDATOR"):
                validate_live_contract(case, sandbox, trace)
                # The frozen result omits optional duplicate. Keep P1-06's
                # extra-optional rule: both present and absent must validate.
                absent = json.loads(json.dumps(trace))
                for row in absent:
                    if row.get("frame", {}).get("ev") == "mutation.result":
                        row["frame"].get("result", {}).pop("duplicate", None)
                validate_live_contract(case, sandbox, absent, kind="optional-absent")
            # Reconnect after completion still gets the original pending receipt,
            # followed by the retained complete outcome.
            late = mutation_request(replay, "torrents.add", args, ref="test-add")
            event = mutation_outcome(replay, "test-add")
            case.check(late["result"] == first["result"] and event == outcome,
                       "late replay returns the original receipt and the same outcome", json.dumps(event))


def duplicate_add(binary, systemd):
    with api_owned("duplicate_add", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        mutation_hello(client, "duplicate")
        fixture, args = mutation_add_path(client, sandbox)
        before = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
        duplicate = mutation_request(client, "torrents.add", dict(args, seed=True, stopped=False, category="changed"), ref="duplicate-path")
        after = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
        case.check(duplicate["result"]["duplicate"] is True and duplicate["result"]["hash"] == fixture.info_hash
                   and all(before[key] == after[key] for key in ("intent", "seed", "save_path")),
                   "duplicate path add preserves existing transfer policy", json.dumps(duplicate))
        with fixtures.HttpFixture(sandbox.root, {"/duplicate.torrent": dict(file=str(fixture.torrent.relative_to(sandbox.root)))}) as http:
            mutation_request(client, "torrents.add", dict(source=dict(url=http.base + "/duplicate.torrent"), stopped=False, seed=True), ref="duplicate-url")
            event = mutation_outcome(client, "duplicate-url")
            case.check(event["result"]["duplicate"] is True and event["result"]["hash"] == fixture.info_hash,
                       "URL duplicate is a successful correlated result", json.dumps(event))


def ref_reused_different_args(binary, systemd):
    with api_owned("ref_reused_different_args", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        mutation_hello(client, "ref-fingerprint")
        fixture, _ = mutation_add_path(client, sandbox)
        args = dict(hashes=[fixture.info_hash], delete_files=False)
        mutation_request(client, "torrents.remove", args, ref="fingerprint")
        for op, changed in [("torrents.remove", dict(args, delete_files=True)), ("torrents.stop", dict(hashes=[fixture.info_hash]))]:
            failure = mutation_request(client, op, changed, ref="fingerprint", error="invalid_args")
            case.check(failure["error"]["message"] == "ref reused for a different request" and failure["error"]["details"]["key"] == "ref",
                       "ref fingerprint rejects changed operation or arguments", json.dumps(failure))
        # Failures are receipts too: making the missing record exist must not
        # convert a replay of its old error into a new application.
        missing = "b" * 40
        stop = dict(hashes=[missing])
        failure = mutation_request(client, "torrents.stop", stop, ref="stored-error", error="not_found")
        mutation_request(client, "torrents.add", dict(source=dict(magnet="magnet:?xt=urn:btih:" + missing), stopped=True))
        replay = mutation_request(client, "torrents.stop", stop, ref="stored-error", error="not_found")
        case.check(failure["error"] == replay["error"], "replay returns the stored error after state changes", json.dumps(replay))


def stale_instance_remove_refused(binary, systemd):
    with api_owned("stale_instance_remove_refused", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        hello = mutation_hello(client, "stale-remove")
        fixture, _ = mutation_add_path(client, sandbox)
        stale = ("0" if hello["instance"][0] != "0" else "1") + hello["instance"][1:]
        failure = mutation_request(client, "torrents.remove", dict(hashes=[fixture.info_hash], delete_files=True), ref="old-remove", instance=stale, error="invalid_state")
        case.check(failure["error"]["message"] == "daemon restarted since this request; check the list" and all(path.exists() for path in fixture.files),
                   "stale destructive intent is refused before removing data", json.dumps(failure))
        case.check(len(client.call("torrents.list", {})["result"]["torrents"]) == 1, "stale removal retains the record")


def stale_instance_stop_applies(binary, systemd):
    with api_owned("stale_instance_stop_applies", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        hello = mutation_hello(client, "stale-stop")
        fixture, _ = mutation_add_path(client, sandbox)
        mutation_request(client, "torrents.start", dict(hashes=[fixture.info_hash]))
        stale = ("0" if hello["instance"][0] != "0" else "1") + hello["instance"][1:]
        reply = mutation_request(client, "torrents.stop", dict(hashes=[fixture.info_hash]), ref="old-stop", instance=stale)
        row = client.call("torrents.get", dict(hash=fixture.info_hash))["result"]["torrent"]
        case.check(reply["result"]["state"] == "done" and row["intent"] == "pause", "stale idempotent stop still applies durable pause intent", json.dumps(reply))


def hashes_cap_257(binary, systemd):
    with api_owned("hashes_cap_257", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        hello = mutation_hello(client, "hash-bound")
        case.check(hello["limits"]["hashes"] == 256, "hello publishes the authoritative hash cap", json.dumps(hello["limits"]))
        for hashes in [[], ["a" * 40] * 257]:
            for op, extra in [("torrents.start", {}), ("torrents.stop", {}), ("torrents.remove", dict(delete_files=False)),
                              ("torrents.set_seed", dict(on=True)), ("torrents.set_upload_limit", dict(bps=32768))]:
                failure = mutation_request(client, op, dict(hashes=hashes, **extra), error="invalid_args")
                case.check(failure["error"].get("details", {}).get("key") == "hashes", "out-of-bounds hash list identifies hashes", json.dumps(failure))


def mutation_magnets_and_file_removal(binary, systemd):
    with api_owned("mutation_magnets_and_file_removal", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        hello = mutation_hello(client, "magnet-controls")
        digest = "c1" * 20
        base32 = base64.b32encode(bytes.fromhex(digest)).decode()
        result = mutation_request(client, "torrents.add", dict(source=dict(magnet="magnet:?xt=urn:btih:" + base32 + "&dn=Local%20name")), ref="base32")
        case.check(result["result"]["hash"] == digest and result["result"]["name"] == "Local name" and result["result"]["state"] == "pending",
                   "base32 magnet immediately has its lowercase hash and display name", json.dumps(result))
        row = client.call("torrents.get", dict(hash=digest))["result"]["torrent"]
        case.check(row["state"] == "metadata" and "size" not in row and "progress" not in row, "unresolved magnet has a truthful metadata row", json.dumps(row))
        duplicate = mutation_request(client, "torrents.add", dict(source=dict(magnet="magnet:?xt=urn:btih:" + digest + "&dn=Changed"), stopped=True), ref="duplicate-magnet")
        case.check(duplicate["result"]["duplicate"] is True and duplicate["result"]["name"] == "Local name", "duplicate magnet preserves its saved name and intent", json.dumps(duplicate))
        mutation_request(client, "torrents.stop", dict(hashes=[digest]))
        ledger = json.loads((Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine/torrents.json").read_text())
        case.check(next(row for row in ledger["records"] if row["hash"] == digest)["paused"], "stop of metadata persists pause before responding")
        mutation_request(client, "torrents.add", dict(source=dict(magnet="magnet:?xt=urn:btmh:1220" + "ab" * 32)), error="unsupported")
        fixture, _ = mutation_add_path(client, sandbox)
        args = dict(hashes=[fixture.info_hash], delete_files=True)
        pending = mutation_request(client, "torrents.remove", args)
        reference = pending["result"]["ref"]
        case.check(pending["result"]["state"] == "pending" and re.fullmatch("srv-" + hello["instance"][:8] + r"-\d+", reference),
                   "file deletion is pending with a server-generated instance ref", json.dumps(pending))
        mutation_outcome(client, reference)
        case.check(not any(path.exists() for path in fixture.files), "file-removal done follows real payload deletion")
        replay = mutation_request(client, "torrents.remove", args, ref=reference)
        case.check(replay["result"] == pending["result"], "generated ref is replayable", json.dumps(replay))
        mutation_outcome(client, reference)


def mutation_url_input_bounds(binary, systemd):
    with api_owned("mutation_url_input_bounds", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        mutation_hello(client, "url-bounds")
        payload = sandbox.root / "input.bin"
        payload.write_bytes(b"only local fixture bytes")
        fixture = fixtures.torrent_fixture(sandbox.root, [payload])
        (sandbox.root / "invalid.torrent").write_bytes(b"not bencoded torrent metadata")
        padding = b"x" * (16 * fixtures.MIB)
        (sandbox.root / "oversized.torrent").write_bytes(b"d7:comment" + str(len(padding)).encode() + b":" + padding + fixture.torrent.read_bytes()[1:])
        with fixtures.HttpFixture(sandbox.root) as http:
            for filename in ["invalid.torrent", "oversized.torrent"]:
                reference = filename.split(".")[0]
                reply = mutation_request(client, "torrents.add", dict(source=dict(url=http.base + "/" + filename)), ref=reference)
                case.check(reply["result"] == dict(state="pending", ref=reference), "URL input is acknowledged before fetching", json.dumps(reply))
                for event in client.events():
                    if event.get("ev") == "mutation.result" and event.get("ref") == reference:
                        case.check(event.get("outcome") == "failed" and event.get("error", {}).get("code") == "upstream",
                                   "invalid or oversized URL metainfo fails before admission", json.dumps(event))
                        break
            case.check(client.call("torrents.list", {})["result"]["torrents"] == [], "rejected URL bodies create no torrent records")


def ref_survives_engine_restart(binary, systemd):
    with api_owned("ref_survives_engine_restart", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        hello = mutation_hello(client, "engine-slot-replay")
        fixture, args = mutation_add_path(client, sandbox)
        remove = dict(hashes=[fixture.info_hash], delete_files=False)
        first = mutation_request(client, "torrents.remove", remove, ref="before-slot-restart")
        mutation_request(client, "torrents.add", args)
        mutation_request(client, "app.restart", dict(scope="engine"), ref="slot-restart")
        def restarted():
            info = client.call("app.info")["result"]
            return info["state"] == "running" and info["engine_epoch"] > hello["engine_epoch"], json.dumps(info)
        harness.wait_for(restarted, "engine slot restarts within the same daemon instance")
        replay = mutation_request(client, "torrents.remove", remove, ref="before-slot-restart", instance=hello["instance"])
        rows = client.call("torrents.list", {})["result"]["torrents"]
        case.check(first["result"] == replay["result"] and [row["hash"] for row in rows] == [fixture.info_hash],
                   "engine replacement retains the daemon-owned ref ring", json.dumps(replay))


MUTATION_CASES = (ref_replay_remove, ref_per_client, remove_requires_flag, add_url_pending_then_result,
                  duplicate_add, ref_reused_different_args, stale_instance_remove_refused,
                  stale_instance_stop_applies, hashes_cap_257, mutation_magnets_and_file_removal, mutation_url_input_bounds, ref_survives_engine_restart)

@contextmanager
def imdb_owned(name, binary, systemd, oversized=False):
    """Real daemon + local IMDb HTTP endpoints in one private cache tree."""
    import gzip
    import shutil
    with Case("daemon", name) as case, Sandbox() as sandbox:
        directory = sandbox.root / "imdb-http"
        shutil.copytree(fixtures.HERE / "fixtures/imdb", directory)
        (directory / "ratings.tsv.gz").write_bytes(gzip.compress((directory / "ratings.tsv").read_bytes(), mtime=0))
        if oversized:
            # Generated locally rather than checking an 8 MiB padding blob into git.
            poster = directory / "poster-oversized.jpg"
            poster.write_bytes((directory / "poster.jpg").read_bytes() + b"x" * (8 * 1024 * 1024))
            routes = json.loads((directory / "routes.json").read_text())
            routes["/poster.jpg"]["file"] = poster.name
            (directory / "routes.json").write_text(json.dumps(routes))
        cache = Path(sandbox.env["XDG_CACHE_HOME"]) / "fbittorrent"
        (cache / "title.ratings.tsv").write_bytes((directory / "ratings.tsv").read_bytes())
        with fixtures.imdb_server(directory) as source:
            sandbox.env["FBT_IMDB_BASE_URL"] = source.base
            sandbox.keys.append("FBT_IMDB_BASE_URL")
            case.check(urllib.parse.urlparse(source.base).hostname == "127.0.0.1" and "FBT_IMDB_BASE_URL" in sandbox.keys,
                       "local IMDb endpoint is explicitly forwarded before either launch mode", source.base)
            with Harness(case, binary, sandbox, systemd) as harness:
                process = harness.start_serve()
                if not (harness.receive_ready() and harness.wait_engine()):
                    raise RuntimeError("private IMDb daemon did not start")
                try:
                    yield case, sandbox, harness, source, directory, cache
                finally:
                    harness.stop_serve(process)
                    harness.lock_free()


def imdb_index_loaded_once(binary, systemd):
    import gzip
    with imdb_owned("imdb_index_loaded_once", binary, systemd) as (case, sandbox, harness, source, directory, cache):
        with ApiConn(harness) as client:
            hello = client.hello()
            case.check("imdb" in hello["result"]["caps"], "daemon advertises implemented IMDb capability", str(hello))
            first = client.call("imdb.lookup", dict(name="Local Fixture"))["result"]["card"]
            second = client.call("imdb.lookup", dict(id="tt1234001"))["result"]["card"]
            count = harness.stderr().count("imdb index loaded")
            case.check(count == 1 and first == second and first["rating"] == 8.2 and first["kind"] == "tvSeries",
                       "two lookups reuse one lazy index", f"imdb index loaded count={count}; cards={first!r}, {second!r}")
            poster = Path(first["poster_path"])
            case.check(poster == cache / "posters/tt1234001.jpg" and 0 < poster.stat().st_size < 8 * 1024 * 1024,
                       "poster_path is an existing bounded local id cache file", f"path={poster}; bytes={poster.stat().st_size}")
            changed = b"tconst\taverageRating\tnumVotes\ntt1234001\t9.1\t4321\n"
            (directory / "ratings.tsv.gz").write_bytes(gzip.compress(changed, mtime=0))
            refreshed = client.call("imdb.lookup", dict(id="tt1234001", refresh=True))["result"]["card"]
            case.check(refreshed["rating"] == 9.1 and refreshed["votes"] == 4321 and harness.stderr().count("imdb index loaded") == 2,
                       "forced refresh redownloads and replaces a live daemon index", str(refreshed))
            (directory / "ratings.tsv.gz").write_bytes(gzip.compress(b"tconst\taverageRating\tnumVotes\n", mtime=0))
            unknown = client.call("imdb.lookup", dict(id="tt1234001", refresh=True))["result"]["card"]
            case.check("rating" not in unknown and "votes" not in unknown,
                       "ratings absent at source remain absent after refresh", str(unknown))
            source.routes["/title.ratings.tsv.gz"] = dict(status=503, json={})
            client.call("imdb.lookup", dict(id="tt1234001", refresh=True), error="upstream")
            for args in ({}, dict(id="tt1234001", name="Local Fixture"), dict(id="tt../../unsafe"), dict(name=" ")):
                client.call("imdb.lookup", args, error="invalid_args")


def imdb_poster_cap(binary, systemd):
    with imdb_owned("imdb_poster_cap", binary, systemd, oversized=True) as (case, sandbox, harness, source, directory, cache):
        with ApiConn(harness) as client:
            client.hello()
            card = client.call("imdb.lookup", dict(id="tt1234001"))["result"]["card"]
            errors = [json.loads(line.split("WARN imdb poster ", 1)[1]) for line in harness.stderr().splitlines() if "WARN imdb poster " in line]
            size = (directory / "poster-oversized.jpg").stat().st_size
            case.check(size > 8 * 1024 * 1024 and "poster_path" not in card and not (cache / "posters/tt1234001.jpg").exists()
                       and errors and errors[-1]["code"] == "upstream" and errors[-1]["retryable"] and errors[-1]["details"]["source"] == "imdb.poster",
                       "oversized poster is refused, typed upstream diagnostic retained, card remains usable",
                       f"fixture bytes={size}; card={card!r}; error envelope={errors!r}")
            # Distinct ids avoid title-cache hits and exercise each JSON read cap.
            for route, title_id in (("/suggestion/t/tt1234002.json", "tt1234002"), ("/w/api.php", "tt1234003"),
                                    ("/api/rest_v1/page/summary/Local_Fixture", "tt1234004")):
                suggestion = json.loads((directory / "suggest.json").read_text())
                suggestion["d"][0]["id"] = title_id
                source.routes["/suggestion/t/" + title_id + ".json"] = dict(json=suggestion)
                old = source.routes.get(route)
                source.routes[route] = dict(json={"padding": "x" * (2 * 1024 * 1024)})
                failure = client.call("imdb.lookup", dict(id=title_id), error="upstream")
                case.check(failure["error"]["retryable"] and failure["error"]["details"]["source"] == "imdb",
                           "2 MiB JSON cap returns typed upstream", f"route={route}; envelope={failure!r}")
                if old is None: source.routes.pop(route, None)
                else: source.routes[route] = old


def imdb_poster_atomic(binary, systemd):
    with imdb_owned("imdb_poster_atomic", binary, systemd) as (case, sandbox, harness, source, directory, cache):
        path = cache / "posters/tt1234001.jpg"
        final = (directory / "poster.jpg").read_bytes()
        stop, started, seen_final = threading.Event(), threading.Event(), threading.Event()
        observations = []
        artifact = case.path.with_suffix(".reader-sizes.jsonl")
        def reader():
            with artifact.open("w") as output:
                while not stop.is_set():
                    try: data = path.read_bytes()
                    except FileNotFoundError: data = b""
                    observations.append(len(data))
                    output.write(json.dumps(dict(bytes=len(data))) + "\n"); output.flush()
                    started.set()
                    if data == final: seen_final.set()
                    stop.wait(.001)
        thread = threading.Thread(target=reader)
        thread.start()
        try:
            if not started.wait(2): raise TimeoutError("poster reader did not start")
            with ApiConn(harness) as client:
                client.hello()
                card = client.call("imdb.lookup", dict(id="tt1234001"))["result"]["card"]
                completed = seen_final.wait(2)
        finally:
            stop.set(); thread.join(2)
        step = case.step("observe local poster during HTTP lookup", "concurrent reader sees only absent or complete atomic poster")
        step.close(completed and not thread.is_alive() and set(observations) == {0, len(final)} and path.read_bytes() == final,
                   f"every observed size={observations}; final bytes={len(final)}; card={card!r}", [artifact])


def settings_invalid_named(binary, systemd):
    with api_owned("settings_invalid_named", binary, systemd) as (case, sandbox, h, process), ApiConn(h) as client:
        client.hello()
        before = client.call("app.settings.get")["result"]
        config = Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        saved = config.read_bytes()
        for key, value in dict(max_active_downloads=0, peer_encryption="invalid", network_interface="../bad",
                               save_path="", seed_by_default=1, autostart=None, idle_exit_secs=-1, notify="yes",
                               timeout_secs=0, handler="exec", open_command="", clipboard_watch=0,
                               reduce_motion=None, peer_port=65536, show_quality="", download_limit_bps=1,
                               upload_limit_bps=1, alt_download_limit_bps=1, alt_upload_limit_bps=1,
                               alt_limits_on=1, restart_class=[], password="not-a-setting").items():
            reply = client.call("app.settings.set", {key:value}, error="invalid_args")
            case.check(reply["error"].get("details", {}).get("key") == key,
                       "invalid settings value names its exact key", json.dumps(reply))
        client.call("app.settings.set", dict(notify=False, max_active_downloads=0), error="invalid_args")
        after = client.call("app.settings.get")["result"]
        case.check(after == before and config.read_bytes() == saved,
                   "invalid multi-key patch changes neither durable bytes nor effective settings", json.dumps(after))


def settings_live_queue(binary, systemd):
    with api_owned("settings_live_queue", binary, systemd) as (case, sandbox, h, process), ApiConn(h) as client:
        client.hello()
        epoch = client.call("app.info")["result"]["engine_epoch"]
        fixtures_created = []
        for index in range(2):
            root = sandbox.root / f"queue-payload-{index}"
            root.mkdir()
            payload = root / f"queue-{index}.bin"
            payload.write_bytes(bytes([index+1]) * (128 * 1024))
            fixture = fixtures.torrent_fixture(root, [payload], name=f"queue-{index}")
            destination = sandbox.root / f"empty-{index}"
            destination.mkdir()
            client.call("torrents.add", dict(source=dict(path=str(fixture.torrent)), stopped=True, save_path=str(destination)))
            fixtures_created.append(fixture)
        h.wait_for(lambda: (all(t["checked"] for t in client.call("torrents.list", {})["result"]["torrents"]), "both real initial checks completed"), "hash fixture torrents")
        client.call("app.settings.set", dict(max_active_downloads=1))
        client.call("torrents.start", dict(hashes=[f.info_hash for f in fixtures_created]))
        def queue_count(active, queued):
            q = json.loads(h.engine_call("app/queue")["value"])
            return q["active_downloads"] == active and q["queued_downloads"] == queued, json.dumps(q)
        h.wait_for(lambda: queue_count(1,1), "queue admits one torrent at limit one")
        applied = client.call("app.settings.set", dict(max_active_downloads=2, idle_exit_secs=0, handler="add",
                              clipboard_watch=True, open_command="mpv --keep-open=yes", seed_by_default=True,
                              upload_limit_bps=65536))["result"]
        h.wait_for(lambda: queue_count(2,0), "live increase admits queued torrent before acknowledgement")
        settings = client.call("app.settings.get")["result"]
        after = client.call("app.info")["result"]
        case.check(not applied["restart"] and after["engine_epoch"] == epoch and settings["max_active_downloads"] == 2
                   and after["policy"]["idle_exit_secs"] == 0 and settings["handler"] == "add",
                   "live settings apply without an engine epoch change", json.dumps(dict(ack=applied, settings=settings, info=after)))
        saved = tomllib.loads((Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml").read_text())
        case.check(saved["max_active_downloads"] == 2 and saved["upload_limit_bps"] == 65536,
                   "live settings acknowledgement follows durable save", json.dumps(saved))
        cli = h.cli("settings", "--max-downloads", "1")
        case.check(cli["running"] is True and cli["applied"] == ["max_active_downloads"] and not cli["restart"],
                   "running CLI settings use durable API acknowledgement", json.dumps(cli))
        h.wait_for(lambda: queue_count(1,1), "CLI queue decrease takes effect on the same engine")
        default_path = sandbox.root / "changed-downloads"
        client.call("app.settings.set", dict(save_path=str(default_path)))
        source = fixtures_created[0]
        unique = sandbox.root / "defaults.torrent"
        info = dict(source.info, name="new-defaults.bin")
        unique.write_bytes(fixtures.bencode(dict(info=info)))
        added = client.call("torrents.add", dict(source=dict(path=str(unique)), stopped=True))["result"]
        detail = client.call("torrents.get", dict(hash=added["hash"]))["result"]["torrent"]
        case.check(detail["seed"] and detail["save_path"] == str(default_path),
                   "new adds immediately use changed seed and save path defaults", json.dumps(detail))


def settings_restart_class_keeps_clients(binary, systemd):
    with api_owned("settings_restart_class_keeps_clients", binary, systemd) as (case, sandbox, h, process), ApiConn(h) as client:
        hello = client.hello()["result"]
        before = client.call("app.info")["result"]
        current = client.call("app.settings.get")["result"]
        encryption = "require" if current["peer_encryption"] != "require" else "prefer"
        reply = client.call("app.settings.set", dict(peer_encryption=encryption), ref="settings-restart")["result"]
        case.check(reply == dict(applied=["peer_encryption"], restart=True),
                   "restart-class settings return durable applied/restart, never pending", json.dumps(reply))
        def restarted():
            info = client.call("app.info")["result"]
            return info["state"] == "running" and info["engine_epoch"] > before["engine_epoch"], json.dumps(info)
        h.wait_for(restarted, "same API connection survives in-place engine restart", timeout=15)
        after = client.call("app.info")["result"]
        settings = client.call("app.settings.get")["result"]
        case.check(after["instance"] == hello["instance"] and after["server"]["pid"] == before["server"]["pid"]
                   and after["engine_epoch"] == before["engine_epoch"] + 1 and settings["peer_encryption"] == encryption,
                   "daemon identity and connection remain, only the engine epoch advances", json.dumps(dict(info=after, settings=settings)))
        duplicate = client.call("app.settings.set", dict(peer_encryption=encryption))["result"]
        case.check(duplicate == dict(applied=[],restart=False), "unchanged settings do not restart again", json.dumps(duplicate))
        case.check(not any(ev.get("ev") == "mutation.result" for ev in client.pushed),
                   "settings acknowledgement produces no pending mutation completion event", json.dumps(client.pushed))
        transition_lines = [line for line in h.stderr().splitlines() if "INFO engine state changed" in line]
        case.check(any("state=starting" in line for line in transition_lines)
                   and sum("state=running" in line for line in transition_lines) >= 2,
                   "supervisor passes Running to Starting to Running", "\n".join(transition_lines))
        case.step("record event-bus dependency", "P1-08 supplies pushed settings and engine.changed frames").ok(
            "P1-08 is absent on this branch; connected restart and durable settings verified; pushed topic/event acceptance remains pending integration.")


def settings_offline_no_start(binary, systemd):
    with Case("daemon", "settings_offline_no_start") as case, Sandbox() as sandbox, Harness(case, binary, sandbox, systemd) as h:
        fixtures.path_shim("systemctl", "exit 99", sandbox.root)
        result = h.cli("settings", "--max-downloads", "5", "--idle-secs", "0", "--handler", "add", "--clipboard-watch", "false", "--port", "6885")
        value = result
        case.check(value["running"] is False and value["max_active_downloads"] == 5 and value["idle_exit_secs"] == 0
                   and value["handler"] == "add" and value["clipboard_watch"] is False and value["peer_port"] == 6885,
                   "offline CLI settings persist the canonical values without starting the daemon", json.dumps(value))
        read = h.cli("settings")
        bad = h.cli("settings", "--max-downloads", "0", code=2)
        case.check(read["max_active_downloads"] == 5 and not h.engine_path.exists()
                   and not (sandbox.root / "shims/systemctl.log").exists(),
                   "settings reads and writes never invoke a manager or create a live engine", json.dumps(bad))


def tui_single_connection(binary, systemd):
    """Disable engine.sock to prove native rows/files travel on the one v1 connection."""
    import native
    with api_owned("tui_single_connection", binary, systemd) as (case, sandbox, harness, process), ApiConn(harness) as client:
        mutation_hello(client,"tui-observer")
        fixture, _ = mutation_add_path(client,sandbox)
        cache=Path(sandbox.env["XDG_CACHE_HOME"]) / "fbittorrent/imdb/by-name-v2/two_files.json"
        cache.write_text('{"missing":true}')
        before=client.call("app.stats")["result"]["active_clients"]
        case.check(before["tui"]==0,"no TUI presence before the real terminal starts",json.dumps(before))
        hidden=harness.engine_path.with_name("hidden-compat.sock")
        harness.engine_path.rename(hidden)
        ui=native.AttachedTui(case,str(binary),sandbox,harness,session="fbt-fbt-p4-03-r5p")
        try:
            target,done=ui.start()
            ui.assert_pane(target,"two-files")
            def one_client():
                info=client.call("app.stats")["result"]
                return info["active_clients"]["tui"]==1,json.dumps(info["active_clients"])
            harness.wait_for(one_client,"one TUI connection after loading its real API rows")
            ui.keys(target,"Enter",literal=False)
            ui.assert_pane(target,"first.bin")
            ui.assert_pane(target,"second.bin")
            ui.keys(target," ")
            def first_skipped():
                reply=client.call("torrents.files",dict(hash=fixture.info_hash))
                files=reply["result"]["files"]
                return not files[0]["included"] and files[1]["included"],json.dumps(files)
            harness.wait_for(first_skipped,"Space changes the actual engine selection over API v1")
            ui.keys(target,"Escape",literal=False)
            ui.assert_pane(target,"two-files")
            # Keep the UI through another scheduled refresh with the compat path unavailable.
            time.sleep(2.2)
            case.check("offline" not in ui.capture(target) and not harness.engine_path.exists(),
                       "native torrent and file reads remain online with engine.sock absent",ui.capture(target))
            harness.wait_for(one_client,"data and file operations retain the same one TUI connection")
            ui.keys(target,"q")
            harness.wait_for(lambda:(done.exists(),ui.capture(target)),"q detaches the API client")
            harness.wait_for(lambda:((counts:=client.call("app.stats")["result"]["active_clients"])["tui"]==0,json.dumps(counts)),
                             "TUI presence is released on detach")
        finally:
            ui.close()
            if hidden.exists(): hidden.rename(harness.engine_path)


from daemon_disk import disk_low_banner, disk_full_reason, disk_full_recovers, add_warns_disk
from daemon_inhibit import port_restart_keeps_clients, inhibitor_lifecycle
from daemon_download_limit import download_cap_holds
from daemon_peers import peers_local_swarm, peer_rates_move
from daemon_shows import shows_inbox_semantics, shows_check_never_downloads, shows_check_only_when_asked, shows_cli_edit_visible
from daemon_rss import rss_refresh_on_interactive, rss_cli_edit_visible, rss_oversized_refused, rss_grab_marks_read_only_on_success
from daemon_clipboard import (clipboard_watch_offer, clipboard_watch_add, clipboard_watch_dedupe, clipboard_watch_off_kills_child, clipboard_never_logged)
from daemon_limits import alt_limit_holds, global_unlimited_keeps_cap, alt_survives_restart

CASES = {function.__name__: function for function in
         (download_cap_holds, peers_local_swarm, peer_rates_move, shows_inbox_semantics, shows_check_never_downloads, shows_check_only_when_asked, shows_cli_edit_visible, rss_refresh_on_interactive, rss_cli_edit_visible, rss_oversized_refused, rss_grab_marks_read_only_on_success, clipboard_watch_offer, clipboard_watch_add, clipboard_watch_dedupe, clipboard_watch_off_kills_child, clipboard_never_logged, port_restart_keeps_clients, inhibitor_lifecycle, disk_low_banner, disk_full_reason, disk_full_recovers, add_warns_disk, alt_limit_holds, global_unlimited_keeps_cap, alt_survives_restart, lazy_restore_no_checks, dormant_seed_checks, dormant_missing_files, dormant_open_no_check,
          fixture_serving_states, fixture_launcher_killed, fixture_daemon_killed, fixture_stdin_lifetime, fixture_peer_pair,
          upgrade_idle_exit_75, upgrade_pending_while_busy, restart_now, restart_closed_never_starts,
          log_injected_error, log_follow, hello_counters, log_down_never_starts,
          settings_invalid_named, settings_live_queue, settings_restart_class_keeps_clients,
          settings_offline_no_start, tui_single_connection, offline_stop_ledger,
          offline_delete_keep_files_ledger, offline_write_waits_for_draining_daemon,
          notify_completed, notify_action_open, notify_action_default, notify_action_folder,
          notify_added_undo, notify_off, notify_failed, notify_queue_bounded, notify_vpn_once_per_transition, notify_cli_policy, notify_killed_handler, notify_handler_cli_once,
          dashboard_bar_presence, idle_exit_no_torrents, idle_exit_download_keeps_alive, idle_exit_interactive_keeps_alive,
          idle_exit_bar_does_not, idle_exit_parked_keeps_alive, idle_exit_disabled_never_exits,
          idle_exit_disabled_shutdown_drains, idle_exit_glance_pauses, idle_exit_glance_drop_resumes,
          idle_exit_mutation_keeps_alive, bus_subscriptions, events_since_seq_replay,
          state_watcher_cancellation_stress, mutation_bus_sequence, subscribe_gap_recovery, structural_frame_within_100ms,
          detail_topic_released, engine_restart_keeps_connection, slow_client_closed_others_live, rev_soak_10min,
          imdb_index_loaded_once, imdb_poster_cap, imdb_poster_atomic, fixture_payloads, fixture_servers, path_shims, notify_and_process, launcher_exit_cleanup,
          systemd_without_bus_address, daemon_api_probe, ready_notify, parked_start, sigterm_drains,
          panic_exits_70, second_serve_refused, engine_restart_retains_owner, shutdown_inflight_add,
          unit_is_static, add_starts_unit_no_launcher, status_never_starts, down_stops, kill9_restarts,
          killmode_mixed, start_limit_not_hit, start_while_deactivating, down_cancels_restart,
          down_failed_manager_query, down_malformed_manager_query, down_failed_query_without_unit_file,
          cleanup_failed_stop, cleanup_stop_timeout, foreground_down, unit_install_unwritable, no_manager, dashboard_routes, setup_desktop_handlers, install_atomic, startup_failure_hint, unit_exec_path,
          cap_change_no_check, seed_toggle_no_check,
          connection_limit, line_cap, non_object_line, hello_twice, hello_deadline, slow_hello_bytes,
          ninth_inflight_busy, api_quiesce_bye, api_torrent_operations, dispatch_queue_busy,
          api_concurrent_adds, api_collection_cap, api_replay_normalization, api_start_results,
          api_missing_completed_payload, api_loopback_seed_receives, deferred_add_saved_record, deferred_add,
          contract_hello, contract_errors, contract_torrents)}
CASES.update((function.__name__, function) for function in MUTATION_CASES)
SYSTEMD_CASES = {"systemd_without_bus_address", "unit_is_static", "add_starts_unit_no_launcher",
                 "status_never_starts", "down_stops", "kill9_restarts", "killmode_mixed",
                 "start_limit_not_hit", "start_while_deactivating", "down_cancels_restart",
                 "down_failed_manager_query", "down_malformed_manager_query", "down_failed_query_without_unit_file",
                 "cleanup_failed_stop", "cleanup_stop_timeout", "dashboard_routes", "install_atomic", "startup_failure_hint", "unit_exec_path"}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binary", nargs="?", type=Path, default=REPO / "target/debug/fbittorrent")
    parser.add_argument("--serve-fixture", choices=("empty", "idle", "downloading", "seeding", "paused"),
                        help="hold a private real-engine UI state until stdin EOF")
    parser.add_argument("--fixture-worker",action="store_true",help=argparse.SUPPRESS)
    parser.add_argument("--throttle",type=int,help="fixture seed upload bytes/s (0 unlimited, else at least 16384)")
    parser.add_argument("--log-root",type=Path,help="retain this invocation below a private evidence root")
    parser.add_argument("--prepare-upgrade-fixtures",action="store_true",help="build/cache both real upgrade binaries before the timed CI matrix")
    parser.add_argument("--jobs",type=int,default=4,help="bounded isolated case workers (1..4; default 4)")
    parser.add_argument("--list", action="store_true", help="list named cases without starting processes")
    parser.add_argument("--mutations", action="store_true", help="run the P1-09 mutation cases")
    parser.add_argument("--case", action="append", choices=CASES,
                        help="run named cases (repeatable); systemd_without_bus_address requires --systemd")
    parser.add_argument("--contract-validator", type=Path,
                        help="fbittorrent Rust unit-test executable for live P1-05 frame validation")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--systemd", action="store_true", help="use disposable transient user units, never installed owner units")
    mode.add_argument("--no-systemd", action="store_true", help="foreground children (default; CI mode)")
    args = parser.parse_args()
    if args.log_root or os.environ.get("FBT_TEST_LOG_ROOT"):
        import fbtlog
        fbtlog.LOG_ROOT=Path(args.log_root or os.environ["FBT_TEST_LOG_ROOT"]).resolve()
    if args.serve_fixture:
        if args.throttle is not None and args.throttle!=0 and args.throttle<16384:
            parser.error("--throttle must be 0 or at least 16384 bytes/s")
        from fixture_serving import serve, proxy
        return serve(args.binary,args.serve_fixture,args.systemd,args.throttle) if args.fixture_worker else proxy(args)
    if args.fixture_worker or args.throttle is not None:
        parser.error("fixture worker/throttle requires --serve-fixture")
    if args.contract_validator:
        os.environ["FBT_API_CONTRACT_VALIDATOR"] = str(args.contract_validator.resolve())
    if "install_atomic" in (args.case or []) and os.environ.get("FBT_TEST_INSTALL") != "1":
        parser.error("install_atomic requires FBT_TEST_INSTALL=1 and the outer release-lock/memory-scope wrapper")
    if "deferred_add" in (args.case or []) and os.environ.get("FBT_TEST_VPN") != "1":
        parser.error("deferred_add requires FBT_TEST_VPN=1 (owned VPN namespace fixture)")
    if set(args.case or []) & SYSTEMD_CASES and not args.systemd:
        parser.error(f"{args.case} requires --systemd")
    if args.list:
        print("\n".join(CASES))
        return 0
    if args.prepare_upgrade_fixtures:
        from daemon_upgrade import binaries
        with Case('daemon','upgrade_fixture_builds') as case,Sandbox() as box:
            old,new=binaries(case,box)
            print(json.dumps(dict(old=str(old),new=str(new))))
        return suite_status('daemon')
    if not 1<=args.jobs<=4:parser.error("--jobs must be 1..4")
    selected=(args.case if args.case else [function.__name__ for function in MUTATION_CASES] if args.mutations else [name for name in CASES if name != "install_atomic" and (name != "deferred_add" or os.environ.get("FBT_TEST_VPN") == "1") and (args.systemd or name not in SYSTEMD_CASES)])
    if args.jobs>1 and len(selected)>1:
        from daemon_runner import run
        return run(args.binary.resolve(),selected,args.systemd,args.jobs,args.contract_validator,
                   budget=180 if not args.case and not args.mutations and os.environ.get("FBT_TEST_SOAK")!="1" else None)
    started = time.monotonic()
    for name in selected:
        CASES[name](args.binary.resolve(), args.systemd)
    print(f"daemon harness: {time.monotonic() - started:.3f} seconds")
    return suite_status("daemon")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--replay-namespace-child":
        sys.exit(replay_namespace_child(sys.argv[2]))
    sys.exit(main())
