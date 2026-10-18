#!/usr/bin/env python3
"""Shared daemon integration harness, using real serve processes and private XDG.

Run locally or in CI with --no-systemd (default), or use --systemd for disposable
transient units and disposable static-unit links. Static-unit fixtures verify
all private XDG paths and working directory before launch and in the child.
P1-06 adds ApiConn. P1-07's fixture-serving and daemon
seed/leech follow-up waits for those items; no --serve-fixture capability is
claimed by this skeleton. There is no --self-test mode.
"""
import argparse
import base64
import ctypes
import fcntl
from contextlib import ExitStack
import hashlib
import json
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
import time
from typing import NamedTuple
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
    """Process contract; future API assumptions remain confined to this adapter.

    No v1 messages are implemented here. ApiConn belongs to P1-06; fixture
    recipes, transfer.set_limits and the daemon peer pair follow that API.
    """
    unit_variable = "FBITTORRENT_UNIT"

    @staticmethod
    def command(binary, idle_exit_secs=300, no_notify=False):
        return [str(binary), "serve", "--idle-exit-secs", str(idle_exit_secs), *(["--no-notify"] if no_notify else [])]

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
        return self.socket.recv(65536).decode()

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
        """Temporary legacy framing until P1-06's API v1 connection exists."""
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
        return self.wait_for(lambda: ((response := self.engine_call("app/version")).get("error") is None,
                                     f"Legacy version response={response!r}."), "engine session answers")

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

    def cli(self, *args, code=0, timeout=10, start=False):
        private_environment(self.sandbox)
        command = [self.binary, *([] if start else ["--no-start"]), *args, "--json"]
        step = self.case.step("CLI " + shlex.join(command), f"exit {code} with one JSON object")
        child = self.case.popen(command, env=self.sandbox.env, cwd=self.sandbox.root, stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=timeout)
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


