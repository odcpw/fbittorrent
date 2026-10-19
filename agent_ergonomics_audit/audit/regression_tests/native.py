#!/usr/bin/env python3
"""Exercise the real native CLI/TUI with local RSS, saved Shows and torrent fixtures.

Every config, download and socket belongs to a temporary directory. No user's
service or torrents are touched. Requires tmux, util-linux script and Python 3.
"""
import argparse
import contextlib
import hashlib
import http.server
import io
import json
import os
import re
import signal
from datetime import datetime, timedelta, timezone
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
from unittest.mock import patch

import daemon
import fixtures
import fbtlog
from fbtlog import Case, REPO, suite_status

CASES = (
    "dec_mouse_mode", "closed_status_stays_closed", "one_enter_adds_once",
    "refresh_preserves_read_history", "rss_mouse_and_scoped_shortcuts",
    "mouse_selection_at_scroll_offset", "native_cli_fields_and_deletion_safety", "no_fake_seed_counts",
    "resume_done_refused", "bulk_start_requires_hashes", "shows_inbox_and_targets", "file_modal_shows_help_80x24",
    "settings_queue_vpn_restart", "quit_restart_down", "resume_unknown_timestamp", "launcher_never_called", "terminal_close_is_graceful",
    "store_modes", "startup_survives_bad_record", "stop_during_hashing",
)


