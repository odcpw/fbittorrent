#!/usr/bin/env python3
"""Exercise the real native CLI/TUI with local RSS, saved Shows and torrent fixtures.

Every config, download and socket belongs to a temporary directory. No user's
service or torrents are touched. Requires tmux, util-linux script and Python 3.
"""
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


def wait_for(check, label, seconds=10):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = check()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError(f"timed out: {label}")


def resume_done_refused(binary, env, root, daemon_stderr):
    """Verify the real CLI/socket/engine refusal on locally verified bytes."""
    suite = "native-p0-10-" + str(time.time_ns())
    logs = Path("target/test-logs") / suite
    logs.mkdir(parents=True)
    case = "resume_done_refused"
    artifact = logs / (case + ".cli.log")
    artifact.touch()
    daemon_log = logs / (case + ".daemon.log")
    step = 0
    started = time.monotonic()

    def record(action, expect, observed, ok=True, final=False):
        nonlocal step, started
        step += 1
        now = time.monotonic()
        daemon_log.write_bytes(daemon_stderr.read_bytes() if daemon_stderr.exists() else b"")
        row = dict(ts=datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z"),
                   suite=suite, case=case, step=step, action=action, expect=expect,
                   observed=observed, ok=ok, ms=round((now-started)*1000), artifacts=[str(artifact), str(daemon_log)])
        if final:
            row["final"] = True
        with (logs / (case + ".jsonl")).open("a") as output:
            output.write(json.dumps(row) + "\n")
        print(json.dumps(row), flush=True)
        if os.environ.get("FBT_TEST_VERBOSE") == "1":
            print(json.dumps(row), file=sys.stderr, flush=True)
        started = now

    def command(*args):
        result = subprocess.run([binary, *args, "--json"], env=env,
                                capture_output=True, text=True, timeout=20)
        with artifact.open("a") as output:
            output.write(f"{args!r}: exit {result.returncode}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}\n")
        return result.returncode, json.loads(result.stdout)

    try:
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
        code, added = command("add", str(torrent), "--quiet")
        assert code == 0, added
        def completed():
            code, value = command("show", hash_)
            return value if code == 0 and value.get("completion_on", 0) != 0 and value["state"] == "stoppedUP" else None
        done = wait_for(completed, "local bytes verify and stop on completion")
        record("add local complete torrent and await verification", "completed torrent stops with seeding off", json.dumps(done))
        code, envelope = command("start", hash_)
        expected = dict(ok=False, code=1, code_name="failure",
                        error="already complete; seeding is off for this torrent", hint=f"fbittorrent seed {hash_}")
        ok = code == 1 and envelope == expected
        record("start finished stop-on-complete torrent", "exit 1; code 1, code_name failure and seed hint", json.dumps(envelope), ok)
        assert ok, (code, envelope)
        code, result = command("start", "all")
        ok = code == 0 and {"hash": hash_, "reason": "complete_no_seed"} in result["skipped"] and hash_ not in result["hashes"]
        record("start all", "exit 0; completed hash is skipped, never reported started", json.dumps(result), ok)
        assert ok, result
        code, result = command("delete", hash_)
        assert code == 0 and (downloads / name).read_bytes() == content, result
        record("remove disposable torrent, retaining verified bytes", "exit 0; fixture removed and files retained", json.dumps(result), final=True)
    except Exception as error:
        record("case failure", "all steps pass", str(error), False, final=True)
        for path in [logs / (case + ".jsonl"), artifact, daemon_log]:
            print(f"{path}:\n" + "\n".join(path.read_text().splitlines()[-20:]), file=sys.stderr)
        raise