# Keep the original TUI-hosted socket case as a compatibility regression while
# P1-15 moves TUI attachment. It shares the legacy wire helper with serve cases.
class InterimEngine:
    def __init__(self, harness):
        self.harness = harness
        self.path = Path(harness.sandbox.env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
        self.server = "fbt-fbt-p1-03-ghy-" + harness.sandbox.root.name
        self.session = "fbt-fbt-p1-03-ghy"

    def call(self, path):
        return self.harness.engine_call(path, timeout=.5)

    def start(self):
        # Keep a foreground owner for the detached tmux server in either mode.
        # The child handles SIGTERM so its finally block also kills that server.
        program = '''import os,shlex,signal,subprocess,sys,time
server,session,binary,stderr,exitfile=sys.argv[1:]
tmux=['tmux','-L',server,'-f','/dev/null']
def stop(*_): raise SystemExit(143)
signal.signal(signal.SIGTERM,stop)
launch=shlex.join([binary,'tui'])+' 2>>'+shlex.quote(stderr)+'; result=$?; printf "%s" "$result" >'+shlex.quote(exitfile)+'; exit "$result"'
try:
    subprocess.run(tmux+['new-session','-d','-s',session,'-x','120','-y','30',launch],check=True)
    while subprocess.run(tmux+['has-session','-t',session],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0: time.sleep(.05)
    sys.exit(int(open(exitfile).read()) if os.path.exists(exitfile) else 1)
finally:
    subprocess.run(tmux+['kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
'''
        h = self.harness
        self.process = h.spawn([sys.executable, "-c", program, self.server, self.session, h.binary,
                                str(h.case.suite.stderr_path), str(h.sandbox.root / "tui.exit")], "interim")
        h.resources.callback(self.close)

    def close(self):
        if self.process.child.poll() is None and self.path.exists():
            try:
                self.harness.engine_call("app/shutdown", method="POST")
            except (OSError, ValueError, EOFError):
                pass
        # The generic spawner reaps its owner after this callback returns.


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


def interim_engine_socket(binary, systemd):
    with Case("daemon", "interim_engine_socket") as case, Sandbox() as sandbox, \
            fixtures.magnet_fixture() as fixture, \
            Harness(case, binary, sandbox, systemd) as harness:
        status = harness.cli("status")
        case.check(status.get("backend") == "stopped" and status.get("app_open") is False,
                   "fresh isolated engine is closed before launch", f"Status={status!r}.")
        engine = InterimEngine(harness)
        engine.start()
        def available():
            value = engine.call("app/version")
            version = value.get("value")
            return isinstance(version, str) and version.startswith("native librqbit"), f"Response={value!r}."
        ready = harness.wait_for(available, "current engine framed socket answers app/version")
        if ready:
            case.check(engine.path.stat().st_mode & 0o777 == 0o600,
                       "current engine socket is private to this sandbox", str(engine.path))
            status = harness.cli("status")
            case.check(status.get("ok") is True and status.get("app_open") is True,
                       "real CLI agrees that the isolated current engine is open", f"Status={status!r}.")
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
            harness.cli("down")
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
            seed.wait_for(lambda: ((row := seed.cli("show", fixture.info_hash)).get("complete") is True,
                                  f"Seed state={row.get('state')}; complete={row.get('complete')}"), "seed verifies the real fixture")
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
        for args in (("list",), ("show", "abcd"), ("stop", "all"), ("delete", "all")):
            value = h.cli(*args, start=True, code=4)
            case.check(value.get("hint") == "fbittorrent up" and unit.state() == "inactive", "read/offline-intent command does not start the daemon", str(value))
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
        case.check(" dashboard\n" in app and " add %u\n" in magnet and "Terminal=false" in magnet,
                   "setup writes dashboard and direct headless magnet desktop entries", app + magnet)
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


CASES = {function.__name__: function for function in
         (fixture_payloads, fixture_servers, path_shims, notify_and_process, launcher_exit_cleanup,
          systemd_without_bus_address, interim_engine_socket, ready_notify, parked_start, sigterm_drains,
          panic_exits_70, second_serve_refused, engine_restart_retains_owner, shutdown_inflight_add,
          unit_is_static, add_starts_unit_no_launcher, status_never_starts, down_stops, kill9_restarts,
          killmode_mixed, start_limit_not_hit, start_while_deactivating, down_cancels_restart,
          cleanup_failed_stop, cleanup_stop_timeout, foreground_down, unit_install_unwritable, no_manager, dashboard_routes, setup_desktop_handlers, install_atomic, startup_failure_hint, unit_exec_path)}
SYSTEMD_CASES = {"systemd_without_bus_address", "unit_is_static", "add_starts_unit_no_launcher",
                 "status_never_starts", "down_stops", "kill9_restarts", "killmode_mixed",
                 "start_limit_not_hit", "start_while_deactivating", "down_cancels_restart",
                 "cleanup_failed_stop", "cleanup_stop_timeout", "dashboard_routes", "install_atomic", "startup_failure_hint", "unit_exec_path"}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binary", nargs="?", type=Path, default=REPO / "target/debug/fbittorrent")
    parser.add_argument("--list", action="store_true", help="list named cases without starting processes")
    parser.add_argument("--case", choices=CASES,
                        help="run one named case; systemd_without_bus_address requires --systemd")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--systemd", action="store_true", help="use disposable transient user units, never installed owner units")
    mode.add_argument("--no-systemd", action="store_true", help="foreground children (default; CI mode)")
    args = parser.parse_args()
    if args.case == "install_atomic" and os.environ.get("FBT_TEST_INSTALL") != "1":
        parser.error("install_atomic requires FBT_TEST_INSTALL=1 and the outer release-lock/memory-scope wrapper")
    if args.case in SYSTEMD_CASES and not args.systemd:
        parser.error(f"{args.case} requires --systemd")
    if args.list:
        print("\n".join(CASES))
        return 0
    started = time.monotonic()
    for name in ([args.case] if args.case else [name for name in CASES if name != "install_atomic" and (args.systemd or name not in SYSTEMD_CASES)]):
        CASES[name](args.binary.resolve(), args.systemd)
    print(f"daemon harness: {time.monotonic() - started:.3f} seconds")
    return suite_status("daemon")


if __name__ == "__main__":
    sys.exit(main())
