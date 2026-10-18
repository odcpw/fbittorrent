#!/usr/bin/env python3
"""Shared daemon integration harness; today exercises the real legacy engine socket.

Run locally or in CI with --no-systemd (default), or use --systemd for disposable
transient user units. No installed unit or owner XDG directory is used.
P1-03 adds daemon cases; P1-06 adds ApiConn. P1-07's fixture-serving and daemon
seed/leech follow-up waits for those items; no --serve-fixture capability is
claimed by this skeleton. There is no --self-test mode.
"""
import argparse
import ctypes
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

from fbtlog import Case, REPO, slug, suite_status
import fixtures


def process_state(pid):
    try:
        return (Path("/proc") / str(pid) / "stat").read_text().rsplit(") ", 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return None


def process_alive(pid):
    state = process_state(pid)
    return state is not None and state[0] not in ("Z", "X")


class FutureDaemon:
    """FUTURE DAEMON ASSUMPTIONS — only this adapter names the P1-03 contract.

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
        command = list(map(str, argv))
        environment = dict(sandbox.env, **{FutureDaemon.unit_variable: self.unit})
        launcher_env = environment.copy()
        self.closed = False
        # systemd's client must discover the real user bus even when no explicit
        # DBUS_SESSION_BUS_ADDRESS exists. Only the service gets sandbox XDG paths.
        self.control_env = dict(os.environ)
        self.control_env.pop("NOTIFY_SOCKET", None)
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
        step.close(not failures, "; ".join(observations + failures) + ".")


class Harness:
    def __init__(self, case, binary, sandbox, systemd=False):
        self.case, self.binary, self.sandbox, self.systemd = case, str(binary), sandbox, systemd
        self.resources = ExitStack()
        self.names = {}

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.resources.close()

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
        """Spawner for P1-03's real binary; intentionally unused until serve exists."""
        self.notify_socket()
        return self.spawn(FutureDaemon.command(self.binary, idle_exit_secs, no_notify))

    def cli(self, *args, code=0, timeout=10):
        command = [self.binary, "--no-start", *args, "--json"]
        step = self.case.step("CLI " + shlex.join(command), f"exit {code} with one JSON object")
        child = self.case.popen(command, env=self.sandbox.env, stdout=subprocess.PIPE, text=True)
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


# INTERIM CURRENT-ENGINE ADAPTER: the pre-daemon engine uses 4-byte lengths,
# not API v1 NDJSON. Remove this probe when P1-03 supplies real daemon cases.
class InterimEngine:
    def __init__(self, harness):
        self.harness = harness
        self.path = Path(harness.sandbox.env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
        self.server = "fbt-fbt-p1-07-0h3-" + harness.sandbox.root.name
        self.session = "fbt-fbt-p1-07-0h3"

    def call(self, path):
        request = json.dumps(dict(method="GET", path=path, args=[], file=None)).encode()
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(.5)
            connection.connect(str(self.path))
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
            if size > 1024 * 1024:
                raise ValueError(f"interim probe frame too large: {size}")
            return json.loads(read(size))

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
                self.call("app/shutdown")
            except (OSError, ValueError, EOFError):
                pass
        # The generic spawner reaps its owner after this callback returns.


def fixture_payloads(binary, systemd):
    with Case("daemon", "fixture_payloads") as case, fixtures.Sandbox() as sandbox, \
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
    with Case("daemon", "path_shims") as case, fixtures.Sandbox() as sandbox:
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
    with Case("daemon", "notify_and_process") as case, fixtures.Sandbox() as sandbox, \
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
                   and value[FutureDaemon.unit_variable] == sandbox.unit,
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
    with Case("daemon", "launcher_exit_cleanup") as case, fixtures.Sandbox() as sandbox, \
            Harness(case, binary, sandbox, systemd) as harness:
        # Adopt and reap the foreground orphan in the regression itself. This
        # avoids depending on PID 1's reaping policy in CI/container environments.
        libc, prior = ctypes.CDLL(None, use_errno=True), ctypes.c_int()
        if libc.prctl(37, ctypes.byref(prior), 0, 0, 0) or libc.prctl(36, 1, 0, 0, 0):
            raise OSError(ctypes.get_errno(), "cannot establish regression child subreaper")
        process, descendant, service_pid = None, None, None
        control_env = dict(os.environ)
        control_env.pop("NOTIFY_SOCKET", None)
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
            with fixtures.Sandbox() as sandbox, Harness(case, binary, sandbox, systemd=True) as harness:
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
    with Case("daemon", "interim_engine_socket") as case, fixtures.Sandbox() as sandbox, \
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


CASES = {function.__name__: function for function in
         (fixture_payloads, fixture_servers, path_shims, notify_and_process, launcher_exit_cleanup,
          systemd_without_bus_address, interim_engine_socket)}
SYSTEMD_CASES = {"systemd_without_bus_address"}


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
    if args.case in SYSTEMD_CASES and not args.systemd:
        parser.error(f"{args.case} requires --systemd")
    if args.list:
        print("\n".join(CASES))
        return 0
    started = time.monotonic()
    for name in ([args.case] if args.case else [name for name in CASES if args.systemd or name not in SYSTEMD_CASES]):
        CASES[name](args.binary.resolve(), args.systemd)
    print(f"daemon harness: {time.monotonic() - started:.3f} seconds; serve-fixture/peer pair deferred until P1-06")
    return suite_status("daemon")


if __name__ == "__main__":
    sys.exit(main())