def main():
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/fbittorrent").resolve())
    with tempfile.TemporaryDirectory(prefix="fbittorrent-native-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
                   XDG_DATA_HOME=str(root / "data"), XDG_RUNTIME_DIR=str(root / "runtime"),
                   XDG_STATE_HOME=str(root / "state"), XDG_CACHE_HOME=str(root / "cache"), TERM="xterm-256color")
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
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

        def cli(*args, code=0):
            result = subprocess.run([binary, *args, "--json"], env=env,
                                    capture_output=True, text=True, timeout=20)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout)

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

        tmux_socket = f"fbt-native-{os.getpid()}"
        session = "fbt-fbt-p0-10-xeq"

        def tmux(*args, check=True):
            result = subprocess.run(["tmux", "-L", tmux_socket, *args], env=env,
                                    capture_output=True, text=True)
            if check and result.returncode:
                raise AssertionError(f"tmux {args!r}: {result.stderr.strip()}")
            return result

        def capture():
            return tmux("capture-pane", "-t", session, "-p", check=False).stdout

        def keys(*args):
            for key in args:
                tmux("send-keys", "-t", session, *(["-l", key] if len(key) == 1 else [key]))

        def mouse_text(text, button):
            rows = capture().splitlines()
            y, row = next((y, row) for y, row in enumerate(rows) if text in row)
            x = row.index(text) + len(text) // 2
            sequence = f"\x1b[<{button};{x+1};{y+1}M"
            if button == 0:
                sequence += f"\x1b[<0;{x+1};{y+1}m"
            tmux("send-keys", "-t", session, "-l", sequence)

        def click_text(text):
            mouse_text(text, 0)

        socket_path = Path(env["XDG_RUNTIME_DIR"]) / "fbittorrent/engine.sock"
        error_file = root / "tui.stderr"
        terminal_bytes = root / "terminal.typescript"
        app_launch = f"exec {shlex.quote(binary)} tui 2>{shlex.quote(str(error_file))}"
        launch = f"exec script -qef -c {shlex.quote(app_launch)} {shlex.quote(str(terminal_bytes))}"
        try:
            assert cli("status")["backend"] == "stopped"
            cli("rss", "add", base + "/rss", "--name", "Local feed")
            assert not socket_path.exists(), "local feed commands opened torrent engine"
            assert len(cli("rss", "items", "Local feed")["items"]) == 1
            fixture["version"] = 2
            tmux("new-session", "-d", "-s", session, "-x", "120", "-y", "30", launch)
            # Keep the isolated server alive between quit and restart. Socket
            # removal precedes terminal exit; neither is tmux session teardown.
            tmux("set-option", "-s", "exit-empty", "off")
            wait_for(lambda: socket_path.exists(), "native socket")
            wait_for(lambda: cli("status").get("ok"), "native status")
            wait_for(lambda: terminal_bytes.exists() and b"\x1b[?1003h" in terminal_bytes.read_bytes(),
                     "actual terminal startup enables unpressed pointer motion")
            print("ok: actual terminal startup emits DEC mouse mode 1003")
            assert cli("capabilities")["contract_version"]
            cli("stop", "zzzz9999", code=6)
            cli("stop", code=2)
            wait_for(lambda: len(cli("rss", "items", "Local feed")["items"]) == 2,
                     "one launch refresh sees new RSS article")
            assert fixture["feed_requests"] == 2, fixture
            print("ok: closed status stays closed; local RSS works; launch refreshes once")

            click_text("RSS")
            wait_for(lambda: "Fixture episode 2" in capture(), "RSS item view")
            fixture["phase"] = "rss-first-click-and-add"
            click_text("Fixture episode 2")
            time.sleep(.2)
            assert fixture["torrent_requests"] == 0, "single click downloaded an RSS item"
            keys("Enter")
            wait_for(lambda: cli("rss", "items", "Local feed")["items"][0]["isRead"],
                     "one Enter marks article read after add")
            wait_for(lambda: "added Fixture episode 2" in capture(), "add result reaches the TUI")
            assert fixture["torrent_requests"] == 1, "Enter added more than once"
            listed = cli("list")["torrents"]
            assert len(listed) == 1 and listed[0]["hash"] == expected_hash
            print("ok: one Enter adds once and marks the RSS article read")

            fixture["phase"] = "rss-refresh-three"
            fixture["version"] = 3
            keys("r")
            wait_for(lambda: "Fixture episode 3" in capture(), "one refresh updates view")
            assert fixture["feed_requests"] == 3, fixture
            items = cli("rss", "items", "Local feed")["items"]
            assert next(item for item in items if item["id"] == "episode-2")["isRead"]
            assert not next(item for item in items if item["id"] == "episode-3")["isRead"]
            print("ok: one refresh updates the visible view and preserves read history")

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
            assert not cli("rss", "items", "Local feed")["items"][0]["isRead"], "parent-pane m marked the whole feed"
            keys("M")
            wait_for(lambda: all(item["isRead"] for item in cli("rss", "items", "Local feed")["items"]), "Shift+M marks this feed read")
            wait_for(lambda: "marked Local feed as read" in capture(), "batch read change reaches the visible TUI")
            print("ok: actual RSS tab/item mouse clicks and scoped read/unread shortcuts")

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
            assert "Fixture episode 45" not in capture(), "list did not scroll"
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
            assert not next(item for item in cli("rss", "items", "Local feed")["items"]
                            if item["id"] == "episode-5")["isRead"]
            assert fixture["torrent_requests"] == 1, "selection or hover downloaded an item"
            print("ok: actual mouse selection at nonzero scroll offset; hover keeps keyboard target")

            cli("stop", expected_hash)
            details = cli("show", expected_hash)
            assert details["files"][0]["name"] == name
            assert details["up_limit"] == 32768
            assert details["state"] == "stoppedDL"
            cli("delete", expected_hash, "--files", code=3)
            print("ok: native CLI fields, file selection, cap and deletion safety")
            resume_done_refused(binary, env, root, error_file)
            cli("stop", expected_hash)

            keys("1")
            wait_for(lambda: "P0-10.complete.txt" not in capture(), "completed fixture removed from TUI")
            keys("Enter")
            wait_for(lambda: "Files" in capture(), "file modal")
            assert "high" not in capture().lower() and "max" not in capture().lower()
            keys("Escape")
            wait_for(lambda: "Files ·" not in capture(), "file modal closes")
            assert "3 Shows (3)" in capture(), "badge is visible from Torrents"
            keys("3")
            wait_for(lambda: "All unread" in capture() and "S01E02" in capture() and "S01E01" in capture(), "combined Shows inbox")
            assert "Alpha Show" in capture() and "Beta Show" in capture()
            assert "(M)ark read" in capture()
            assert not show_episodes("tt1234002")["S01E02"]["read"]
            keys("M")
            wait_for(lambda: "Shift+M applies to one show" in capture(), "inbox batch scope is explicit")
            assert all(not episode["read"] for episode in show_episodes("tt1234002").values())
            keys("m")
            wait_for(lambda: show_episodes("tt1234002")["S01E02"]["read"], "inbox read saves the selected show episode")
            wait_for(lambda: "3 Shows (2)" in capture() and "S01E02" not in "\n".join(capture().splitlines()[1:-2]), "read removes the inbox row")
            assert not show_episodes("tt1234002")["S01E01"]["read"]
            keys("u")
            wait_for(lambda: not show_episodes("tt1234002")["S01E02"]["read"], "undo restores exact inbox release")
            wait_for(lambda: "3 Shows (3)" in capture() and "S01E02" in capture(), "undo restores the visible row")
            click_text("S01E01")
            mouse_text("Alpha Show", 35)
            keys("m")
            wait_for(lambda: show_episodes("tt1234002")["S01E01"]["read"], "hover keeps clicked inbox target")
            assert not show_episodes("tt1234002")["S01E02"]["read"]
            assert not show_episodes("tt1234001")["S01"]["read"]
            keys("u")
            wait_for(lambda: not show_episodes("tt1234002")["S01E01"]["read"], "undo restores clicked release")
            wait_for(lambda: "3 Shows (3)" in capture(), "restored inbox count")
            click_text("S01E01")
            fixture["phase"] = "shows-inbox-download"
            keys("Enter")
            wait_for(lambda: show_episodes("tt1234002")["S01E01"]["read"], "inbox download marks exact show release read")
            wait_for(lambda: "added Beta Show S01E01" in capture(), "inbox add feedback")
            assert fixture["torrent_requests"] == 2, "one inbox download dispatches once"
            assert not show_episodes("tt1234002")["S01E02"]["read"]
            click_text("Alpha Show")
            keys("Enter", "M")
            wait_for(lambda: show_episodes("tt1234001")["S01"]["read"], "per-show batch includes season pack")
            assert not show_episodes("tt1234002")["S01E02"]["read"]
            click_text("All unread")
            keys("Enter")
            wait_for(lambda: "S01E02" in capture() and "3 Shows (1)" in capture(), "returns to combined unread inbox")
            keys("4")
            wait_for(lambda: "Fixture episode" in capture(), "RSS is direct key four")
            assert "3 Shows (1)" in capture(), "badge is visible from RSS"
            keys("3")
            wait_for(lambda: "All unread" in capture() and "S01E02" in capture(), "Shows is direct key three")
            print("ok: combined Shows inbox, exact read/undo/download targets, parent batch scope and tab badge/order")
            keys("?")
            wait_for(lambda: "GLOBAL" in capture(), "help")
            keys("PageDown")
            keys("Escape")
            wait_for(lambda: "GLOBAL" not in capture(), "help closes")
            tmux("resize-window", "-t", session, "-x", "80", "-y", "24")
            keys("1")
            wait_for(lambda: "Torrents" in capture(), "small terminal")
            print("ok: native file modal, Shows, scrollable help and 80x24 terminal")

            keys("F6")
            wait_for(lambda: "Download settings" in capture(), "download settings")
            assert "Simultaneous downloads" in capture()
            assert "Peer encryption" in capture()
            assert "VPN connection" in capture()
            keys("Left", "Enter")
            wait_for(lambda: "Download settings" not in capture(), "settings save closes panel")
            assert cli("settings")["max_active_downloads"] == 2
            keys("F6", "Right", "Enter")
            wait_for(lambda: "Download settings" not in capture() and cli("settings")["max_active_downloads"] == 3,
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
            assert tomllib.loads((config_dir / "config.toml").read_text())["max_active_downloads"] == 1
            cli("settings", "--max-downloads", "0", code=1)
            assert cli("settings")["max_active_downloads"] == 1
            for hash_ in extra_hashes:
                cli("delete", hash_)
            cli("settings", "--max-downloads", "3")
            cli("settings", "--vpn", "fbmissing0")
            wait_for(lambda: cli("status").get("connection") == "vpn_waiting", "required VPN blocks networking")
            wait_for(lambda: "Waiting for fbmissing0" in capture(), "VPN waiting is visible")
            assert cli("show", expected_hash)["progress_known"] is False
            cli("stop", expected_hash)
            cli("settings", "--vpn", "off", "--encryption", "require")
            wait_for(lambda: cli("status").get("connection") != "vpn_waiting", "settings safely reopen engine")
            assert cli("show", expected_hash)["state"] == "stoppedDL"
            assert cli("settings")["peer_encryption"] == "require"
            print("ok: settings, durable live queue limit, VPN waiting and safe engine restart")

            keys("q")
            wait_for(lambda: not socket_path.exists(), "quit removes socket", seconds=6)
            wait_for(lambda: tmux("has-session", "-t", session, check=False).returncode != 0,
                     "quit closes the tmux session", seconds=6)
            assert cli("status")["backend"] == "stopped"
            assert not error_file.read_text(), error_file.read_text()
            tmux("new-session", "-d", "-s", session, "-x", "120", "-y", "30", launch)
            wait_for(lambda: socket_path.exists(), "restart socket")
            assert show_episodes("tt1234001")["S01"]["read"]
            assert show_episodes("tt1234002")["S01E01"]["read"]
            assert not show_episodes("tt1234002")["S01E02"]["read"]
            assert "3 Shows (1)" in capture()
            restored = cli("show", expected_hash)
            assert restored["state"] == "stoppedDL" and restored["up_limit"] == 32768
            assert next(item for item in cli("rss", "items", "Local feed")["items"]
                        if item["id"] == "episode-2")["isRead"]
            cli("down")
            wait_for(lambda: not socket_path.exists(), "CLI down closes app", seconds=6)
            assert cli("status")["backend"] == "stopped"
            print("ok: quit stops engine; restart preserves state; down closes app")
        except Exception:
            print(f"fixture state: {fixture}", file=sys.stderr)
            print(capture(), file=sys.stderr)
            if error_file.exists():
                print(error_file.read_text(), file=sys.stderr)
            raise
        finally:
            tmux("kill-server", check=False)
            server.shutdown()
    print("native integration passed")


if __name__ == "__main__":
    main()
