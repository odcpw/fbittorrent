#!/usr/bin/env python3
"""Exercise app VPN loss/reconnect in an owned Linux network namespace.

The parent captures its network namespace, then unshare creates a private user
and network namespace. All interface mutations and all XDG files belong to
that fixture. The user's VPN, files and running app are untouched.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, dict):
        return b"d" + b"".join(bencode(key) + bencode(value[key]) for key in sorted(value)) + b"e"
    raise TypeError(value)


def child(binary, fixture):
    assert os.readlink("/proc/self/ns/net") != os.environ["FBT_TEST_HOST_NETNS"]
    print("exercising whole app inside disposable namespace", os.readlink("/proc/self/ns/net"), flush=True)

    def ip(*arguments):
        subprocess.run(["ip", *arguments], check=True, capture_output=True)

    ip("link", "set", "lo", "up")
    assert [entry["ifname"] for entry in json.loads(subprocess.check_output(["ip", "-j", "link", "show"]))] == ["lo"]
    fixture = Path(fixture)
    env = os.environ.copy()
    for key, leaf in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                      ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime")]:
        path = fixture / leaf
        path.mkdir(mode=0o700)
        env[key] = str(path)
    env["TERM"] = "xterm-256color"
    config_dir = fixture / "config/fbittorrent"
    config_dir.mkdir()
    config = config_dir / "config.toml"
    config.write_text('engine = "native"\npeer_port = 0\nmax_active_downloads = 1\n'
                      'peer_encryption = "require"\nnetwork_interface = "proton"\n'
                      f'save_path = {json.dumps(str(fixture / "downloads"))}\n'
                      'notify = false\nautostart = false\n')
    config_before = config.read_bytes()
    engine_dir = fixture / "data/fbittorrent/engine"
    engine_dir.mkdir(parents=True)
    records = []
    for number, paused in [(1, False), (2, False), (3, True)]:
        name = f"vpn-fixture-{number}.bin"
        info = {b"name": name.encode(), b"length": 8, b"piece length": 16384,
                b"pieces": hashlib.sha1(b"fixture!").digest(), b"private": 1}
        digest = hashlib.sha1(bencode(info)).hexdigest()
        (engine_dir / f"{digest}.torrent").write_bytes(bencode({b"info": info}))
        records.append(dict(hash=digest, name=name, source="", output_folder=str(fixture / "downloads"),
                            only_files=None, upload_limit=32768, ratio_limit=0.0, category="fixture", tags="",
                            added_on=number, completion_on=0, paused=paused, resume_after_migration=False))
    ledger = engine_dir / "torrents.json"
    ledger.write_text(json.dumps({"migrated_legacy": True, "records": records}))
    saved_intent = {record["hash"]: record["paused"] for record in records}
    endpoint = fixture / "runtime/fbittorrent/engine.sock"
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 150, 0, 0))
    process = subprocess.Popen([binary, "tui"], stdin=slave, stdout=slave, stderr=slave,
                               env=env, start_new_session=True)
    os.close(slave)
    transcript = bytearray()
    stop = threading.Event()

    def drain():
        while not stop.is_set():
            try:
                if select.select([master], [], [], 0.1)[0]:
                    data = os.read(master, 65536)
                    if not data:
                        break
                    transcript.extend(data)
            except OSError:
                break

    reader = threading.Thread(target=drain)
    reader.start()

    def request(path, method="GET"):
        packet = json.dumps(dict(method=method, path=path, args=[], file=None)).encode()
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(2)
            stream.connect(str(endpoint))
            stream.sendall(struct.pack("!I", len(packet)) + packet)

            def read(size):
                data = bytearray()
                while len(data) < size:
                    chunk = stream.recv(size - len(data))
                    if not chunk:
                        raise ConnectionError("short IPC response")
                    data.extend(chunk)
                return data

            response = json.loads(read(struct.unpack("!I", read(4))[0]))
        assert response["error"] is None, response
        value = response["value"]
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            return value

    def inet_sockets():
        owned = set()
        for fd in (Path("/proc") / str(process.pid) / "fd").iterdir():
            try:
                link = os.readlink(fd)
                if link.startswith("socket:["):
                    owned.add(link[8:-1])
            except FileNotFoundError:
                pass
        active = set()
        for protocol in ["tcp", "tcp6", "udp", "udp6"]:
            lines = (Path("/proc") / str(process.pid) / "net" / protocol).read_text().splitlines()[1:]
            active.update(line.split()[9] for line in lines if len(line.split()) > 9)
        return owned & active

    def preserved():
        assert config.read_bytes() == config_before, "VPN transitions rewrote settings"
        current = json.loads(ledger.read_text())["records"]
        assert {record["hash"]: record["paused"] for record in current} == saved_intent
        assert len(current) == 3, "VPN transitions lost saved history"

    def wait_state(label, ready, expected_index=None, previous_sockets=None):
        began = time.monotonic()
        last = None
        while time.monotonic() - began < 10:
            assert process.poll() is None, f"app exited: {process.returncode}\n{transcript[-1500:].decode(errors='replace')}"
            try:
                last = request("app/network")
                if last["ready"] == ready and (expected_index is None or last["index"] == expected_index):
                    torrents = request("torrents/info")
                    assert len(torrents) == 3
                    active = inet_sockets()
                    if ready:
                        assert active, "ready app has no native network session"
                        if previous_sockets is not None:
                            assert active.isdisjoint(previous_sockets), "replacement retained old engine sockets"
                    else:
                        assert last["message"] == "Waiting for ProtonVPN"
                        assert not active, f"parked app retains native network sockets: {active}"
                        by_hash = {entry["hash"]: entry for entry in torrents}
                        for digest, paused in saved_intent.items():
                            assert by_hash[digest]["state"] == ("stoppedDL" if paused else "waitingVPN"), by_hash[digest]
                    preserved()
                    print(f"PASS {label}: ready={ready} index={last['index']} sockets={len(active)} transition_ms={(time.monotonic()-began)*1000:.1f}", flush=True)
                    return active
            except (OSError, ConnectionError, AssertionError) as error:
                last = str(error)
            time.sleep(0.1)
        raise AssertionError(f"{label} did not settle: {last}")

    try:
        wait_state("initial absent Proton", False)
        ip("link", "add", "proton0", "type", "dummy")
        wait_state("Proton device down", False)
        ip("address", "add", "198.18.0.1/32", "dev", "proton0")
        ip("link", "set", "proton0", "up")
        first_index = socket.if_nametoindex("proton0")
        first_sockets = wait_state("Proton becomes available", True, first_index)
        ip("link", "set", "proton0", "down")
        wait_state("live Proton goes down", False)
        ip("link", "set", "proton0", "up")
        restored_sockets = wait_state("same Proton recovers", True, first_index, first_sockets)
        ip("link", "delete", "proton0")
        wait_state("live Proton disappears", False)
        ip("link", "add", "proton0", "type", "dummy")
        ip("address", "add", "198.18.0.1/32", "dev", "proton0")
        ip("link", "set", "proton0", "up")
        second_index = socket.if_nametoindex("proton0")
        assert second_index != first_index
        replaced_sockets = wait_state("same-name replacement starts new session", True, second_index, restored_sockets)
        ip("link", "delete", "proton0")
        ip("link", "add", "proton0", "type", "dummy")
        ip("address", "add", "198.18.0.1/32", "dev", "proton0")
        ip("link", "set", "proton0", "up")
        third_index = socket.if_nametoindex("proton0")
        assert third_index != second_index
        wait_state("rapid replacement also drains old session", True, third_index, replaced_sockets)
        request("app/shutdown", "POST")
        process.wait(timeout=10)
        assert process.returncode == 0
        assert not endpoint.exists(), "closed app left engine socket behind"
        preserved()
        print("PASS close: process exit=0, IPC socket removed, settings and saved pause/queue intent preserved", flush=True)
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        stop.set()
        reader.join(timeout=2)
        os.close(master)
        (fixture / "tui-output.txt").write_bytes(transcript)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--child":
        child(sys.argv[2], sys.argv[3])
    else:
        binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/fbittorrent").resolve())
        assert Path(binary).is_file(), "build the app before running its network fixture"
        with tempfile.TemporaryDirectory(prefix="fbittorrent-vpn-fixture-") as directory:
            subprocess.run(["unshare", "-Urn", "env", f"FBT_TEST_HOST_NETNS={os.readlink('/proc/self/ns/net')}",
                            sys.executable, str(Path(__file__).resolve()), "--child", binary, directory], check=True)
