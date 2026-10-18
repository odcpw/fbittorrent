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
from datetime import datetime, timedelta, timezone
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time
from unittest.mock import patch

import fbtlog
from fbtlog import Case, REPO, suite_status

CASES = (
    "dec_mouse_mode", "closed_status_stays_closed", "one_enter_adds_once",
    "refresh_preserves_read_history", "rss_mouse_and_scoped_shortcuts",
    "mouse_selection_at_scroll_offset", "native_cli_fields_and_deletion_safety", "no_fake_seed_counts",
    "resume_done_refused", "bulk_start_requires_hashes", "shows_inbox_and_targets", "file_modal_shows_help_80x24",
    "settings_queue_vpn_restart", "quit_restart_down", "resume_unknown_timestamp",
)


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
    # The YTS JSON-to-result mapping is covered by browse's Rust unit test.
    # This Nova fixture exercises the same unknown SearchResult through the TUI.
    cache = Path(env["XDG_CACHE_HOME"]) / "fbittorrent"
    names = cache / "imdb/by-name-v2"
    names.mkdir(parents=True, exist_ok=True)
    for name in [torrent_name, "Local YTS unknown counts", "Local seeded fixture"] + [f"Fixture episode {i}" for i in range(1, 46)]:
        safe = re.sub(r"[^a-z0-9]", "_", name.lower())[:120]
        (names / (safe + ".json")).write_text('{"missing":true}')
    if not (cache / "title.ratings.tsv").exists():
        (cache / "title.ratings.tsv").write_text("tconst\taverageRating\tnumVotes\n")

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
    with tempfile.TemporaryDirectory(prefix="fbittorrent-native-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
                   XDG_DATA_HOME=str(root / "data"), XDG_RUNTIME_DIR=str(root / "runtime"),
                   XDG_CACHE_HOME=str(root / "cache"),
                   XDG_STATE_HOME=str(root / "state"), TERM="xterm-256color")
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
        if options.case in (None, "no_fake_seed_counts"):
            prepare_peer_label_fixture(env, name)
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

        tmux_socket = f"fbt-fbt-p0-10-xeq-{os.getpid()}"
        session = "fbt-fbt-p0-10-xeq"

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
                harness.start_tmux(tmux, session, parked_launch)
                parked_socket = parked_root / "runtime/fbittorrent/engine.sock"
                wait_for(lambda: parked_socket.exists() and parked_cli("status").get("connection") == "vpn_waiting",
                         "parked engine fixture opens without a network session")
                listed = {torrent["hash"]: torrent for torrent in parked_cli("list")["torrents"]}
                unknown, zero, done = [item["hash"] for item in records]
                case.check(listed[unknown]["completion_on"] == -1 and listed[unknown]["state"] == "stoppedDL",
                           "A saved -1 record remains incomplete.", json.dumps(listed[unknown]))
                keys("1")
                wait_for(lambda: "P0-10.timestamp.-1.fixture" in capture(), "unknown-date fixture reaches the TUI")
                keys("Home")  # The -1 record has the newest added_on timestamp.
                wait_for(lambda: "(P) Resume" in capture(),
                         "unknown-date selection keeps the incomplete-row Resume control")
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
            server.shutdown()
    return suite_status("native")


if __name__ == "__main__":
    sys.exit(main())