def launcher_never_called(binary):
    """Probe the widget's closed-app paths with autostart enabled."""
    with Case("native", "launcher_never_called") as case, \
            tempfile.TemporaryDirectory(prefix="fbt-no-start-") as temporary, \
            socket.socket() as legacy_endpoint:
        root = Path(temporary)
        env = dict(os.environ)
        env.pop("FBITTORRENT_NO_START", None)
        home = root / "home"
        home.mkdir()
        env["HOME"] = str(home)
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
                    "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            directory = root / key.lower()
            directory.mkdir(mode=0o700)
            env[key] = str(directory)
        config = Path(env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        config.parent.mkdir()
        config_text = ('engine = "native"\nautostart = true\nnotify = false\n'
                       'network_interface = "fbmissing0"\n')
        config.write_text(config_text)
        shims = root / "shims"
        shims.mkdir()
        env["PATH"] = str(shims) + os.pathsep + env.get("PATH", "")
        calls = case.path.with_suffix(".launcher.log")
        calls.write_text("")
        for name in ("omarchy-launch-or-focus-tui", "systemctl"):
            shim = shims / name
            shim.write_text("#!/bin/sh\nprintf '%s\\n' " + shlex.quote(name) +
                            ' "$@" >> ' + shlex.quote(str(calls)) + "\nexit 99\n")
            shim.chmod(0o700)
        clipboard = shims / "wl-paste"
        clipboard.write_text("#!/bin/sh\nprintf '%s' 'magnet:?xt=urn:btih:" + "1" * 40 + "'\n")
        clipboard.chmod(0o700)

        probes = (
            (("status",), 0, {}, "stopped"),
            (("reconcile", "--recent", "180"), 0, {}, "intrinsic"),
            (("--no-start", "list"), 0, {}, "offline"),
            (("list", "--no-start"), 0, {}, "offline"),
            (("list",), 0, {"FBITTORRENT_NO_START": "1"}, "offline"),
            (("--no-start", "stop", "all"), 0, {}, "offline"),
            (("--no-start", "up"), 4, {}, "closed"),
            (("up",), 4, {"FBITTORRENT_NO_START": "1"}, "closed"),
            (("--no-start", "status"), 0, {}, "stopped"),
            (("--no-start", "start", "all"), 4, {}, "closed"),
            (("--no-start", "delete", "all"), 0, {}, "offline"),
            (("--no-start", "open", "deadbeef"), 4, {}, "closed"),
            (("--no-start", "seed", "all"), 4, {}, "closed"),
            (("--no-start", "noseed", "all"), 4, {}, "closed"),
            (("--no-start", "add-clipboard"), 4, {}, "closed"),
            (("--no-start", "add", "magnet:?xt=urn:btih:" + "1" * 40), 4, {}, "closed"),
        )
        for args, code, overrides, outcome in probes:
            argv = [binary, *args, "--json"]
            step = case.step("CLI " + shlex.join(argv),
                             f"Exit {code}, {outcome} JSON, and zero launcher calls.")
            child = case.popen(argv, env=dict(env, **overrides),
                               stdout=subprocess.PIPE, text=True)
            try:
                stdout, _ = child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                stdout, _ = child.communicate()
                step.fail(f"argv={argv!r}, env={overrides!r}; timed out; stdout={stdout!r}", [calls])
                continue
            try:
                value = json.loads(stdout)
            except ValueError:
                value = {}
            if outcome == "stopped":
                expected = value.get("backend") == "stopped" and value.get("app_open") is False
            elif outcome == "offline":
                expected = value.get("ok") is True and value.get("running") is False and value.get("count") == 0
            elif outcome == "intrinsic":
                expected = value == dict(ok=True, touched=0, note="native scheduling is intrinsic")
            else:
                expected = (value.get("ok") is False and value.get("code") == 4 and
                            value.get("code_name") == "backend" and
                            "fbittorrent is closed" in value.get("error", "") and
                            "fbittorrent up" in value.get("hint", ""))
            step.close(child.returncode == code and expected and not calls.read_text(),
                       f"argv={argv!r}, env={overrides!r}; exit {child.returncode}; "
                       f"JSON={value!r}; launcher calls={calls.read_text()!r}.", [calls])
        # Service.qml collects action errors from stderr without --json.
        argv = [binary, "--no-start", "stop", "all"]
        step = case.step("Widget action " + shlex.join(argv),
                         "Exit 0, empty offline stop count on stdout, no stderr error and zero launcher calls.")
        offset = case.suite.stderr_path.stat().st_size
        child = case.popen(argv, env=env, stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=5)
            stderr = case.suite.stderr_path.read_bytes()[offset:].decode(errors="replace")
            step.close(child.returncode == 0 and "stop: 0 torrent(s)" in stdout and not stderr
                       and not calls.read_text(),
                       f"argv={argv!r}; exit {child.returncode}; stdout={stdout!r}; "
                       f"stderr={stderr!r}; launcher calls={calls.read_text()!r}.", [calls])
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate()
            step.fail("The widget action timed out.", [calls])
        argv = [binary, "--no-start", "tui", "--json"]
        step = case.step("CLI in a real terminal: " + shlex.join(argv),
                         "Exit 4 with a closed-engine hint and zero launcher calls.")
        tmux = ["tmux", "-L", f"fbt-no-start-{os.getpid()}", "-f", "/dev/null"]
        session = "fbt-fbt-p0-07-563"
        exit_path = root / "tui.exit"
        transcript = case.path.with_suffix(".tui.log")
        command = (shlex.join(argv) + '\nprintf \'%s\' "$?" > ' +
                   shlex.quote(str(exit_path)) + "\nsleep 15")
        try:
            child = case.popen([*tmux, "new-session", "-d", "-s", session,
                                "-x", "160", "-y", "24", command], env=env)
            child.wait(timeout=5)
            deadline = time.monotonic() + 5
            while not exit_path.exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            capture = case.popen([*tmux, "capture-pane", "-pt", session], env=env,
                                  stdout=subprocess.PIPE, text=True)
            stdout, _ = capture.communicate(timeout=5)
            transcript.write_text(stdout)
            value = next((json.loads(line) for line in stdout.splitlines()
                          if line.startswith("{")), {})
            code = exit_path.read_text() if exit_path.exists() else "timed out"
            step.close(code == "4" and value.get("code_name") == "backend" and
                       "fbittorrent is closed" in value.get("error", "") and
                       "fbittorrent up" in value.get("hint", "") and not calls.read_text(),
                       f"argv={argv!r}; exit {code}; JSON={value!r}; "
                       f"launcher calls={calls.read_text()!r}.", [calls, transcript])
        finally:
            cleanup = case.popen([*tmux, "kill-server"], env=env)
            cleanup.wait(timeout=5)
        case.check(config.read_text() == config_text,
                   "Invocation flags leave the stored autostart setting enabled.")
        case.check(not (Path(env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock").exists(),
                   "No probe has started a native engine socket.")
        legacy_endpoint.bind(("127.0.0.1", 0))
        legacy_config = ('engine = "qbittorrent"\nautostart = true\nnotify = false\ntimeout_secs = 1\n'
                         f'url = "http://127.0.0.1:{legacy_endpoint.getsockname()[1]}"\n')
        config.write_text(legacy_config)
        for args, overrides in ((("--no-start", "setup"), {}),
                                (("setup",), {"FBITTORRENT_NO_START": "1"})):
            argv = [binary, *args, "--json"]
            step = case.step("Legacy setup " + shlex.join(argv),
                             "Typed exit 4 with the opening hint, zero systemctl calls, and no setup writes.")
            child = case.popen(argv, env=dict(env, **overrides), stdout=subprocess.PIPE, text=True)
            try:
                stdout, _ = child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.communicate()
                step.fail("Legacy setup timed out.", [calls])
                continue
            value = json.loads(stdout)
            step.close(child.returncode == 4 and value.get("code") == 4 and
                       value.get("code_name") == "backend" and value.get("hint") == "fbittorrent up" and
                       value.get("error") == "legacy setup cannot start qBittorrent while --no-start or FBITTORRENT_NO_START=1 is active" and
                       not calls.read_text() and not (home / ".config").exists() and
                       config.read_text() == legacy_config,
                       f"argv={argv!r}, env={overrides!r}; exit {child.returncode}; JSON={value!r}; "
                       f"launcher/systemctl calls={calls.read_text()!r}; setup writes={(home / '.config').exists()}.", [calls])

        # Ordinary setup must still reach its explicit service start. Refuse
        # that start in a recording shim; no real systemd service is controlled.
        control = case.path.with_suffix(".setup-systemctl.log")
        control.write_text("")
        (shims / "systemctl").write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> " +
            shlex.quote(str(control)) + '\n[ "$2" != start ]\n')
        step = case.step("Ordinary legacy setup without startup restrictions",
                         "Installed nox reaches the recording service-start refusal; missing nox retains its dependency error.")
        child = case.popen([binary, "setup", "--save", str(root / "downloads"), "--json"],
                           env=env, stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=5)
            value = json.loads(stdout)
            if Path("/usr/bin/qbittorrent-nox").exists():
                expected_calls = ["--user daemon-reload", "--user disable qbittorrent-nox.service",
                                  "--user start qbittorrent-nox.service"]
                ok = (child.returncode == 1 and control.read_text().splitlines() == expected_calls and
                      value.get("error") == "systemctl --user start qbittorrent-nox.service failed")
            else:
                ok = child.returncode == 4 and "qbittorrent-nox is not installed" in value.get("error", "") and not control.read_text()
            step.close(ok and not calls.read_text(),
                       f"exit {child.returncode}; JSON={value!r}; systemctl argv={control.read_text()!r}; "
                       f"automatic-path launcher calls={calls.read_text()!r}.", [control, calls])
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate()
            step.fail("Ordinary setup timed out.", [control, calls])



def terminal_close_is_graceful(binary):
    with Case("native", "terminal_close_is_graceful") as case, tempfile.TemporaryDirectory(prefix="fbt-hup-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOME=str(root / "home"), TERM="xterm-256color", FBT_DEBUG_SIGNALS="1")
        for key in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            if key != "HOME": env[key] = str(root / key.lower())
            Path(env[key]).mkdir(mode=0o700)
        env.pop("FBITTORRENT_NO_START", None)
        shims = root / "shims"
        shims.mkdir()
        for name in ("systemctl", "notify-send", "xdg-open", "wl-paste", "wl-copy", "systemd-inhibit", "omarchy-launch-or-focus-tui"):
            path = shims / name
            path.write_text("#!/bin/sh\nexit 3\n")
            path.chmod(0o700)
        env["PATH"] = str(shims) + os.pathsep + env["PATH"]
        config = Path(env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        config.parent.mkdir()
        with socket.socket() as port:
            port.bind(("127.0.0.1", 0))
            peer_port = port.getsockname()[1]
        config.write_text(f'engine = "native"\nautostart = false\nnotify = false\nnetwork_interface = "lo"\n'
                          f'peer_port = {peer_port}\nurl = "http://127.0.0.1:9"\n'
                          f'save_path = {json.dumps(str(root / "downloads"))}\n')
        stderr = case.path.with_suffix(".tui.stderr")
        stderr.write_text("")
        trace = case.path.with_suffix(".signals.log")
        source_trace = Path(env["XDG_STATE_HOME"]) / "fbittorrent/tui-signals.log"
        status_path, pid_path = root / "exit.json", root / "pid"
        supervisor = root / "supervise.py"
        # Keep a parent alive after the real pty is hung up, so the test can
        # observe the app's wait status rather than infer it from socket removal.
        supervisor.write_text('''import json, os, signal, subprocess, sys
from pathlib import Path
signal.signal(signal.SIGHUP, signal.SIG_IGN)
def prepare_child():
    signal.signal(signal.SIGHUP, signal.SIG_DFL)
with open(sys.argv[4], "ab", buffering=0) as err:
    child = subprocess.Popen([sys.argv[1], "tui"], stderr=err,
        preexec_fn=prepare_child)
    forwarded = []
    def hangup(*_):
        forwarded.append("SIGHUP")
        try: os.kill(child.pid, signal.SIGHUP)
        except ProcessLookupError: pass
    signal.signal(signal.SIGHUP, hangup)
    Path(sys.argv[3]).write_text(str(child.pid))
    code = child.wait()
    Path(sys.argv[2]).write_text(json.dumps({"returncode": code, "terminal_signals_forwarded": forwarded}))
''')
        tmux_base = ["tmux", "-L", f"fbt-p0-08-{os.getpid()}", "-f", "/dev/null"]
        session = "fbt-fbt-p0-08-plg"
        def tmux(*args):
            child = case.popen([*tmux_base, *args], env=env, stdout=subprocess.PIPE, text=True)
            out, _ = child.communicate(timeout=5)
            return child.returncode, out.strip()
        def cli(*args):
            child = case.popen([binary, *args, "--json"], env=env, stdout=subprocess.PIPE, text=True)
            out, _ = child.communicate(timeout=5)
            return child.returncode, json.loads(out)
        socket_path = Path(env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
        lock_path = socket_path.with_name("engine.lock")
        owner = NativeHarness(lambda: case, binary, env)
        try:
            owner.ensure_serve()
            tmux("new-session", "-d", "-s", session, "-n", "keeper", "sleep 120")
            launch = shlex.join([sys.executable, str(supervisor), binary, str(status_path), str(pid_path), str(stderr)])
            code, window = tmux("new-window", "-d", "-P", "-F", "#{window_id}", "-t", session, launch)
            step = case.step("start the native TUI on a disposable tmux pty", "An open native engine socket within 10 seconds.")
            deadline = time.monotonic() + 10
            screen = ""
            while not status_path.exists() and time.monotonic() < deadline:
                _, screen = tmux("capture-pane", "-p", "-t", window)
                if socket_path.exists() and "Torrents" in screen: break
                time.sleep(.1)
            step.close(code == 0 and socket_path.exists() and "Torrents" in screen,
                       f"tmux exit {code}; window={window}; socket={socket_path.exists()}; screen={screen!r}", [stderr])
            # A private, trackerless, unfinished torrent keeps a real download
            # handle alive without contacting a public tracker or peer.
            info = {"name": "terminal-close.fixture", "length": 65536, "piece length": 16384,
                    "pieces": hashlib.sha1(b"x" * 16384).digest() * 4, "private": 1}
            torrent = root / "fixture.torrent"
            torrent.write_bytes(bencode({"info": info}))
            code, added = cli("add", str(torrent), "--quiet")
            case.check(code == 0 and added.get("ok") is True, "A real unfinished torrent is admitted before terminal close.", json.dumps(added))
            owner.wait_for(lambda: owner.cli("show", added["hash"])["state"] in ("stalledDL", "downloading"),
                           "unfinished fixture is active before terminal close")
            step = case.step("check the daemon owns engine.lock before closing the TUI terminal", "flock -n refuses the held instance lock.")
            probe = case.popen(["flock", "-n", str(lock_path), "true"], env=env)
            lock_code = probe.wait(timeout=5)
            step.close(lock_code == 1, f"flock exit {lock_code} while the app is open.")
            step = case.step("tmux kill-window " + window,
                             "TUI exits 129 within 6 seconds while the daemon socket and instance lock remain live.")
            began = time.monotonic()
            kill_code, _ = tmux("kill-window", "-t", window)
            acquired, samples = False, []
            while time.monotonic() - began < 6:
                probe = case.popen(["flock", "-n", str(lock_path), "true"], env=env)
                acquired = probe.wait(timeout=1) == 0
                samples.append(dict(ms=round((time.monotonic()-began)*1000), socket=socket_path.exists(), lock=acquired))
                if not acquired and socket_path.exists() and status_path.exists(): break
                time.sleep(.1)
            elapsed = time.monotonic() - began
            status = json.loads(status_path.read_text()) if status_path.exists() else {}
            trace.write_text(source_trace.read_text() if source_trace.exists() else "<no signal trace>\n")
            diagnostics = stderr.read_text()
            download = cli("show", added["hash"])[1]
            step.close(kill_code == 0 and elapsed <= 6 and not acquired and socket_path.exists() and
                       status.get("returncode") == 129 and "engine drained in " not in diagnostics and
                       download["state"] in ("stalledDL", "downloading"),
                       f"elapsed={elapsed:.3f}s; status={status}; download={download}; polls={samples}; tui.stderr tail={diagnostics[-2000:]!r}; "
                       f"signals={trace.read_text()!r}", [stderr, trace])
        finally:
            if pid_path.exists() and not status_path.exists():
                pid = int(pid_path.read_text())
                try:
                    if ("XDG_RUNTIME_DIR=" + env["XDG_RUNTIME_DIR"]).encode() in Path(f"/proc/{pid}/environ").read_bytes().split(b"\0"):
                        os.kill(pid, signal.SIGKILL)
                except FileNotFoundError:
                    pass
            tmux("kill-server")
            owner.close_serve()


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, str):
        value = value.encode()
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, dict):
        return b"d" + b"".join(bencode(k) + bencode(value[k]) for k in sorted(value)) + b"e"
    return b"l" + b"".join(bencode(item) for item in value) + b"e"


def prepare_imdb_fixture(env, names):
    """Unrelated IMDb lookups stay cache-only in these local torrent fixtures."""
    cache = Path(env["XDG_CACHE_HOME"]) / "fbittorrent"
    by_name = cache / "imdb/by-name-v2"
    by_name.mkdir(parents=True, exist_ok=True)
    for path in (cache, cache / "imdb", by_name):
        path.chmod(0o700)
    for name in names:
        key = re.sub(r"[^a-z0-9]", "_", name.lower())[:120]
        path = by_name / (key + ".json")
        path.write_text('{"missing":true}')
        path.chmod(0o600)
    ratings = cache / "title.ratings.tsv"
    if not ratings.exists():
        ratings.write_text("tconst\taverageRating\tnumVotes\n")
        ratings.chmod(0o600)


class NativeHarness:
    """Record native CLI calls and retain only the asserted polling captures."""

    def __init__(self, get_case, binary, env):
        self.get_case, self.binary, self.env = get_case, binary, env
        self.polling, self.poll_records = False, []

    def ensure_serve(self):
        """Existing adapter cases own a disposable foreground daemon explicitly."""
        path = Path(self.env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
        if getattr(self, "serve", None) is not None and self.serve.poll() is None:
            return
        if not Path(self.binary).exists():
            # Logging-contract fixtures pass a placeholder binary; they exercise tmux and logs, never a daemon.
            return
        # The sandbox root: fixtures that keep the real XDG_RUNTIME_DIR (/run/user/<uid>) must not
        # derive scratch paths from it, so use the sandboxed data dir's parent, else a private tempdir.
        data = self.env.get("XDG_DATA_HOME")
        root = Path(data).parent if data and os.access(Path(data).parent, os.W_OK) else Path(tempfile.mkdtemp(prefix="fbt-native-"))
        # These foreground cases never control the user's service manager.
        self.env["FBITTORRENT_UNIT"] = "fbt-native-" + str(os.getpid()) + ".service"
        self.env["HOME"] = str(root / "home")
        Path(self.env["HOME"]).mkdir(exist_ok=True, mode=0o700)
        shim = root / "native-shims"
        shim.mkdir(exist_ok=True)
        control = shim / "systemctl"
        control.write_text("#!/bin/sh\ncase \"$*\" in *daemon-reload*) exit 0;; *show*) printf 'LoadState=not-found\\nActiveState=inactive\\nSubState=dead\\nMainPID=0\\nJob=\\n';; *) exit 99;; esac\n")
        control.chmod(0o700)
        if str(shim) not in self.env.get("PATH", "").split(os.pathsep):
            self.env["PATH"] = str(shim) + os.pathsep + self.env.get("PATH", "")
        ledger = Path(self.env["XDG_DATA_HOME"]) / "fbittorrent/engine/torrents.json"
        names = [record["name"] for record in json.loads(ledger.read_text())["records"]] if ledger.exists() else []
        prepare_imdb_fixture(self.env, names + ["terminal-close.fixture"])
        self.serve = self.get_case().popen([self.binary, "serve", "--idle-exit-secs", "600"],
                                          env=self.env, stdout=subprocess.DEVNULL)
        self.wait_for(lambda: path.exists() and self.serve.poll() is None, "disposable daemon listener opens")
        def ready():
            try:
                response = daemon.Harness.engine_call(self, "app/version", timeout=1)
                return isinstance(response.get("value"), str) and not response.get("error")
            except (OSError, EOFError):
                return False
        self.wait_for(ready, "disposable daemon answers its legacy version request")

    def api(self, op, args=None):
        """Read real daemon state independently of the CLI's legacy/API shaper."""
        adapter = SimpleNamespace(case=self.get_case(), sandbox=SimpleNamespace(env=self.env))
        with daemon.ApiConn(adapter) as connection:
            connection.hello()
            return connection.call(op, {} if args is None else args)["result"]

    @property
    def engine_path(self):
        return Path(self.env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"

    def close_serve(self):
        process = getattr(self, "serve", None)
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait(timeout=2)
                self.record("clean up owned daemon", "daemon drains within ten seconds", "required SIGKILL", False)

    def record(self, action, expect, observed, ok=True, started=None):
        if self.polling and action == "capture terminal pane" and ok:
            self.poll_records.append((action, expect, observed, ok, started))
            if os.environ.get("FBT_TEST_VERBOSE") != "1":
                return
        step = self.get_case().step(action, expect)
        if started is not None:
            step.started = started
        step.close(ok, observed)

    def wait_for(self, check, label, seconds=10):
        started = time.monotonic()
        end = started + seconds
        while time.monotonic() < end:
            self.poll_records.clear()
            self.polling = True
            try:
                result = check()
            finally:
                self.polling = False
            if result:
                break
            time.sleep(0.1)
        else:
            result = False
        if os.environ.get("FBT_TEST_VERBOSE") != "1":
            for observation in self.poll_records:
                self.record(*observation)
        self.record("wait for " + label, f"{label} within {seconds} seconds",
                    f"Condition {'satisfied' if result else 'timed out'} after {time.monotonic()-started:.3f} seconds.",
                    bool(result), started)
        return result

    def start_tmux(self, tmux, session, launch, seconds=10):
        started = time.monotonic()
        end = started + seconds
        gone, probes, probe = False, 0, None
        while probe is None or time.monotonic() < end:
            probes += 1
            try:
                probe = tmux("has-session", "-t", session, check=False,
                             timeout=max(.01, end - time.monotonic()))
            except subprocess.TimeoutExpired:
                detail = "has-session probe timed out"
                break
            gone = probe.returncode == 1
            detail = f"last has-session exit {probe.returncode}; stderr: {probe.stderr.strip() or '<empty>'}"
            if gone:
                break
            time.sleep(max(0, min(.1, end - time.monotonic())))
        state = "absent" if gone else "still present" if probe and probe.returncode == 0 else "absence unconfirmed"
        observed = (f"Tmux session {session!r} {state} after {time.monotonic()-started:.3f} seconds; "
                    f"{detail}; {probes} probes.")
        self.record("wait for tmux session teardown", f"Session {session!r} absent before reuse within {seconds} seconds",
                    observed, gone, started)
        if not gone:
            raise TimeoutError(observed)
        self.ensure_serve()
        tmux("new-session", "-d", "-s", session, "-x", "120", "-y", "30", launch)
        # Socket removal precedes terminal teardown. Keep this private server
        # alive between sessions so session absence cannot race server exit.
        tmux("set-option", "-s", "exit-empty", "off")

    def wait_for_reopen(self, socket_path, hash_, seconds=10):
        started = time.monotonic()
        end = started + seconds
        restored, ready = {}, False
        observed = "No status response before the deadline."
        while time.monotonic() < end:
            try:
                # A closed engine (exit 4) is expected only in these readiness
                # probes. Every response is logged; other exits still fail.
                status_exit, status = self.cli_result("status", code=(0, 4), timeout=max(.001, end-time.monotonic()))
                status = status if isinstance(status, dict) else {}
                present = socket_path.exists()
                observed = (f"socket={present}; status exit {status_exit}, ok={status.get('ok')}, "
                            f"running={status.get('running')}, daemon={status.get('daemon')}, connection={status.get('connection')}")
                if (status_exit == 0 and status.get("ok") is True and status.get("running") is True
                        and status.get("daemon", {}).get("state") == "running"
                        and status.get("connection") == "api" and present and time.monotonic() < end):
                    show_exit, show = self.cli_result("show", hash_, code=(0, 4), timeout=max(.001, end-time.monotonic()))
                    show = show if isinstance(show, dict) else {}
                    present = socket_path.exists()
                    ready = show_exit == 0 and show.get("ok") is True and present
                    observed += (f"; show exit {show_exit}, ok={show.get('ok')}, state={show.get('state')}; "
                                 f"socket={present} after show")
                    if ready:
                        restored = show
                        break
            except subprocess.TimeoutExpired:
                observed += "; CLI probe timed out at the readiness deadline."
                break
            time.sleep(max(0, min(.1, end-time.monotonic())))
        self.record("wait for settings safely reopen engine",
                    f"Socket present, status ok/running with daemon running over API and show {hash_} exit 0 within {seconds} seconds",
                    f"{'Ready' if ready else 'Timed out'} after {time.monotonic()-started:.3f} seconds; {observed}",
                    ready, started)
        return restored

    def cli(self, *args, code=0, timeout=20):
        return self.cli_result(*args, code=code, timeout=timeout)[1]

    def cli_result(self, *args, code=0, timeout=20):
        started = time.monotonic()
        end = started + timeout
        codes = (code,) if isinstance(code, int) else tuple(code)
        expected = " or ".join(map(str, codes))
        child = self.get_case().popen([self.binary, *args, "--json"], env=self.env,
                                      stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=max(0, end-time.monotonic()))
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate()
            self.record("CLI " + shlex.join(args), f"exit {expected} with JSON within {timeout} seconds",
                        "CLI timed out and was killed.", False, started)
            raise
        try:
            value = json.loads(stdout)
        except ValueError:
            self.record("CLI " + shlex.join(args), f"exit {expected} with JSON",
                        f"Exit {child.returncode}; invalid JSON: {stdout}", False, started)
            raise
        self.record("CLI " + shlex.join(args), f"exit {expected} with JSON",
                    f"Exit {child.returncode}; stdout: {stdout.strip()}", child.returncode in codes, started)
        return child.returncode, value


def record_bulk_start(harness, code, result, hash_):
    harness.record("start all", "exit 0; list-valued hashes excludes the completed hash, which is reported skipped",
                   json.dumps(result), code == 0 and isinstance(result.get("hashes"), list) and
                   {"hash": hash_, "reason": "complete_no_seed"} in result["skipped"] and
                   hash_ not in result["hashes"])


def bulk_start_requires_hashes(case):
    """Malformed contract fixtures must fail the same Case check as the live CLI."""
    hash_ = "0000000000000000000000000000000000000001"
    valid = dict(hashes=[], skipped=[dict(hash=hash_, reason="complete_no_seed")])
    missing = {key: value for key, value in valid.items() if key != "hashes"}
    directory = case.suite.directory / "fixtures/bulk_start_contract"
    diagnostics = io.StringIO()
    with patch.object(fbtlog, "LOG_ROOT", directory), contextlib.redirect_stdout(diagnostics), \
            contextlib.redirect_stderr(diagnostics):
        with Case("bulk_start_contract", "valid") as control:
            record_bulk_start(NativeHarness(lambda: control, None, None), 0, valid, hash_)
        with Case("bulk_start_contract", "missing_hashes") as negative:
            record_bulk_start(NativeHarness(lambda: negative, None, None), 0, missing, hash_)
        with Case("bulk_start_contract", "non_list_hashes") as wrong_type:
            record_bulk_start(NativeHarness(lambda: wrong_type, None, None), 0,
                              dict(valid, hashes="other_hash"), hash_)
    diagnostic_path = directory / "diagnostics.log"
    diagnostic_path.write_text(diagnostics.getvalue())
    control_rows = [json.loads(line) for line in control.path.read_text().splitlines()]
    negative_rows = [json.loads(line) for line in negative.path.read_text().splitlines()]
    type_rows = [json.loads(line) for line in wrong_type.path.read_text().splitlines()]
    case.step("remove hashes from an otherwise valid bulk response",
              "valid response passes; missing or non-list hashes fails a step and the final Case verdict").close(
        control_rows[-1]["ok"] is True and negative_rows[0]["ok"] is False and
        negative_rows[-1]["final"] is True and negative_rows[-1]["ok"] is False and
        type_rows[0]["ok"] is False and type_rows[-1]["final"] is True and type_rows[-1]["ok"] is False,
        json.dumps(dict(control=control_rows, missing_hashes=negative_rows, non_list_hashes=type_rows)),
        artifacts=[control.path, negative.path, negative.daemon_path, wrong_type.path, diagnostic_path])


def resume_done_refused(harness, root):
    """Exercise the real CLI/socket/engine using the current shared Case."""
    name = "P0-10.complete.txt"
    content = b"Verified local P0-10 torrent.\n"
    downloads = root / "downloads"
    downloads.mkdir(exist_ok=True)
    (downloads / name).write_bytes(content)
    info = {"length": len(content), "name": name, "piece length": 16384,
            "pieces": hashlib.sha1(content).digest(), "private": 1}
    torrent = root / "complete.torrent"
    torrent.write_bytes(bencode({"info": info}))
    hash_ = hashlib.sha1(bencode(info)).hexdigest()
    harness.cli("add", str(torrent), "--quiet")

    def completed():
        value = harness.cli("show", hash_)
        return value if value.get("completion_on", 0) > 0 and value.get("state") == "stoppedUP" else None

    harness.wait_for(completed, "local bytes verify and stop on completion")
    code, envelope = harness.cli_result("start", hash_, code=1)
    expected = dict(ok=False, code=1, code_name="failure",
                    error="already complete; seeding is off for this torrent", hint=f"fbittorrent seed {hash_}")
    harness.record("start finished stop-on-complete torrent",
                   "exit 1; code 1, code_name failure, exact refusal and seed hint",
                   json.dumps(envelope), code == 1 and envelope == expected)
    code, result = harness.cli_result("start", "all")
    record_bulk_start(harness, code, result, hash_)
    code, result = harness.cli_result("delete", hash_)
    harness.record("remove disposable torrent, retaining verified bytes",
                   "exit 0; fixture removed and verified files retained", json.dumps(result),
                   code == 0 and (downloads / name).read_bytes() == content)
def prepare_peer_label_fixture(env, torrent_name):
    """Install an offline provider before the TUI enumerates its plugins."""
    provider = Path(env["XDG_DATA_HOME"]) / "fbittorrent/nova3/engines/peer_labels.py"
    provider.parent.mkdir(parents=True, exist_ok=True)
    # Planted fixtures must look like the app's own private files: the P0-14
    # store_modes sweep stats everything under the store tree with umask 000.
    for directory in (provider.parent, provider.parent.parent):
        directory.chmod(0o700)
    provider.write_text('''from novaprinter import prettyPrinter
class peer_labels:
    name = 'Local peer-label fixtures'
    url = 'https://fixture.invalid'
    supported_categories = {'all': '0'}
    def search(self, what, cat='all'):
        unknown = 'unknown' in what
        prettyPrinter(dict(link='magnet:?xt=urn:btih:' + ('2' if unknown else '1') * 40,
            name='Local YTS unknown counts' if unknown else 'Local seeded fixture',
            size=1048576, seeds=-1 if unknown else 12, leech=-1 if unknown else 3,
            engine_url=self.url))
''')
    provider.chmod(0o600)
    # The YTS JSON-to-result mapping is covered by browse's Rust unit test.
    # This Nova fixture exercises the same unknown SearchResult through the TUI.
    cache = Path(env["XDG_CACHE_HOME"]) / "fbittorrent"
    names = cache / "imdb/by-name-v2"
    names.mkdir(parents=True, exist_ok=True)
    for directory in (cache, cache / "imdb", names):
        directory.chmod(0o700)
    for name in [torrent_name, "Local YTS unknown counts", "Local seeded fixture"] + [f"Fixture episode {i}" for i in range(1, 46)]:
        safe = re.sub(r"[^a-z0-9]", "_", name.lower())[:120]
        target = names / (safe + ".json")
        target.write_text('{"missing":true}')
        target.chmod(0o600)
    if not (cache / "title.ratings.tsv").exists():
        (cache / "title.ratings.tsv").write_text("tconst\taverageRating\tnumVotes\n")
        (cache / "title.ratings.tsv").chmod(0o600)

def no_fake_seed_counts(case, root, cli, tmux, keys, capture, wait_for, session):
    """P0-11: real TUI captures; local provider fixtures, never public swarms."""
    artifacts = REPO / "target/test-logs/native"
    def check_capture(filename, ready, extra=lambda pane: True):
        step = case.step("Capture " + filename,
                         "The populated pane has no fabricated seed count.")
        try:
            last = [""]
            def poll():
                last[0] = capture()
                return ready(last[0])
            available = wait_for(poll, filename)
            pane = last[0]
            path = artifacts / ("no_fake_seed_counts." + filename)
            path.write_text(pane)
            valid = available and not re.search(r"\b0 seeds\b|\b0s\b", pane) and extra(pane)
            (step.ok if valid else step.fail)(f"Captured {path}:\n{pane}", artifacts=[path])
        except Exception as error:
            step.fail(f"Capture failed: {error}; last pane:\n{capture()}")

    if not cli("list")["torrents"]:
        cli("add", str(root / "fixture.torrent"), "--quiet")
    tmux("resize-window", "-t", session, "-x", "160", "-y", "45")
    keys("1", "Home")
    check_capture("01-torrents-160x45.txt", lambda pane: "peers connected" in pane)
    keys("2", "/", "C-a")
    tmux("send-keys", "-t", session, "-l", "known peers")
    keys("Enter")
    wait_for(lambda: "Local seeded fixture" in capture(), "local seeded search result")
    keys("Down")
    check_capture("08-search-results-selected.txt",
                  lambda pane: "12 seeds, 3 leechers" in pane)
    keys("/", "C-a")
    tmux("send-keys", "-t", session, "-l", "unknown peers")
    keys("Enter")
    wait_for(lambda: "Local YTS unknown counts" in capture(), "local unknown-count search result")
    keys("Down")

    def blank_seed_column(pane):
        header = next(line for line in pane.splitlines() if "NAME" in line and "SEEDS" in line)
        end = header.index("SEEDS") + len("SEEDS")
        row = next(line for line in pane.splitlines()
                   if "Local YTS unknown counts" in line[:end - 6])
        return not row[end - 6:end].strip() and "seeds" not in pane and "leechers" not in pane

    check_capture("10-browse-yts-latest.txt",
                  lambda pane: pane.count("Local YTS unknown counts") == 2 and "source" in pane,
                  blank_seed_column)
    keys("Escape", "4")
    check_capture("14-rss-feeds.txt", lambda pane: "Local feed" in pane)
    step = case.step("Read capabilities --json", "The native zero seed sentinel is documented as unknown.")
    capabilities = cli("capabilities")
    note = capabilities.get("field_notes", {}).get("list[].num_seeds")
    (step.ok if note == "always 0 in native mode: unknown" else step.fail)(f"Field note: {note}.")
    keys("1")
    tmux("resize-window", "-t", session, "-x", "120", "-y", "30")


def store_modes(case, root, env, cli, base, fixture):
    """Stat every saved app-data file after the real CLI/TUI lifecycle."""
    data = Path(env["XDG_DATA_HOME"])
    config = Path(env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
    parked_data = root / "parked/data"
    cache = Path(env["XDG_CACHE_HOME"]) / "fbittorrent"
    previous_mask = os.umask(0)
    try:
        stale = config.with_suffix(".toml.tmp")
        stale.write_text("unpublished setup temporary")
        stale.chmod(0o644)
        case.check(stale.stat().st_mode & 0o7777 == 0o644,
                   "The native setup fixture starts with an existing 0644 fixed temporary.")
        cli("setup", "--save", str(root / "downloads"))
        case.check(config.stat().st_mode & 0o7777 == 0o600 and
                   str(root / "downloads") in config.read_text() and
                   stale.read_text() == "unpublished setup temporary",
                   "Setup saves a private config through its fresh atomic writer, bypassing the old temporary.",
                   f"config mode={config.stat().st_mode & 0o7777:o}; old temporary is preserved; CLI umask=000.")

        # Old readable caches exercise real CLI cache hits, without public requests.
        by_name = cache / "imdb/by-name-v2/private_cache_fixture.json"
        by_id = cache / "imdb/by-id/tt1234567.json"
        ratings = cache / "title.ratings.tsv"
        for directory in [cache, cache / "imdb", by_name.parent, by_id.parent]:
            directory.mkdir(parents=True, exist_ok=True)
            directory.chmod(0o777)
        by_name.write_text(json.dumps(dict(id="tt1234567")))
        by_id.write_text(json.dumps(dict(id="tt1234567", name="Private cache fixture", year=2026,
                         kind="feature", stars="", poster_url=base + "/poster.jpg", rating=8.0,
                         votes=42, blurb="Local fixture", wiki_url="")))
        ratings.write_text("tconst\taverageRating\tnumVotes\ntt1234567\t8.0\t42\n")
        for path in [by_name, by_id, ratings]:
            path.chmod(0o644)
        result = cli("imdb", "Private cache fixture")
        poster = cache / "posters/poster_300.jpg"
        case.check(result.get("found") is True and poster.is_file() and poster.read_bytes() == b"Local poster fixture",
                   "The IMDb CLI repairs old JSON/ratings caches and downloads a poster from the local HTTP fixture.",
                   f"found={result.get('found')}; local poster requests={fixture['poster_requests']}; CLI umask=000.")
        if poster.is_file():
            poster.chmod(0o644)
            poster.parent.chmod(0o777)
        cli("imdb", "Private cache fixture")
        case.check(fixture["poster_requests"] == 1 and poster.is_file() and poster.stat().st_mode & 0o7777 == 0o600,
                   "The second IMDb CLI call repairs the existing poster without another HTTP request.",
                   f"local poster requests={fixture['poster_requests']}; poster present={poster.is_file()}.")
    finally:
        os.umask(previous_mask)

    files = sorted(path for directory in [data, parked_data] for path in directory.rglob("*") if path.is_file())
    store = data / "fbittorrent"
    expected = {store / name for name in ["shows.json", "rss.json", "search.json", "engine/torrents.json"]}
    expected.add(parked_data / "fbittorrent/engine/torrents.json")
    case.check(expected.issubset(files) and any(path.suffix == ".torrent" and path.parent == store / "engine" for path in files),
               "Shows, RSS, search preferences, ledger and torrent metadata were actually saved.",
               "Saved data paths: " + ", ".join(str(path.relative_to(data.parent)) for path in files))
    caches = [cache, root / "parked/cache/fbittorrent"]
    paths = files + [config]
    for directory in caches:
        if directory.exists():
            paths += [directory, *sorted(directory.rglob("*"))]
    for directory in [Path(env["XDG_RUNTIME_DIR"]) / "fbittorrent", root / "parked/runtime/fbittorrent"]:
        paths += [directory, directory / "engine.lock"]
    for path in paths:
        mode = "700" if path.is_dir() else "600"
        step = case.step("stat -c %a " + str(path), f"The saved {'directory' if path.is_dir() else 'file'} has mode {mode}.")
        result = subprocess.run(["stat", "-c", "%a", str(path)], capture_output=True, text=True)
        (step.ok if result.returncode == 0 and result.stdout.strip() == mode else step.fail)(
            f"{path}: stat exit {result.returncode}, mode {result.stdout.strip()!r}; {result.stderr.strip()}")


def expected_shutdown_stderr(stderr):
    return stderr == "" or re.fullmatch(r"INFO engine drained in [0-9]+ ms\n?", stderr) is not None


def expected_daemon_stderr(stderr, complete=True):
    """Only the daemon's named success phases and legacy dispatch tracing are allowed."""
    patterns = [r'INFO daemon listening \{.*\} no_notify=(?:false|true)',
                r'INFO daemon ready state=starting', r'INFO engine starting session',
                r'INFO engine session exists restores=[0-9]+',
                r'DEBUG engine initial validation admitted source=engine hash=[0-9a-f]{40}',
                r'INFO engine state changed state=(?:starting|running|parked|stopping) engine_epoch=[0-9]+',
                r'DEBUG api legacy dispatch path="[A-Za-z0-9/]+"',
                r'INFO daemon restarting engine reason=(?:Explicit|Settings)', r'INFO network Waiting for fbmissing0',
                r'INFO resume trusted hash=[0-9a-f]{40}', r'INFO resume check_required hash=[0-9a-f]{40} clean=(?:true|false)',
                r'INFO imdb index loaded refresh=(?:true|false)',
                r'INFO daemon draining', r'INFO engine drained in [0-9]+ ms',
                r'INFO daemon slot released elapsed_ms=[0-9]+', r'INFO daemon sockets unlinked',
                r'INFO daemon stopped reason=(?:Quit|Signal\(15\)) code=0']
    lines = stderr.splitlines()
    known = all(any(re.fullmatch(pattern, line) for pattern in patterns) for line in lines)
    if not complete:
        return known
    phases = [r'INFO daemon draining', r'INFO engine drained in [0-9]+ ms',
              r'INFO daemon slot released elapsed_ms=[0-9]+', r'INFO daemon sockets unlinked',
              r'INFO daemon stopped reason=(?:Quit|Signal\(15\)) code=0']
    indices = [next((i for i, line in enumerate(lines) if re.fullmatch(pattern, line)), -1) for pattern in phases]
    return known and all(i >= 0 for i in indices) and indices == sorted(indices)


def startup_survives_bad_record(binary):
    """Open the real TUI with a corrupt cache and a local unresolved magnet."""
    with Case("native", "startup_survives_bad_record") as case, \
            tempfile.TemporaryDirectory(prefix="fbittorrent-bad-record-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOME=str(root / "home"), TERM="xterm-256color")
        for key, directory in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                               ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CACHE_HOME", "cache"),
                               ("XDG_STATE_HOME", "state")]:
            path = root / directory
            path.mkdir(mode=0o700)
            env[key] = str(path)
        harness = NativeHarness(lambda: case, binary, env)
        downloads = root / "downloads"
        downloads.mkdir()
        store = root / "data/fbittorrent/engine"
        store.mkdir(parents=True)
        tracker_requests = []

        class Tracker(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                tracker_requests.append(self.path)
                body = bencode(dict(interval=3600, peers=b""))
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        tracker = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Tracker)
        threading.Thread(target=tracker.serve_forever, daemon=True).start()

        def cached(name):
            content = b"P0-09 verified local bytes.\n"
            (downloads / name).write_bytes(content)
            info = dict(name=name, length=len(content), private=1,
                        **{"piece length": 16384, "pieces": hashlib.sha1(content).digest()})
            hash_ = hashlib.sha1(bencode(info)).hexdigest()
            return hash_, bencode(dict(info=info))

        valid, valid_bytes = cached("P0-09.valid.txt")
        bad, repaired_bytes = cached("P0-09.corrupt.txt")
        magnet = "a" * 40
        records = [dict(hash=hash_, name=name, source=source, output_folder=str(downloads),
                        only_files=None, upload_limit=32768, ratio_limit=0.0, category="", tags="",
                        added_on=100-index, completion_on=0, paused=False, resume_after_migration=False)
                   for index, (hash_, name, source) in enumerate([
                       (magnet, "P0-09.magnet", f"magnet:?xt=urn:btih:{magnet}&tr=http%3A%2F%2F127.0.0.1%3A{tracker.server_port}%2Fannounce"),
                       (bad, "P0-09.corrupt.txt", f"magnet:?xt=urn:btih:{bad}&tr=http%3A%2F%2F127.0.0.1%3A{tracker.server_port}%2Fannounce"),
                       (valid, "P0-09.valid.txt", "")])]
        ledger = store / "torrents.json"
        ledger.write_text(json.dumps(dict(migrated_legacy=True, records=records)))
        (store / f"{valid}.torrent").write_bytes(valid_bytes)
        bad_cache = store / f"{bad}.torrent"
        bad_cache.write_bytes(b"corrupt cache")
        config = root / "config/fbittorrent/config.toml"
        config.parent.mkdir()
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        # Bind all engine networking to loopback; the tracker never supplies peers.
        config.write_text(f'engine = "native"\nautostart = false\nnotify = false\n'
                          f'peer_port = {port}\nnetwork_interface = "lo"\n'
                          f'save_path = {json.dumps(str(downloads))}\n')
        tmux_socket, session = f"fbt-fbt-p0-09-kzw-{os.getpid()}", "fbt-fbt-p0-09-kzw"

        def tmux(*args, check=True, timeout=None):
            return subprocess.run(["tmux", "-L", tmux_socket, *args], env=env,
                                  capture_output=True, text=True, check=check, timeout=timeout)

        def rows():
            return {row["hash"]: row for row in harness.cli("list")["torrents"]}

        socket_path = root / "runtime/fbittorrent/engine.sock"
        try:
            app_launch = shlex.join([binary, "tui"]) + " 2>>" + shlex.quote(str(case.suite.stderr_path))
            launch = f"exec script -qef -c {shlex.quote(app_launch)} {shlex.quote(str(root / 'terminal.typescript'))}"
            began = time.monotonic()
            harness.start_tmux(tmux, session, launch)
            harness.wait_for(lambda: socket_path.exists() and "P0-09" in
                             tmux("capture-pane", "-t", session, "-p", check=False).stdout,
                             "startup socket and first TUI frame open", seconds=9)
            opened = time.monotonic() - began
            case.check(opened < 10 and socket_path.exists(), "App opens in under 10 seconds with an unresolved saved magnet.",
                       f"Socket {'present' if socket_path.exists() else 'absent'} after {opened:.3f} seconds.")
            initial = rows()
            case.check(set(initial) == {valid, bad, magnet}, "All three ledger records are listed.", json.dumps(initial))
            case.check(initial[magnet]["state"] == "metaDL" and "error_message" not in initial[magnet],
                       "Unresolved magnet is metadata in progress without a premature error.", json.dumps(initial[magnet]))
            harness.wait_for(lambda: rows()[bad]["state"] == "error", "corrupt cache fails independently")
            failed = harness.cli("show", bad)
            case.check(bool(failed["error_message"]) and "could not restore" in failed["error_message"],
                       "show --json preserves the corrupt-cache restore failure message.", json.dumps(failed))
            harness.wait_for(lambda: rows()[valid]["completion_on"] > 0, "valid cache verifies despite unresolved magnet")
            # The details column appears at 140+ columns; inspect its real UI.
            tmux("resize-window", "-t", session, "-x", "160", "-y", "40")
            tmux("send-keys", "-t", session, "Home", "Down")
            harness.wait_for(lambda: "could not restore" in tmux("capture-pane", "-t", session, "-p").stdout,
                             "TUI details display the selected restore failure")
            pane = tmux("capture-pane", "-t", session, "-p").stdout
            case.check("could not restore" in pane, "The wide TUI details show the restore error in text.", pane)
            original_error = failed["error_message"]
            bad_cache.unlink()
            harness.cli("start", bad)
            pending_retry = harness.cli("show", bad)
            case.check(pending_retry["state"] == "metaDL" and pending_retry["error_message"] == original_error,
                       "An unresolved retry retains the previous corruption diagnostic.", json.dumps(pending_retry))
            harness.cli("stop", bad)
            bad_cache.write_bytes(valid_bytes)  # Valid metainfo for the wrong saved hash.
            harness.cli("start", bad)
            harness.wait_for(lambda: rows()[bad]["state"] == "error", "retry with mismatched cache fails again")
            failed_again = harness.cli("show", bad)
            case.check("does not match saved record" in failed_again["error_message"],
                       "A retry that fails again replaces the diagnostic with its new failure.", json.dumps(failed_again))
            bad_cache.write_bytes(repaired_bytes)
            harness.cli("start", bad)
            harness.wait_for(lambda: rows()[bad]["completion_on"] > 0 and
                             "error_message" not in rows()[bad], "start retries the repaired cache successfully")
            # Keep the production 90-second bound: prove timeout becomes error,
            # rather than declaring an unresolved magnet failed at startup.
            seen = []

            def magnet_failed():
                row = harness.cli("show", magnet)
                seen.append((row["state"], row.get("error_message")))
                if row["state"] != "error":
                    time.sleep(.9)
                return row if row["state"] == "error" else None

            failure = harness.wait_for(magnet_failed, "unresolved local magnet reaches its real 90-second timeout", seconds=100)
            case.check(bool(failure) and "timed out" in failure["error_message"] and
                       all(state == "metaDL" and message is None for state, message in seen[:-1]),
                       "Magnet stays metadata until the real timeout, then shows error_message.", json.dumps(seen))
            case.check(bool(tracker_requests), "The real engine queried the local tracker fixture.",
                       f"Local tracker received {len(tracker_requests)} requests; engine is bound to lo.")
            harness.cli("start", magnet)
            retry = harness.cli("show", magnet)
            case.check(retry["state"] == "metaDL" and retry["error_message"] == failure["error_message"],
                       "Start retries a timed-out magnet while retaining the last failure until success.", json.dumps(retry))
            harness.cli("stop", magnet)
            case.check(harness.cli("show", magnet)["state"] == "stoppedDL", "Stop cancels metadata resolution.")
            harness.cli("start", magnet)
            harness.cli("delete", magnet)
            case.check(magnet not in rows(), "Delete removes an unresolved record without needing a handle.")
            harness.cli("down")
            harness.wait_for(lambda: not socket_path.exists(), "bad-record app closes", seconds=6)
            tmux("send-keys", "-t", session, "-l", "q")
            harness.wait_for(lambda: tmux("has-session", "-t", session, check=False).returncode == 1,
                             "bad-record app process exits after session cleanup", seconds=6)
            stderr = case.suite.stderr_path.read_bytes()[case.stderr_offset:].decode("utf-8", errors="replace")
            case.check(expected_daemon_stderr(stderr),
                       "Daemon startup, legacy dispatch and ordered drain phases are the only stderr lines accepted.",
                       stderr or "No stderr was emitted.")
            case.check(not expected_daemon_stderr(stderr + "WARNING unexpected\n") and
                       not expected_daemon_stderr(stderr.replace("INFO daemon sockets unlinked", "")) and
                       not expected_daemon_stderr(stderr.replace("INFO daemon draining", "INFO daemon FAILED")),
                       "Daemon stderr controls reject unknown lines and incomplete or replaced teardown phases.")
            stderr_examples = [("", True), ("INFO engine drained in 0 ms\n", True),
                               ("INFO engine drained in 123 ms", True), ("\n", False),
                               (" INFO engine drained in 1 ms\n", False),
                               ("INFO engine drained in -1 ms\n", False),
                               ("INFO engine drained in 1.5 ms\n", False),
                               ("INFO engine drained in 1 ms\nWARNING unexpected\n", False),
                               ("INFO engine drained in 1 ms\n\n", False)]
            case.check(all(expected_shutdown_stderr(text) == expected for text, expected in stderr_examples),
                       "The stderr predicate accepts the P0-08 success format and rejects malformed or additional lines.",
                       json.dumps([dict(stderr=text, expected=expected, observed=expected_shutdown_stderr(text))
                                   for text, expected in stderr_examples]))
        finally:
            tmux("kill-server", check=False)
            harness.close_serve()
            tracker.shutdown()


def stop_during_hashing(binary):
    """Observe saved-paused verification, then Stop and Resume a real check."""
    with Case("native", "stop_during_hashing") as case, \
            tempfile.TemporaryDirectory(prefix="fbittorrent-hashing-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOME=str(root / "home"), TERM="xterm-256color")
        for key, directory in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                               ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CACHE_HOME", "cache"),
                               ("XDG_STATE_HOME", "state")]:
            path = root / directory
            path.mkdir(mode=0o700)
            env[key] = str(path)
        harness = NativeHarness(lambda: case, binary, env)
        downloads = root / "downloads"
        downloads.mkdir()
        name, length, piece_length = "P0-09.hashing.bin", 1024**3, 4*1024**2
        with (downloads / name).open("wb") as payload:
            payload.truncate(length)
        piece_hash = hashlib.sha1(bytes(piece_length)).digest()
        # The last hash intentionally differs, so finishing the recheck still
        # leaves a download, with no P0-10 completed-torrent refusal involved.
        pieces = piece_hash * (length // piece_length - 1) + bytes(20)
        info = dict(name=name, length=length, private=1, **{"piece length": piece_length, "pieces": pieces})
        hash_ = hashlib.sha1(bencode(info)).hexdigest()
        store = root / "data/fbittorrent/engine"
        store.mkdir(parents=True)
        record = dict(hash=hash_, name=name, source="", output_folder=str(downloads),
                      only_files=None, upload_limit=32768, ratio_limit=0.0, category="", tags="",
                      added_on=100, completion_on=0, paused=True, resume_after_migration=False)
        ledger = store / "torrents.json"
        ledger.write_text(json.dumps(dict(migrated_legacy=True, records=[record])))
        (store / f"{hash_}.torrent").write_bytes(bencode(dict(info=info)))
        config = root / "config/fbittorrent/config.toml"
        config.parent.mkdir()
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        config.write_text(f'engine = "native"\nautostart = false\nnotify = false\n'
                          f'peer_port = {port}\nnetwork_interface = "lo"\n'
                          f'save_path = {json.dumps(str(downloads))}\n')
        tmux_socket, session = f"fbt-fbt-p0-09-kzw-hash-{os.getpid()}", "fbt-fbt-p0-09-kzw"

        def tmux(*args, check=True, timeout=None):
            return subprocess.run(["tmux", "-L", tmux_socket, *args], env=env,
                                  capture_output=True, text=True, check=check, timeout=timeout)

        def capture():
            return tmux("capture-pane", "-t", session, "-p").stdout

        socket_path = root / "runtime/fbittorrent/engine.sock"
        stderr_offset = case.stderr_offset
        try:
            command = shlex.join([binary, "tui"]) + " 2>>" + shlex.quote(str(case.suite.stderr_path))
            launch = f"exec script -qef -c {shlex.quote(command)} {shlex.quote(str(root / 'terminal.typescript'))}"
            harness.start_tmux(tmux, session, launch)
            harness.wait_for(lambda: socket_path.exists(), "hashing fixture opens its isolated socket")

            def active_hashing():
                row = harness.cli("show", hash_)
                return row if row["state"] == "checkingDL" and 0 < row["completed"] < length else None

            initial = harness.wait_for(active_hashing, "saved-paused startup verification is actively hashing")
            case.check(bool(initial) and json.loads(ledger.read_text())["records"][0]["paused"] is True,
                       "A saved-paused record lists checkingDL while real verification is active.", json.dumps(initial))

            def finished_paused():
                row = harness.cli("show", hash_)
                return row if row["state"] == "stoppedDL" else None

            finished = harness.wait_for(finished_paused, "saved-paused verification completes before listing stoppedDL", seconds=15)
            case.check(bool(finished) and finished["completed"] == length-piece_length and
                       json.loads(ledger.read_text())["records"][0]["paused"] is True,
                       "The stopped state follows completed verification and preserves saved paused intent.", json.dumps(finished))
            harness.cli("down")
            harness.wait_for(lambda: not socket_path.exists(), "saved-paused verification app closes", seconds=6)
            tmux("send-keys", "-t", session, "-l", "q")
            harness.wait_for(lambda: tmux("has-session", "-t", session, check=False).returncode == 1,
                             "saved-paused verification app drains and exits", seconds=6)
            stderr_bytes = case.suite.stderr_path.read_bytes()
            first_stderr = stderr_bytes[stderr_offset:].decode("utf-8", errors="replace")
            case.check(expected_daemon_stderr(first_stderr),
                       "The verification daemon emits only known success phases and legacy dispatch traces, with an ordered drain.",
                       first_stderr or "No stderr was emitted.")
            stderr_offset = len(stderr_bytes)
            # Restart with the same saved-paused intent, then explicitly Stop
            # its fresh startup check before exercising the actual Resume key.
            # A clean stop lets fast resume (P6-06) trust the saved state and
            # skip the check, so consume the CLEAN marker as a crash would.
            marker = store / "resume/CLEAN"
            case.check(marker.is_file(), "A clean down leaves the fast-resume CLEAN marker.", str(marker))
            marker.unlink(missing_ok=True)
            harness.start_tmux(tmux, session, launch)
            harness.wait_for(lambda: socket_path.exists(), "saved-paused hashing fixture reopens")
            hashing = harness.wait_for(active_hashing, "real file hashing is active before Stop")
            case.check(bool(hashing), "Stop is issued while the real 1 GiB payload is being hashed.", json.dumps(hashing))
            harness.cli("stop", hash_)
            stopped = harness.cli("show", hash_)
            case.check(stopped["state"] == "stoppedDL" and json.loads(ledger.read_text())["records"][0]["paused"] is True,
                       "Stop during hashing lists a paused row and durably saves paused intent.", json.dumps(stopped))
            harness.wait_for(lambda: "(p) Resume" in capture(), "the paused hash-check row offers Resume in the TUI")
            pane = capture()
            case.check("(p) Resume" in pane, "The actual TUI footer offers Resume.", pane)
            tmux("send-keys", "-t", session, "-l", "p")
            harness.wait_for(lambda: json.loads(ledger.read_text())["records"][0]["paused"] is False,
                             "TUI Resume starts the saved torrent")

            def resumed():
                row = harness.cli("show", hash_)
                return row if row["state"] == "stalledDL" and 0 < row["completed"] < length else None

            restored = harness.wait_for(resumed, "resumed hash check finishes and admits the incomplete download", seconds=15)
            case.check(bool(restored), "Resume completes the real recheck and admits the torrent.", json.dumps(restored))
            harness.cli("down")
            harness.wait_for(lambda: not socket_path.exists(), "hashing fixture closes", seconds=6)
            tmux("send-keys", "-t", session, "-l", "q")
            harness.wait_for(lambda: tmux("has-session", "-t", session, check=False).returncode == 1,
                             "hashing fixture process drains and exits", seconds=6)
            stderr = case.suite.stderr_path.read_bytes()[stderr_offset:].decode("utf-8", errors="replace")
            case.check(expected_daemon_stderr(stderr),
                       "The hashing daemon emits only known success phases and legacy dispatch traces, with an ordered drain.",
                       stderr or "No stderr was emitted.")
        finally:
            tmux("kill-server", check=False)
            harness.close_serve()


# Presence and quit cases use the real static-unit startup path, scoped to a
# unique runtime-only test unit. No persistent owner unit is started or changed.
ATTACH_CASES = ("tui_starts_daemon", "q_detaches", "q_confirms", "first_q_notice_once",
                "daemon_gone_banner", "second_tui_attaches")


class AttachedTui:
    def __init__(self, case, binary, sandbox, harness, session="fbt-fbt-p1-15-h0l"):
        self.case, self.binary, self.sandbox, self.h = case, binary, sandbox, harness
        self.socket = session + "-" + str(os.getpid()) + "-" + sandbox.root.name
        self.session, self.counter = session, 0
        created = self.tmux("new-session", "-d", "-s", self.session, "-x", "120", "-y", "30", "sleep 600")
        case.check(created.returncode == 0, "case owns a fresh private tmux server",
                   f"socket={self.socket}; exit={created.returncode}; stderr={created.stderr}")

    def tmux(self, *args):
        return subprocess.run(["tmux", "-L", self.socket, "-f", "/dev/null", *args],
                              env=self.sandbox.env, capture_output=True, text=True, timeout=5)

    def start(self):
        before = self.clients() if daemon.DaemonContract.socket_path(self.sandbox).exists() else 0
        self.counter += 1
        done = self.sandbox.root / f"ui-{self.counter}.exit"
        stderr = self.case.path.with_suffix(f".tui-{self.counter}.stderr")
        command = (shlex.join([self.binary, "tui"]) + " 2>>" + shlex.quote(str(stderr)) +
                   "; code=$?; printf '%s' \"$code\" >" + shlex.quote(str(done)) + "; sleep 600")
        result = self.tmux("new-window", "-d", "-P", "-F", "#{window_id}", "-t", self.session, command)
        target = result.stdout.strip()
        self.case.check(result.returncode == 0, "a new TUI starts in this terminal", f"target={target}; {result.stderr}")
        self.h.wait_for(lambda: ("Torrents" in self.capture(target), self.capture(target)), "attached TUI draws its first frame")
        self.h.wait_for(lambda: ((count := self.clients()) == before + 1, f"tui clients={count}; before={before}"),
                        "new terminal completes its interactive API hello")
        self.status()
        return target, done

    def capture(self, target):
        result = self.tmux("capture-pane", "-p", "-t", target)
        return result.stdout

    def assert_pane(self, target, text, timeout=5):
        return self.h.wait_for(lambda: (text in self.capture(target), self.capture(target)),
                               "pane contains " + text, timeout=timeout)

    def keys(self, target, key, literal=True):
        step = self.case.step("tmux send-keys " + ("-l " if literal else "") + repr(key), "key reaches the selected TUI")
        result = self.tmux("send-keys", "-t", target, *(["-l", key] if literal else [key]))
        step.close(result.returncode == 0, f"exit={result.returncode}; pane={self.capture(target)!r}")
        self.status()

    def status(self):
        return self.h.cli("status")

    def clients(self):
        with daemon.ApiConn(self.h) as connection:
            connection.hello()
            return connection.call("app.stats")["result"]["active_clients"]["tui"]

    def close(self):
        self.tmux("kill-server")


@contextlib.contextmanager
def local_download(case, binary, leech):
    """Actual BitTorrent payload from a capped loopback seed, not simulated progress."""
    import base64
    import struct
    import tomllib
    with daemon.Sandbox() as seedbox, daemon.Harness(case, binary, seedbox) as seedh:
        config = tomllib.loads((Path(seedbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml").read_text())
        peer = socket.inet_aton("127.0.0.1") + struct.pack(">H", config["peer_port"])
        body = fixtures.bencode(dict(interval=1, peers=peer))
        with fixtures.HttpFixture(seedbox.root, {"/announce": dict(base64=base64.b64encode(body).decode())}) as tracker:
            payload = seedbox.downloads / "daemon-fixture.bin"
            payload.write_bytes(os.urandom(8 * 1024 * 1024))
            torrent = fixtures.torrent_fixture(seedbox.root, [payload], tracker=tracker.base + "/announce")
            process = seedh.start_serve()
            if not (seedh.receive_ready() and seedh.wait_engine()):
                raise RuntimeError("loopback seed failed to start")
            seedh.cli("add", str(torrent.torrent), "--seed", "--quiet")
            seedh.wait_for(lambda: ((row := json.loads(seedh.engine_call("torrents/info")["value"])[0])["state"] == "uploading", str(row)), "verified local seed")
            reply = seedh.engine_call("torrents/setUploadLimit", "POST", [("hashes", torrent.info_hash), ("limit", "32768")])
            case.check(reply.get("value") == "Ok.", "seed upload is capped so the download remains active", str(reply))
            leech.cli("add", str(torrent.torrent), "--quiet")
            leech.wait_for(lambda: ((row := leech.cli("show", torrent.info_hash))["completed"] >= 32768 and
                                   row["progress"] < 1, str(row)), "real local payload begins", timeout=20)
            try:
                yield torrent.info_hash
            finally:
                seedh.stop_serve(process)


def attachment_case(name, binary):
    with Case("native", name) as case, daemon.Sandbox() as sandbox, \
            daemon.Harness(case, binary, sandbox) as h, daemon.InstalledUnit(h) as unit:
        ui = AttachedTui(case, binary, sandbox, h)
        try:
            case.check(not h.engine_path.exists() and unit.pid() == 0,
                       "fixture has no daemon before the TUI starts it", str(h.engine_path))
            target, done = ui.start()
            pid = unit.pid()
            case.check(pid > 1 and h.engine_path.exists(), "starting TUI starts its isolated daemon", f"pid={pid}")
            ui.assert_pane(target, "Torrents")
            h.wait_for(lambda: ((count := ui.clients()) == 1, f"tui clients={count}"), "one presence connection")
            # The worker must keep the presence alive beyond its five-second heartbeat.
            if name == "tui_starts_daemon":
                time.sleep(5.3)
                case.check(ui.clients() == 1 and unit.pid() == pid, "presence heartbeat retains the interactive client", f"pid={pid}")
                ui.keys(target, "C-c", literal=False)
                h.wait_for(lambda: (done.exists(), "UI exit marker=" + str(done.exists())), "Ctrl+C detaches")
                case.check(unit.pid() == pid, "Ctrl+C leaves the daemon running", json.dumps(ui.status()))
            elif name in ("q_detaches", "q_confirms"):
                with local_download(case, binary, h) as hash_:
                    ui.assert_pane(target, "daemon-fixture.bin")
                    if name == "q_detaches":
                        before = h.cli("show", hash_)["completed"]
                        ui.keys(target, "q")
                        h.wait_for(lambda: (done.exists(), ui.capture(target)), "q closes only the TUI")
                        h.wait_for(lambda: ((row := h.cli("show", hash_))["completed"] > before and row["progress"] < 1,
                                           str(row)), "download gains payload after q", timeout=8)
                        case.check(unit.pid() == pid, "same daemon continues the real local download", json.dumps(ui.status()))
                    else:
                        # Each cancellation is a control against accidental shutdown.
                        for key, literal in [("Y", True), ("n", True), ("Enter", False)]:
                            ui.keys(target, "Q")
                            ui.assert_pane(target, "Stop fbittorrent? 1 downloads pause until it starts again. y/N")
                            ui.keys(target, key, literal)
                            case.check(unit.pid() == pid and not done.exists(), "only lowercase y can confirm shutdown", json.dumps(ui.status()))
                        ui.keys(target, "Q")
                        ui.assert_pane(target, "Stop fbittorrent? 1 downloads pause until it starts again. y/N")
                        ui.keys(target, "y")
                        h.wait_for(lambda: (not h.engine_path.exists() and unit.pid() == 0, str(ui.status())), "Q/y stops daemon", timeout=6)
                        h.wait_for(lambda: (done.exists(), ui.capture(target)), "confirmed Q closes this TUI")
            elif name == "first_q_notice_once":
                notice = "Downloads continue in the background; fbittorrent stops itself when idle."
                for ordinal in (1, 2):
                    ui.keys(target, "q")
                    h.wait_for(lambda: (done.exists(), ui.capture(target)), "q detaches without acknowledgement")
                    pane = ui.capture(target)
                    case.check((notice in pane) == (ordinal == 1), "only first q prints the background notice", pane)
                    saved = Path(sandbox.env["XDG_STATE_HOME"]) / "fbittorrent/notices.json"
                    case.check(json.loads(saved.read_text())["first_quit_seen"] is True, "first quit preference is saved", saved.read_text())
                    if ordinal == 1:
                        target, done = ui.start()
            elif name == "daemon_gone_banner":
                # Cancel a manager restart before delivering an actual SIGKILL;
                # the TUI must stay in place and Enter must start a new owner.
                drop = unit.drop / "no-restart.conf"
                drop.write_text("[Service]\nRestart=no\n")
                unit.control("daemon-reload")
                began = time.monotonic()
                os.kill(pid, signal.SIGKILL)
                h.wait_for(lambda: ("fbittorrent stopped" in ui.capture(target) and "Enter) Start" in ui.capture(target), ui.capture(target)),
                           "stopped banner and enabled Enter action", timeout=5)
                case.check(time.monotonic() - began < 5 and not done.exists(), "daemon loss is visible within five seconds without closing the client", ui.capture(target))
                ui.status()
                ui.keys(target, "Enter", literal=False)
                h.wait_for(lambda: (unit.pid() > 1 and unit.pid() != pid and h.engine_path.exists(), str(ui.status())), "Enter starts a new daemon")
                h.wait_for(lambda: ("fbittorrent stopped" not in ui.capture(target), ui.capture(target)), "reconnection clears stopped banner")
                ui.assert_pane(target, "Torrents")
            elif name == "second_tui_attaches":
                with fixtures.magnet_fixture(sandbox.root) as torrent:
                    h.cli("add", str(torrent.torrent), "--quiet")
                    ui.assert_pane(target, "daemon-fixture.bin")
                    other, _ = ui.start()
                    ui.assert_pane(other, "daemon-fixture.bin")
                    h.wait_for(lambda: ((count := ui.clients()) == 2, f"tui clients={count}"), "two independent interactive clients")
                    case.check(unit.pid() == pid and len(h.cli("list")["torrents"]) == 1,
                               "second terminal attaches to the same daemon and torrent", ui.capture(target) + "\nSECOND:\n" + ui.capture(other))
                    launcher = sandbox.root / "shims/omarchy-launch-or-focus-tui.log"
                    case.check(not launcher.exists(), "terminal attachment never invokes the desktop focus helper", str(launcher))
                    ui.keys(other, "q")
                    h.wait_for(lambda: ((count := ui.clients()) == 1, f"tui clients={count}"), "detaching second TUI retains first")
        finally:
            ui.close()



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default=str(REPO / "target/debug/fbittorrent"))
    parser.add_argument("--list", action="store_true", help="list case slugs without launching anything")
    parser.add_argument("--case", choices=CASES + ATTACH_CASES,
                        help="log one case, replaying its prerequisites in the isolated fixture")
    options = parser.parse_args()
    if options.list:
        print("\n".join(CASES + ATTACH_CASES))
        return 0
    binary = str(Path(options.binary).resolve())
    if options.case in ATTACH_CASES:
        attachment_case(options.case, binary)
        return suite_status("native")
    if options.case is None:
        for name in ATTACH_CASES:
            attachment_case(name, binary)
    if options.case in (None, "stop_during_hashing"):
        stop_during_hashing(binary)
        if options.case == "stop_during_hashing":
            return suite_status("native")
    if options.case in (None, "startup_survives_bad_record"):
        startup_survives_bad_record(binary)
        if options.case == "startup_survives_bad_record":
            return suite_status("native")
    if options.case == "launcher_never_called":
        launcher_never_called(binary)
    if options.case == "terminal_close_is_graceful":
        terminal_close_is_graceful(binary)
        return suite_status("native")
    with tempfile.TemporaryDirectory(prefix="fbittorrent-native-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                   XDG_DATA_HOME=str(root / "data"), XDG_RUNTIME_DIR=str(root / "runtime"),
                   XDG_CACHE_HOME=str(root / "cache"),
                   XDG_STATE_HOME=str(root / "state"), TERM="xterm-256color")
        env.pop("FBITTORRENT_NO_START", None)
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
            Path(env[key]).mkdir(mode=0o700)
        config_dir = Path(env["XDG_CONFIG_HOME"]) / "fbittorrent"
        config_dir.mkdir()
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            peer_port = probe.getsockname()[1]
        (config_dir / "config.toml").write_text(
            f'engine = "native"\nautostart = false\nnotify = false\nnetwork_interface = "lo"\n'
            f'backend_unit = "fbittorrent-fixture-never.service"\n'
            f'url = "http://127.0.0.1:9"\npeer_port = {peer_port}\n'
            f'save_path = {json.dumps(str(root / "downloads"))}\n')

        harness = NativeHarness(lambda: case, binary, env)
        cli, record, wait_for = harness.cli, harness.record, harness.wait_for

        name = "Fixture.with.a.long.filename." + "readable." * 14 + "END.txt"
        prepare_imdb_fixture(env, [name] + [f"Fixture episode {i}" for i in range(1, 46)])
        if options.case in (None, "no_fake_seed_counts"):
            prepare_peer_label_fixture(env, name)
        content = b"Local torrent lifecycle fixture.\n"
        info = {"length": len(content), "name": name, "piece length": 16384,
                "pieces": hashlib.sha1(content).digest()}
        torrent = bencode({"info": info})
        (root / "fixture.torrent").write_bytes(torrent)
        expected_hash = hashlib.sha1(bencode(info)).hexdigest()
        fixture = {"version": 1, "feed_requests": 0, "torrent_requests": 0, "poster_requests": 0,
                   "phase": "setup", "torrent_request_phases": []}

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                if self.path == "/fixture.torrent":
                    fixture["torrent_requests"] += 1
                    fixture["torrent_request_phases"].append(fixture["phase"])
                    body = torrent
                    kind = "application/x-bittorrent"
                elif self.path == "/poster.jpg":
                    fixture["poster_requests"] += 1
                    body = b"Local poster fixture"
                    kind = "image/jpeg"
                elif self.path == "/rss":
                    fixture["feed_requests"] += 1
                    version = fixture["version"]
                    items = "".join(
                        f'<item><guid>episode-{i}</guid><title>Fixture episode {i}</title>'
                        f'<pubDate>{(datetime(2026, 10, 1, 12, tzinfo=timezone.utc) + timedelta(days=i-1)).strftime("%d %b %Y %H:%M:%S %z")}</pubDate>'
                        f'<enclosure type="application/x-bittorrent" url="{base}/fixture.torrent"/>'
                        f'</item>' for i in range(version, 0, -1))
                    body = f'<rss version="2.0"><channel><title>Local feed</title>{items}</channel></rss>'.encode()
                    kind = "application/rss+xml"
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        base = f"http://127.0.0.1:{server.server_port}"
        threading.Thread(target=server.serve_forever, daemon=True).start()
        shows_path = Path(env["XDG_DATA_HOME"]) / "fbittorrent/shows.json"
        shows_path.parent.mkdir(exist_ok=True)

        def saved_release(key, released):
            return dict(key=key, season=1, episode=0 if key == "S01" else int(key[-2:]),
                        title=f"Fixture.{key}.1080p", magnet=base + "/fixture.torrent",
                        seeds=10, size=len(content), released=released, quality="1080p", candidates=1)

        checked = int(time.time())
        shows_path.write_text(json.dumps({"shows": [
            dict(imdb="tt1234001", title="Alpha Show", last_checked=checked,
                 catalogue_complete=True, seen=["S01E01"],
                 episodes=[saved_release("S01E01", 10), saved_release("S01", 20)]),
            dict(imdb="tt1234002", title="Beta Show", last_checked=checked,
                 catalogue_complete=True, episodes=[saved_release("S01E02", 40), saved_release("S01E01", 30)]),
        ]}))

        def show_episodes(imdb):
            return {episode["key"]: episode for episode in cli("shows", "episodes", imdb)["episodes"]}

        session = "fbt-fbt-p4-02-6bx" if options.case == "file_modal_shows_help_80x24" else "fbt-fbt-p0-14-rsx"
        tmux_socket = f"{session}-{os.getpid()}"

        def tmux(*args, check=True, timeout=None):
            return subprocess.run(["tmux", "-L", tmux_socket, *args], env=env,
                                  capture_output=True, text=True, check=check, timeout=timeout)

        def capture(log=True):
            result = tmux("capture-pane", "-t", session, "-p", check=False)
            if log:
                record("capture terminal pane", "pane is available for the assertion",
                       result.stdout, result.returncode == 0)
            return result.stdout

        def keys(*args):
            for key in args:
                result = tmux("send-keys", "-t", session, *(["-l", key] if len(key) == 1 else [key]))
                record("tmux send-keys " + repr(key), "key delivered to isolated TUI", f"exit={result.returncode}", result.returncode == 0)

        def mouse_text(text, button):
            rows = capture(log=False).splitlines()
            y, row = next((y, row) for y, row in enumerate(rows) if text in row)
            x = row.index(text) + len(text) // 2
            sequence = f"\x1b[<{button};{x+1};{y+1}M"
            if button == 0:
                sequence += f"\x1b[<0;{x+1};{y+1}m"
            tmux("send-keys", "-t", session, "-l", sequence)

        def click_text(text):
            mouse_text(text, 0)

        socket_path = Path(env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
        error_file = None  # suite capture is assigned before the first launch
        terminal_bytes = root / "terminal.typescript"
        try:
            with Case("native", "dec_mouse_mode", enabled=options.case in (None, "dec_mouse_mode")) as case:
                error_file = case.suite.stderr_path
                engine_stderr_offset = error_file.stat().st_size
                app_launch = f"exec {shlex.quote(binary)} tui 2>>{shlex.quote(str(error_file))}"
                launch = f"exec script -qef -c {shlex.quote(app_launch)} {shlex.quote(str(terminal_bytes))}"
                case.check(cli("status")["backend"] == "stopped", 'Status reports that the native app is stopped.')
                cli("rss", "add", base + "/rss", "--name", "Local feed")
                case.check(not socket_path.exists(), 'Local RSS commands leave the torrent engine closed.')
                case.check(len(cli("rss", "items", "Local feed")["items"]) == 1, 'The local feed initially contains one article.')
                fixture["version"] = 2
                harness.start_tmux(tmux, session, launch)
                wait_for(lambda: socket_path.exists(), "native socket")
                wait_for(lambda: cli("status").get("ok"), "native status")
                wait_for(lambda: terminal_bytes.exists() and b"\x1b[?1003h" in terminal_bytes.read_bytes(),
                         "actual terminal startup enables unpressed pointer motion")
            if options.case == "dec_mouse_mode":
                return suite_status("native")
            with Case("native", "closed_status_stays_closed", enabled=options.case in (None, "closed_status_stays_closed")) as case:
                case.check(cli("capabilities")["contract_version"], 'Capabilities include the contract version.')
                cli("stop", "zzzz9999", code=6)
                cli("stop", code=2)
                wait_for(lambda: len(cli("rss", "items", "Local feed")["items"]) == 2,
                         "one launch refresh sees new RSS article")
                case.check(fixture["feed_requests"] == 2, 'Starting the app refreshes the feed exactly once.', f"HTTP fixture recorded {fixture['feed_requests']} feed requests and {fixture['torrent_requests']} torrent requests; phase {fixture['phase']}.")
            if options.case == "closed_status_stays_closed":
                return suite_status("native")
            with Case("native", "one_enter_adds_once", enabled=options.case in (None, "one_enter_adds_once")) as case:
                click_text("RSS")
                wait_for(lambda: "Fixture episode 2" in capture(), "RSS item view")
                fixture["phase"] = "rss-first-click-and-add"
                click_text("Fixture episode 2")
                time.sleep(.2)
                case.check(fixture["torrent_requests"] == 0, 'A single RSS click selects without downloading.', f"HTTP fixture recorded {fixture['feed_requests']} feed requests and {fixture['torrent_requests']} torrent requests; phase {fixture['phase']}.")
                keys("Enter")
                wait_for(lambda: cli("rss", "items", "Local feed")["items"][0]["isRead"],
                         "one Enter marks article read after add")
                wait_for(lambda: "added Fixture episode 2" in capture(), "add result reaches the TUI")
                case.check(fixture["torrent_requests"] == 1, 'Only the explicit RSS Enter action downloads a torrent once.', f"HTTP fixture recorded {fixture['feed_requests']} feed requests and {fixture['torrent_requests']} torrent requests; phase {fixture['phase']}.")
                listed = cli("list")["torrents"]
                case.check(len(listed) == 1 and listed[0]["hash"] == expected_hash, 'The torrent list contains exactly the expected fixture hash.')
            if options.case == "one_enter_adds_once":
                return suite_status("native")
            with Case("native", "refresh_preserves_read_history", enabled=options.case in (None, "refresh_preserves_read_history")) as case:
                fixture["phase"] = "rss-refresh-three"
                fixture["version"] = 3
                keys("r")
                wait_for(lambda: "Fixture episode 3" in capture(), "one refresh updates view")
                case.check(fixture["feed_requests"] == 3, 'Pressing refresh requests the feed exactly once more.', f"HTTP fixture recorded {fixture['feed_requests']} feed requests and {fixture['torrent_requests']} torrent requests; phase {fixture['phase']}.")
                items = cli("rss", "items", "Local feed")["items"]
                case.check(next(item for item in items if item["id"] == "episode-2")["isRead"], 'Refresh preserves the read marker for episode two.')
                case.check(not next(item for item in items if item["id"] == "episode-3")["isRead"], 'The new third episode is unread.')
            if options.case == "refresh_preserves_read_history":
                return suite_status("native")
            with Case("native", "rss_mouse_and_scoped_shortcuts", enabled=options.case in (None, "rss_mouse_and_scoped_shortcuts")) as case:
                fixture["phase"] = "rss-new-first-row-click"
                click_text("Fixture episode 3")
                keys("m")
                wait_for(lambda: cli("rss", "items", "Local feed")["items"][0]["isRead"], "single item marked read")
                wait_for(lambda: "(m) Mark unread" in capture(), "read change reaches the visible TUI")
                keys("m")
                wait_for(lambda: not cli("rss", "items", "Local feed")["items"][0]["isRead"], "same item marked unread")
                wait_for(lambda: "(m) Mark read" in capture(), "unread change reaches the visible TUI")
                keys("Left", "m")
                time.sleep(.2)
                case.check(not cli("rss", "items", "Local feed")["items"][0]["isRead"], 'Lowercase m in the parent pane leaves the feed unread.')
                keys("M")
                wait_for(lambda: all(item["isRead"] for item in cli("rss", "items", "Local feed")["items"]), "Shift+M marks this feed read")
                wait_for(lambda: "marked Local feed as read" in capture(), "batch read change reaches the visible TUI")
            if options.case == "rss_mouse_and_scoped_shortcuts":
                return suite_status("native")
            with Case("native", "mouse_selection_at_scroll_offset", enabled=options.case in (None, "mouse_selection_at_scroll_offset")) as case:
                fixture["phase"] = "rss-large-refresh"
                fixture["version"] = 45
                keys("r")
                wait_for(lambda: len(cli("rss", "items", "Local feed")["items"]) == 45, "large feed refresh")
                wait_for(lambda: "Fixture episode 4" in capture(), "large feed reaches the visible TUI")
                keys("Left", "Down", "Right")
                wait_for(lambda: "Fixture episode 45" in capture(), "large feed refresh")
                keys(*("Down" for _ in range(40)))
                wait_for(lambda: "Fixture episode 6" in capture() and "Fixture episode 5" in capture(),
                         "feed is scrolled beyond the first page")
                case.check("Fixture episode 45" not in capture(), 'Scrolling moves episode 45 out of the visible pane.')
                fixture["phase"] = "rss-scrolled-click"
                click_text("Fixture episode 6")
                keys("m")
                wait_for(lambda: next(item for item in cli("rss", "items", "Local feed")["items"]
                                      if item["id"] == "episode-6")["isRead"],
                         "mouse uses the scrolled row's item")
                wait_for(lambda: "(m) Mark unread" in capture(), "scrolled read change reaches the visible TUI")
                mouse_text("Fixture episode 5", 35)
                keys("m")
                wait_for(lambda: not next(item for item in cli("rss", "items", "Local feed")["items"]
                                          if item["id"] == "episode-6")["isRead"],
                         "hover retains the clicked keyboard target")
                case.check(not next(item for item in cli("rss", "items", "Local feed")["items"]
                                if item["id"] == "episode-5")["isRead"], 'Hovering episode five does not change its read marker.')
                case.check(fixture["torrent_requests"] == 1, 'Only the explicit RSS Enter action downloads a torrent once.', f"HTTP fixture recorded {fixture['feed_requests']} feed requests and {fixture['torrent_requests']} torrent requests; phase {fixture['phase']}.")
            if options.case == "mouse_selection_at_scroll_offset":
                return suite_status("native")
            with Case("native", "native_cli_fields_and_deletion_safety", enabled=options.case in (None, "native_cli_fields_and_deletion_safety")) as case:
                cli("stop", expected_hash)
                details = cli("show", expected_hash)
                case.check(details["files"][0]["name"] == name, 'The native file list preserves the full fixture filename.')
                case.check(details["up_limit"] == 32768, 'The native upload cap is 32768 bytes per second.')
                case.check(details["state"] == "stoppedDL", 'The explicitly stopped torrent remains stopped.')
                cli("delete", expected_hash, "--files", code=3)
            if options.case == "native_cli_fields_and_deletion_safety":
                return suite_status("native")
            with Case("native", "resume_done_refused", enabled=options.case in (None, "resume_done_refused")) as case:
                resume_done_refused(harness, root)
                cli("stop", expected_hash)
            if options.case == "resume_done_refused":
                return suite_status("native")
            with Case("native", "bulk_start_requires_hashes", enabled=options.case in (None, "bulk_start_requires_hashes")) as case:
                bulk_start_requires_hashes(case)
            if options.case == "bulk_start_requires_hashes":
                return suite_status("native")
            if options.case in (None, "no_fake_seed_counts"):
                with Case("native", "no_fake_seed_counts") as case:
                    no_fake_seed_counts(case, root, cli, tmux, keys, capture, wait_for, session)
                if options.case == "no_fake_seed_counts":
                    return suite_status("native")
            with Case("native", "shows_inbox_and_targets", enabled=options.case in (None, "shows_inbox_and_targets")) as case:
                keys("1")
                wait_for(lambda: "P0-10.complete.txt" not in capture(), "completed fixture removed from TUI")
                keys("Enter")
                wait_for(lambda: "Files" in capture(), "file modal")
                case.check("high" not in capture().lower() and "max" not in capture().lower(), 'The native file modal offers no unsupported high or max priorities.')
                keys("Escape")
                wait_for(lambda: "Files ·" not in capture(), "file modal closes")
                case.check("3 Shows (3)" in capture(), 'The Torrents screen displays the Shows badge with three unread releases.')
                keys("3")
                wait_for(lambda: "All unread" in capture() and "S01E02" in capture() and "S01E01" in capture(), "combined Shows inbox")
                case.check("Alpha Show" in capture() and "Beta Show" in capture(), 'The combined inbox displays releases from both followed shows.')
                case.check("(m) Mark read" in capture(), 'The selected unread release offers the mark-read action.')
                case.check(not show_episodes("tt1234002")["S01E02"]["read"], 'The other Beta Show release remains unread.')
                keys("M")
                wait_for(lambda: "Shift+M applies to one show" in capture(), "inbox batch scope is explicit")
                case.check(all(not episode["read"] for episode in show_episodes("tt1234002").values()), 'An unscoped inbox batch action leaves all Beta Show releases unread.')
                keys("m")
                wait_for(lambda: show_episodes("tt1234002")["S01E02"]["read"], "inbox read saves the selected show episode")
                wait_for(lambda: "3 Shows (2)" in capture() and "S01E02" not in "\n".join(capture().splitlines()[1:-2]), "read removes the inbox row")
                case.check(not show_episodes("tt1234002")["S01E01"]["read"], 'The unselected Beta Show episode remains unread.')
                keys("u")
                wait_for(lambda: not show_episodes("tt1234002")["S01E02"]["read"], "undo restores exact inbox release")
                wait_for(lambda: "3 Shows (3)" in capture() and "S01E02" in capture(), "undo restores the visible row")
                click_text("S01E01")
                mouse_text("Alpha Show", 35)
                keys("m")
                wait_for(lambda: show_episodes("tt1234002")["S01E01"]["read"], "hover keeps clicked inbox target")
                case.check(not show_episodes("tt1234002")["S01E02"]["read"], 'The other Beta Show release remains unread.')
                case.check(not show_episodes("tt1234001")["S01"]["read"], 'The Alpha Show season pack remains unread.')
                keys("u")
                wait_for(lambda: not show_episodes("tt1234002")["S01E01"]["read"], "undo restores clicked release")
                wait_for(lambda: "3 Shows (3)" in capture(), "restored inbox count")
                click_text("S01E01")
                fixture["phase"] = "shows-inbox-download"
                keys("Enter")
                wait_for(lambda: show_episodes("tt1234002")["S01E01"]["read"], "inbox download marks exact show release read")
                wait_for(lambda: "added Beta Show S01E01" in capture(), "inbox add feedback")
                case.check(fixture["torrent_requests"] == 2, 'One explicit Shows Enter action makes one additional torrent request.', f"HTTP fixture recorded {fixture['feed_requests']} feed requests and {fixture['torrent_requests']} torrent requests; phase {fixture['phase']}.")
                case.check(not show_episodes("tt1234002")["S01E02"]["read"], 'The other Beta Show release remains unread.')
                click_text("Alpha Show")
                keys("Enter", "M")
                wait_for(lambda: show_episodes("tt1234001")["S01"]["read"], "per-show batch includes season pack")
                case.check(not show_episodes("tt1234002")["S01E02"]["read"], 'The other Beta Show release remains unread.')
                click_text("All unread")
                keys("Enter")
                wait_for(lambda: "S01E02" in capture() and "3 Shows (1)" in capture(), "returns to combined unread inbox")
                keys("4")
                wait_for(lambda: "Fixture episode" in capture(), "RSS is direct key four")
                case.check("3 Shows (1)" in capture(), 'The current screen displays a Shows badge with one unread release.')
                keys("3")
                wait_for(lambda: "All unread" in capture() and "S01E02" in capture(), "Shows is direct key three")
            if options.case == "shows_inbox_and_targets":
                return suite_status("native")
            with Case("native", "file_modal_shows_help_80x24", enabled=options.case in (None, "file_modal_shows_help_80x24")) as case:
                keys("?")
                wait_for(lambda: "GLOBAL" in capture(), "help")
                keys("PageDown")
                keys("Escape")
                wait_for(lambda: "GLOBAL" not in capture(), "help closes")
                tmux("resize-window", "-t", session, "-x", "80", "-y", "24")
                keys("1")
                wait_for(lambda: "[Torrents]" in capture() and "…" in capture().splitlines()[-2], "small terminal footer overflow")
                pane = capture()
                case.check("…" in pane.splitlines()[-2] and "(?) Help" in pane.splitlines()[-2],
                           "At 80×24 the footer marks omitted actions with an ellipsis and retains Help.", pane)
            if options.case == "file_modal_shows_help_80x24":
                return suite_status("native")
            with Case("native", "settings_queue_vpn_restart", enabled=options.case in (None, "settings_queue_vpn_restart")) as case:
                keys("F6")
                wait_for(lambda: "Download settings" in capture(), "download settings")
                case.check("Simultaneous downloads" in capture(), 'Download settings display the simultaneous-download limit.')
                case.check("Peer encryption" in capture(), 'Download settings display peer encryption.')
                case.check("VPN connection" in capture(), 'Download settings display VPN binding.')
                keys("Left", "Enter")
                wait_for(lambda: "Download settings" not in capture(), "settings save closes panel")
                case.check(cli("settings").get("max_active_downloads") == 2, 'The settings panel persists a two-download limit.')
                keys("F6", "Right", "Enter")
                wait_for(lambda: "Download settings" not in capture() and cli("settings").get("max_active_downloads") == 3,
                         "settings panel changes queue limit and persists it")
                extra_hashes = []
                for index in range(4):
                    extra_info = dict(info, name=f"Queue.fixture.{index}.txt", private=1)
                    extra_path = root / f"queue-{index}.torrent"
                    extra_path.write_bytes(bencode({"info": extra_info}))
                    extra_hashes.append(hashlib.sha1(bencode(extra_info)).hexdigest())
                    cli("add", str(extra_path), "--quiet")
                wait_for(lambda: cli("status").get("active") == 3 and cli("status").get("queued") == 1,
                         "three active downloads and one queued")
                cli("settings", "--max-downloads", "1")
                wait_for(lambda: cli("status").get("active") == 1 and cli("status").get("queued") == 3,
                         "live queue limit changes admission")
                import tomllib
                case.check(tomllib.loads((config_dir / "config.toml").read_text())["max_active_downloads"] == 1, 'The one-download limit is durable in the isolated config file.')
                cli("settings", "--max-downloads", "0", code=2)  # usage: value out of range (CLI exit-code dictionary)
                case.check(cli("settings").get("max_active_downloads") == 1, 'Rejecting a zero limit preserves the previous one-download limit.')
                for hash_ in extra_hashes:
                    cli("delete", hash_)
                cli("settings", "--max-downloads", "3")
                cli("settings", "--vpn", "fbmissing0")
                wait_for(lambda: cli("status").get("daemon", {}).get("state") == "parked", "required VPN blocks networking")
                wait_for(lambda: "Waiting for fbmissing0" in capture(), "VPN waiting is visible")
                case.check(cli("show", expected_hash).get("progress_known") is False, 'Progress is unknown while the required VPN is absent.')
                cli("stop", expected_hash)
                cli("settings", "--vpn", "off", "--encryption", "require")
                restored = harness.wait_for_reopen(socket_path, expected_hash)
                case.check(restored.get("state") == "stoppedDL", 'Engine reopening preserves the manual pause.',
                           f"Reopened torrent response: {restored}")
                case.check(cli("settings").get("peer_encryption") == "require", 'Required peer encryption persists after engine reopening.')
            if options.case == "settings_queue_vpn_restart":
                return suite_status("native")
            with Case("native", "quit_restart_down", enabled=options.case in (None, "quit_restart_down")) as case:
                owner_before = harness.api("app.info")
                keys("q")
                wait_for(lambda: tmux("has-session", "-t", session, check=False).returncode == 1, "q detaches TUI", seconds=6)
                status = cli("status")
                owner_after = harness.api("app.info")
                reported_running = status["running"] if "running" in status else (
                    status.get("engine") == "native" and status.get("connection") in ("connected", "vpn_waiting"))
                case.check(socket_path.exists() and status["ok"] and reported_running is True and
                           owner_after["instance"] == owner_before["instance"] and
                           owner_after["server"]["pid"] == owner_before["server"]["pid"] and
                           owner_after["state"] in ("starting", "running", "parked"),
                           'q leaves the same daemon running.', json.dumps(dict(status=status, before=owner_before, after=owner_after)))
                diagnostics = error_file.read_bytes()[engine_stderr_offset:].decode()
                case.check(expected_daemon_stderr(diagnostics, complete=False),
                           'Daemon and CLI stderr contain only known success phases and legacy dispatch traces.', diagnostics)
                case.check(not expected_daemon_stderr(diagnostics + "WARNING unexpected\n", complete=False) and
                           not expected_daemon_stderr(diagnostics + "INFO daemon restarting engine reason=Unknown\n", complete=False),
                           "Daemon stderr allowlist still rejects unknown lines and restart reasons")
                harness.start_tmux(tmux, session, launch)
                wait_for(lambda: socket_path.exists(), "restart socket")
                case.check(show_episodes("tt1234001")["S01"]["read"], 'Restart preserves the Alpha Show season-pack read marker.')
                case.check(show_episodes("tt1234002")["S01E01"]["read"], 'Restart preserves the exact Beta Show episode read marker.')
                case.check(not show_episodes("tt1234002")["S01E02"]["read"], 'The other Beta Show release remains unread.')
                wait_for(lambda: "3 Shows (1)" in capture(), "reopened TUI restores unread Shows badge")
                case.check("3 Shows (1)" in capture(), 'The current screen displays a Shows badge with one unread release.')
                restored = cli("show", expected_hash)
                case.check(restored["state"] == "stoppedDL" and restored["up_limit"] == 32768, 'Restart preserves both the manual pause and upload cap.')
                case.check(next(item for item in cli("rss", "items", "Local feed")["items"]
                            if item["id"] == "episode-2")["isRead"], 'Restart preserves the RSS episode-two read marker.')
                cli("down")
                wait_for(lambda: not socket_path.exists(), "CLI down closes app", seconds=6)
                case.check(cli("status")["backend"] == "stopped", 'Status reports that the native app is stopped.')
            if options.case == "quit_restart_down":
                return suite_status("native")
            with Case("native", "resume_unknown_timestamp", enabled=options.case in (None, "resume_unknown_timestamp")) as case:
                # Existing on-disk records deliberately retain legacy -1 dates.
                # A separate root cannot race the preceding app's final save.
                parked_root = root / "parked"
                parked_root.mkdir(mode=0o700)
                (parked_root / "downloads").mkdir(mode=0o700)
                parked_env = dict(env)
                for key, directory in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                                       ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CACHE_HOME", "cache"),
                                       ("XDG_STATE_HOME", "state")]:
                    path = parked_root / directory
                    path.mkdir(mode=0o700)
                    parked_env[key] = str(path)
                parked_harness = NativeHarness(lambda: case, binary, parked_env)
                parked_cli = parked_harness.cli
                records = [dict(hash=f"{index:040x}", name=f"P0-10.timestamp.{stamp}.fixture",
                                source=f"magnet:?xt=urn:btih:{index:040x}", output_folder=str(parked_root / "downloads"),
                                only_files=None, upload_limit=32768, ratio_limit=0.0, category="", tags="",
                                added_on=checked-index, completion_on=stamp, paused=True, resume_after_migration=False)
                           for index, stamp in [(1, -1), (2, 0), (3, 123)]]
                ledger = parked_root / "data/fbittorrent/engine/torrents.json"
                ledger.parent.mkdir(parents=True)
                ledger.write_text(json.dumps(dict(migrated_legacy=True, records=records)))
                parked_config = parked_root / "config/fbittorrent/config.toml"
                parked_config.parent.mkdir()
                # The missing interface prevents any torrent networking.
                parked_config.write_text(
                    f'engine = "native"\nautostart = false\nnotify = false\npeer_port = {peer_port}\n'
                    f'network_interface = "fbmissing0"\nsave_path = {json.dumps(str(parked_root / "downloads"))}\n')
                record("restore existing records with completion dates -1, 0 and 123",
                       "unknown dates remain unmodified on disk before startup", ledger.read_text())
                environment = shlex.join([f"{key}={parked_env[key]}" for key in parked_env if key.startswith("XDG_")])
                parked_launch = (f"exec env {environment} script -qef -c {shlex.quote(app_launch)} "
                                 f"{shlex.quote(str(parked_root / 'terminal.typescript'))}")
                keys("q")
                parked_harness.ensure_serve()
                harness.start_tmux(tmux, session, parked_launch)
                parked_socket = parked_root / "runtime/fbittorrent/engine.sock"
                def parked_without_session():
                    if not parked_socket.exists(): return False
                    owner = parked_harness.api("app.info")
                    network = parked_harness.api("app.network")["state"]
                    return owner["state"] == "parked" and network["ready"] is False and network["interface"] == "fbmissing0"
                wait_for(parked_without_session, "parked engine fixture opens without a network session")
                listed = {torrent["hash"]: torrent for torrent in parked_cli("list")["torrents"]}
                unknown, zero, done = [item["hash"] for item in records]
                normalized = {row["hash"]: row for row in parked_harness.api("torrents.list")["torrents"]}
                case.check(normalized[unknown]["intent"] == "pause" and normalized[unknown]["state"] == "waiting_vpn" and
                           "completed_at" not in normalized[unknown] and "progress" not in normalized[unknown] and
                           normalized[done]["completed_at"] == 123,
                           "API preserves pause intent, omits unknown completion, and keeps the known date", json.dumps(normalized))
                # API omits an unknown date; contract-1.x shapes it as zero.
                # The legacy adapter retains the disk sentinel. Both must stay incomplete.
                expected_cli_unknown = 0 if "native_state" in listed[unknown] else -1
                case.check(listed[unknown]["completion_on"] == expected_cli_unknown and listed[unknown]["state"] == "stoppedDL",
                           "A saved -1 record remains incomplete.", json.dumps(listed[unknown]))
                keys("1")
                wait_for(lambda: "P0-10.timestamp.-1.fixture" in capture(), "unknown-date fixture reaches the TUI")
                keys("Home")  # The -1 record has the newest added_on timestamp.
                wait_for(lambda: "(p) Resume" in capture(),
                         "unknown-date selection keeps the incomplete-row Resume control")
                keys("p")
                wait_for(lambda: parked_harness.api("torrents.get",dict(hash=unknown))["torrent"]["intent"] == "run",
                         "Resume in the TUI changes the paused VPN-waiting record to run intent")
                wait_for(lambda: "(p) Pause" in capture(), "VPN-waiting run intent offers Pause")
                keys("p")
                wait_for(lambda: parked_harness.api("torrents.get",dict(hash=unknown))["torrent"]["intent"] == "pause",
                         "Pause in the TUI restores saved pause intent without opening a network session")
                wait_for(lambda: "(p) Resume" in capture(), "VPN-waiting paused intent offers Resume again")
                result = parked_cli("start", unknown)
                case.check(result["hashes"] == [unknown] and result["skipped"] == [],
                           "Explicit start accepts the existing -1 record.", json.dumps(result))
                parked_cli("stop", unknown)
                result = parked_cli("start", "all")
                case.check(set(result["hashes"]) == {unknown, zero} and
                           result["skipped"] == [dict(hash=done, reason="complete_no_seed")],
                           "Bulk start accepts -1 and zero dates and skips only the positive date.", json.dumps(result))
                result = parked_cli("start", done, code=1)
                case.check(result == dict(ok=False, code=1, code_name="failure",
                                         error="already complete; seeding is off for this torrent",
                                         hint=f"fbittorrent seed {done}"),
                           "A positive completion date still refuses explicit start with the seed hint.", json.dumps(result))
                case.check(json.loads(ledger.read_text())["records"][0]["completion_on"] == -1,
                           "Starting an unknown-date record does not invent a completion timestamp.")
                parked_cli("down")
                wait_for(lambda: not parked_socket.exists(), "parked fixture closes without a stale socket", seconds=6)
            if options.case == "resume_unknown_timestamp":
                return suite_status("native")
            with Case("native", "store_modes", enabled=options.case in (None, "store_modes")) as case:
                store_modes(case, root, env, cli, base, fixture)
            if options.case == "store_modes":
                return suite_status("native")
        except Exception:
            print(f"fixture state: {fixture}", file=sys.stderr)
            print(capture(log=False), file=sys.stderr)
            if error_file.exists():
                print(error_file.read_text(), file=sys.stderr)
            raise
        finally:
            if options.case in (None, "no_fake_seed_counts") and terminal_bytes.exists():
                terminal_artifact = REPO / "target/test-logs/native/no_fake_seed_counts.terminal.typescript"
                terminal_artifact.write_bytes(terminal_bytes.read_bytes())
                print(f"Terminal stream retained: {terminal_artifact}")
            tmux("kill-server", check=False)
            harness.close_serve()
            if 'parked_harness' in locals(): parked_harness.close_serve()
            server.shutdown()
    if options.case is None:
        terminal_close_is_graceful(binary)
    # This case deliberately captures the widget's human error on stderr.
    # Run it after the lifecycle cases' assertion of otherwise empty stderr.
    if options.case is None:
        launcher_never_called(binary)
    return suite_status("native")


if __name__ == "__main__":
    sys.exit(main())
