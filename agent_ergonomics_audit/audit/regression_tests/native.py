#!/usr/bin/env python3
"""Exercise the real native CLI/TUI with local RSS, saved Shows and torrent fixtures.

Every config, download and socket belongs to a temporary directory. No user's
service or torrents are touched. Requires tmux, util-linux script and Python 3.
"""
import argparse
import hashlib
import http.server
import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time

from fbtlog import Case, REPO, suite_status

CASES = (
    "dec_mouse_mode", "closed_status_stays_closed", "one_enter_adds_once",
    "refresh_preserves_read_history", "rss_mouse_and_scoped_shortcuts",
    "mouse_selection_at_scroll_offset", "native_cli_fields_and_deletion_safety",
    "shows_inbox_and_targets", "file_modal_shows_help_80x24",
    "settings_queue_vpn_restart", "quit_restart_down", "launcher_never_called",
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
            (("--no-start", "list"), 4, {}, "closed"),
            (("list", "--no-start"), 4, {}, "closed"),
            (("list",), 4, {"FBITTORRENT_NO_START": "1"}, "closed"),
            (("--no-start", "stop", "all"), 4, {}, "closed"),
            (("--no-start", "up"), 4, {}, "closed"),
            (("up",), 4, {"FBITTORRENT_NO_START": "1"}, "closed"),
            (("--no-start", "status"), 0, {}, "stopped"),
            (("--no-start", "start", "all"), 4, {}, "closed"),
            (("--no-start", "delete", "all"), 4, {}, "closed"),
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
                         "Exit 4, empty stdout, and 'fbittorrent is closed' on stderr; zero launcher calls.")
        offset = case.suite.stderr_path.stat().st_size
        child = case.popen(argv, env=env, stdout=subprocess.PIPE, text=True)
        try:
            stdout, _ = child.communicate(timeout=5)
            stderr = case.suite.stderr_path.read_bytes()[offset:].decode(errors="replace")
            step.close(child.returncode == 4 and not stdout and "fbittorrent is closed" in stderr
                       and "fbittorrent up" in stderr and not calls.read_text(),
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


class NativeHarness:
    """Record native CLI calls and retain only the asserted polling captures."""

    def __init__(self, get_case, binary, env):
        self.get_case, self.binary, self.env = get_case, binary, env
        self.polling, self.poll_records = False, []

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
                            f"app_open={status.get('app_open')}, connection={status.get('connection')}")
                if (status_exit == 0 and status.get("ok") is True and status.get("app_open") is True
                        and status.get("connection") == "connected" and present and time.monotonic() < end):
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
                    f"Socket present, status ok/app_open/connected and show {hash_} exit 0 within {seconds} seconds",
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default=str(REPO / "target/debug/fbittorrent"))
    parser.add_argument("--list", action="store_true", help="list case slugs without launching anything")
    parser.add_argument("--case", choices=CASES,
                        help="log one case, replaying its prerequisites in the isolated fixture")
    options = parser.parse_args()
    if options.list:
        print("\n".join(CASES))
        return 0
    binary = str(Path(options.binary).resolve())
    if options.case == "launcher_never_called":
        launcher_never_called(binary)
        return suite_status("native")
    with tempfile.TemporaryDirectory(prefix="fbittorrent-native-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
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
            f'engine = "native"\nautostart = false\nnotify = false\n'
            f'backend_unit = "fbittorrent-fixture-never.service"\n'
            f'url = "http://127.0.0.1:9"\npeer_port = {peer_port}\n'
            f'save_path = {json.dumps(str(root / "downloads"))}\n')

        harness = NativeHarness(lambda: case, binary, env)
        cli, record, wait_for = harness.cli, harness.record, harness.wait_for

        name = "Fixture.with.a.long.filename." + "readable." * 14 + "END.txt"
        content = b"Local torrent lifecycle fixture.\n"
        info = {"length": len(content), "name": name, "piece length": 16384,
                "pieces": hashlib.sha1(content).digest()}
        torrent = bencode({"info": info})
        (root / "fixture.torrent").write_bytes(torrent)
        expected_hash = hashlib.sha1(bencode(info)).hexdigest()
        fixture = {"version": 1, "feed_requests": 0, "torrent_requests": 0,
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

        tmux_socket = f"fbt-fbt-p0-15-3hq-{os.getpid()}"
        session = "fbt-fbt-p0-15-3hq"

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
                tmux("send-keys", "-t", session, *(["-l", key] if len(key) == 1 else [key]))

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
                wait_for(lambda: "(M)ark unread" in capture(), "read change reaches the visible TUI")
                keys("m")
                wait_for(lambda: not cli("rss", "items", "Local feed")["items"][0]["isRead"], "same item marked unread")
                wait_for(lambda: "(M)ark read" in capture(), "unread change reaches the visible TUI")
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
                wait_for(lambda: "(M)ark unread" in capture(), "scrolled read change reaches the visible TUI")
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
            with Case("native", "shows_inbox_and_targets", enabled=options.case in (None, "shows_inbox_and_targets")) as case:
                keys("1", "Enter")
                wait_for(lambda: "Files" in capture(), "file modal")
                case.check("high" not in capture().lower() and "max" not in capture().lower(), 'The native file modal offers no unsupported high or max priorities.')
                keys("Escape")
                wait_for(lambda: "Files ·" not in capture(), "file modal closes")
                case.check("3 Shows (3)" in capture(), 'The Torrents screen displays the Shows badge with three unread releases.')
                keys("3")
                wait_for(lambda: "All unread" in capture() and "S01E02" in capture() and "S01E01" in capture(), "combined Shows inbox")
                case.check("Alpha Show" in capture() and "Beta Show" in capture(), 'The combined inbox displays releases from both followed shows.')
                case.check("(M)ark read" in capture(), 'The selected unread release offers the mark-read action.')
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
                wait_for(lambda: "Torrents" in capture(), "small terminal")
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
                cli("settings", "--max-downloads", "0", code=1)
                case.check(cli("settings").get("max_active_downloads") == 1, 'Rejecting a zero limit preserves the previous one-download limit.')
                for hash_ in extra_hashes:
                    cli("delete", hash_)
                cli("settings", "--max-downloads", "3")
                cli("settings", "--vpn", "fbmissing0")
                wait_for(lambda: cli("status").get("connection") == "vpn_waiting", "required VPN blocks networking")
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
                keys("q")
                wait_for(lambda: not socket_path.exists(), "quit removes socket", seconds=6)
                case.check(cli("status")["backend"] == "stopped", 'Status reports that the native app is stopped.')
                case.check(not error_file.read_text(), 'The app and CLI have emitted no unexpected stderr.')
                harness.start_tmux(tmux, session, launch)
                wait_for(lambda: socket_path.exists(), "restart socket")
                case.check(show_episodes("tt1234001")["S01"]["read"], 'Restart preserves the Alpha Show season-pack read marker.')
                case.check(show_episodes("tt1234002")["S01E01"]["read"], 'Restart preserves the exact Beta Show episode read marker.')
                case.check(not show_episodes("tt1234002")["S01E02"]["read"], 'The other Beta Show release remains unread.')
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
        except Exception:
            print(f"fixture state: {fixture}", file=sys.stderr)
            print(capture(log=False), file=sys.stderr)
            if error_file.exists():
                print(error_file.read_text(), file=sys.stderr)
            raise
        finally:
            tmux("kill-server", check=False)
            server.shutdown()
    # This case deliberately captures the widget's human error on stderr.
    # Run it after the lifecycle cases' assertion of otherwise empty stderr.
    if options.case is None:
        launcher_never_called(binary)
    return suite_status("native")


if __name__ == "__main__":
    sys.exit(main())
