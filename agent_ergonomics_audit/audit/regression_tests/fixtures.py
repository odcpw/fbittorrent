"""Local payloads, HTTP servers and recording PATH shims shared by process tests.

Servers bind only loopback and own their threads. Callers own payload directories;
``magnet_fixture()`` also works as a context manager with its own temporary tree.
No fixture starts a torrent client or contacts an external service.
"""
import base64
from contextvars import ContextVar
from dataclasses import dataclass
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import shlex
import socket
import struct
import tempfile
import threading
import urllib.parse

HERE = Path(__file__).resolve().parent
MIB = 1024 * 1024
_sandbox_root = ContextVar("fixture_sandbox_root", default=None)


def _directory(directory=None):
    root = directory if directory is not None else _sandbox_root.get()
    if root is None:
        raise ValueError("enter fixtures.Sandbox() or supply an explicit fixture directory")
    return Path(root)


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, str):
        value = value.encode()
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, dict):
        return b"d" + b"".join(bencode(key) + bencode(value[key]) for key in sorted(value)) + b"e"
    return b"l" + b"".join(map(bencode, value)) + b"e"


class Sandbox:
    """One short-lived tree for HOME, all five XDG variables, shims and downloads."""
    def __init__(self):
        # rch's TMPDIR can exceed AF_UNIX's pathname limit; /tmp is intentionally
        # explicit, like the short socket runner in all.sh.
        self.temporary = tempfile.TemporaryDirectory(prefix="fbt-d-", dir="/tmp")
        self.root = Path(self.temporary.name)
        self.env = dict(os.environ)
        for key in ("FBITTORRENT_NO_START", "NOTIFY_SOCKET", "WATCHDOG_PID", "WATCHDOG_USEC",
                    "LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES", "TMUX"):
            self.env.pop(key, None)
        self.keys = ["HOME", "TMPDIR", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
                     "XDG_STATE_HOME", "XDG_CACHE_HOME"]
        for key, name in zip(self.keys, ("home", "tmp", "config", "data", "run", "state", "cache")):
            path = self.root / name
            path.mkdir(mode=0o700)
            self.env[key] = str(path)
        self.unit = "fbt-fixture-" + self.root.name + ".service"
        self.env.update(FBITTORRENT_UNIT=self.unit, TERM="xterm-256color",
                        PATH=str(self.root / "shims") + os.pathsep + self.env.get("PATH", ""))
        self.keys += ["FBITTORRENT_UNIT", "TERM", "PATH"]
        self.downloads = self.root / "downloads"
        self.downloads.mkdir()
        directory = Path(self.env["XDG_CONFIG_HOME"]) / "fbittorrent"
        directory.mkdir()
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            peer_port = probe.getsockname()[1]
        (directory / "config.toml").write_text(
            'engine = "native"\nautostart = false\nnotify = false\n'
            'network_interface = "lo"\npeer_encryption = "off"\n'
            f'backend_unit = {json.dumps(self.unit)}\npeer_port = {peer_port}\n'
            f'save_path = {json.dumps(str(self.downloads))}\n')
        cache = Path(self.env["XDG_CACHE_HOME"]) / "fbittorrent"
        cache.mkdir()
        (cache / "title.ratings.tsv").write_text("tconst\taverageRating\tnumVotes\n")
        titles = cache / "imdb/by-name-v2"
        titles.mkdir(parents=True)
        for name in ("daemon_fixture_bin", "two_files", "fixture_mkv", "fixture_iso"):
            (titles / (name + ".json")).write_text('{"missing":true}')
        for name in ("notify-send", "xdg-open", "wl-paste", "wl-copy", "systemctl",
                     "systemd-inhibit", "omarchy-launch-or-focus-tui"):
            path_shim(name, "exit 99\n", self.root)

    def __enter__(self):
        self.token = _sandbox_root.set(self.root)
        return self

    def __exit__(self, *_):
        try:
            self.temporary.cleanup()
        finally:
            _sandbox_root.reset(self.token)


def path_shim(name, script, root=None):
    """Create <root>/shims/name; log one JSON argv array per invocation.

    ``script`` is a shell body (for example ``printf '%s' 'fixture text'``).
    Defaults to the current Sandbox. Pass its shims directory first in PATH.
    """
    if not re.fullmatch(r"[a-zA-Z0-9_-]+", name):
        raise ValueError("shim name must be a plain executable name")
    directory = _directory(root) / "shims"
    directory.mkdir(exist_ok=True)
    log = directory / (name + ".log")
    logger = "import json,sys; open(sys.argv[1],'a').write(json.dumps(sys.argv[2:])+'\\n')"
    path = directory / name
    path.write_text("#!/bin/sh\n" + shlex.quote(os.sys.executable) + " -c " + shlex.quote(logger) +
                    " " + shlex.quote(str(log)) + ' "$@"\n' + script + "\n")
    path.chmod(0o700)
    return path


class HttpFixture:
    """Serve files plus optional routes.json responses; never forward requests.

    Route values can name a file, JSON body, base64 body, status and headers.
    ``{base}`` in a textual response expands to this server's loopback URL.
    """
    def __init__(self, directory, routes=None):
        self.directory = Path(directory).resolve()
        manifest = self.directory / "routes.json"
        self.routes = routes if routes is not None else json.loads(manifest.read_text()) if manifest.exists() else {}
        self.closed = False
        self.requests = []
        self.lock = threading.Lock()
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def setup(self):
                super().setup()
                self.connection.settimeout(2)

            def log_message(self, *_):
                pass

            def do_GET(self):
                with fixture.lock:
                    fixture.requests.append(self.path)
                route = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path)
                response = fixture.routes.get(route, {})
                try:
                    if "json" in response:
                        body = json.dumps(response["json"]).encode()
                    elif "base64" in response:
                        body = base64.b64decode(response["base64"], validate=True)
                    else:
                        path = (fixture.directory / response.get("file", route.lstrip("/"))).resolve()
                        if not path.is_relative_to(fixture.directory):
                            self.send_error(403)
                            return
                        body = path.read_bytes()
                        if "field" in response:
                            body = json.loads(body)[response["field"]].encode()
                    if "json" in response or "field" in response or route.endswith(".json"):
                        body = body.replace(b"{base}", fixture.base.encode())
                except (OSError, ValueError):
                    self.send_error(404)
                    return
                self.send_response(response.get("status", 200))
                self.send_header("Content-Type", response.get("content_type", "application/json" if route.endswith(".json") else "application/octet-stream"))
                self.send_header("Content-Length", str(len(body)))
                for key, value in response.get("headers", {}).items():
                    self.send_header(key, value.replace("{base}", fixture.base))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except OSError:
                    pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = False
        self.base = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs=dict(poll_interval=.05), daemon=True)
        self.thread.start()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def close(self):
        if self.closed:
            return
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        if self.thread.is_alive():
            raise TimeoutError("local HTTP fixture did not stop")
        self.closed = True


def eztv_server():
    return HttpFixture(HERE / "fixtures/eztv")


def imdb_server(directory=None):
    return HttpFixture(directory or HERE / "fixtures/imdb")


def torznab_server():
    return HttpFixture(HERE / "fixtures/torznab")


@dataclass
class TorrentFixture:
    files: list
    torrent: Path
    info: dict
    magnet: str
    info_hash: str


def torrent_fixture(directory, files, name="fixture", tracker=None, piece_length=16384):
    """Build a private v1 torrent, hashing across file boundaries in order."""
    directory = Path(directory)
    info = {"name": name, "piece length": piece_length, "private": 1}
    paths = [Path(path) for path in files]
    if len(paths) == 1:
        info.update(name=paths[0].name, length=paths[0].stat().st_size)
    else:
        info["files"] = [dict(length=path.stat().st_size, path=[path.name]) for path in paths]
    pieces, pending = [], b""
    for path in paths:
        with path.open("rb") as source:
            while chunk := source.read(piece_length):
                pending += chunk
                while len(pending) >= piece_length:
                    pieces.append(hashlib.sha1(pending[:piece_length]).digest())
                    pending = pending[piece_length:]
    if pending:
        pieces.append(hashlib.sha1(pending).digest())
    info["pieces"] = b"".join(pieces)
    info_hash = hashlib.sha1(bencode(info)).hexdigest()
    document = dict(info=info)
    if tracker:
        document["announce"] = tracker
    torrent = directory / (name + ".torrent")
    torrent.write_bytes(bencode(document))
    magnet = "magnet:?" + urllib.parse.urlencode(dict(xt="urn:btih:" + info_hash, dn=info["name"], **({"tr": tracker} if tracker else {})))
    return TorrentFixture(paths, torrent, info, magnet, info_hash)


class magnet_fixture:
    """256 KiB random bytes, private torrent, local tracker and HTTP exact source.

    The tracker advertises no peers. The daemon seed/leech pair is deliberately
    deferred to P1-07's follow-up after P1-06; this is not a downloading swarm.
    """
    def __init__(self, directory=None):
        parent = directory if directory is not None else _sandbox_root.get()
        self.temporary = tempfile.TemporaryDirectory(prefix="fbt-magnet-", dir=parent or "/tmp")
        self.root = Path(self.temporary.name)
        payload = self.root / "daemon-fixture.bin"
        payload.write_bytes(os.urandom(256 * 1024))
        tracker_body = bencode(dict(interval=30, complete=0, incomplete=0, peers=b""))
        self.server = HttpFixture(self.root, {"/announce": dict(base64=base64.b64encode(tracker_body).decode())})
        self.fixture = torrent_fixture(self.root, [payload], tracker=self.server.base + "/announce")
        self.fixture.magnet += "&" + urllib.parse.urlencode(dict(xs=self.server.base + "/fixture.torrent"))

    def __enter__(self):
        return self.fixture

    def __exit__(self, *_):
        try:
            self.server.close()
        finally:
            self.temporary.cleanup()


def _random_file(path, size, prefix=b""):
    if size < len(prefix):
        raise ValueError("payload size is smaller than its header")
    with path.open("wb") as output:
        output.write(prefix)
        remaining = size - len(prefix)
        while remaining:
            chunk = os.urandom(min(remaining, MIB))
            output.write(chunk)
            remaining -= len(chunk)
    return path


def _ebml_size(size):
    for count in range(1, 9):
        if size < (1 << (7 * count)) - 1:
            return (size | (1 << (7 * count))).to_bytes(count, "big")
    raise ValueError("EBML element is too large")


def _element(identifier, body):
    return bytes.fromhex(identifier) + _ebml_size(len(body)) + body


def mkv_fixture(size=64 * MIB, duration=120.0, directory=None):
    """Random payload in an EBML Void, with Segment/Info duration (not playable video)."""
    if not 0 < duration < float("inf"):
        raise ValueError("duration must be positive and finite")
    header = _element("1a45dfa3", _element("4286", b"\x01") + _element("42f7", b"\x01") +
                      _element("42f2", b"\x04") + _element("42f3", b"\x08") +
                      _element("4282", b"matroska") + _element("4287", b"\x04") + _element("4285", b"\x02"))
    info = _element("1549a966", _element("2ad7b1", (1000000).to_bytes(3, "big")) +
                    _element("4489", struct.pack(">d", duration * 1000)))
    # Eight-byte size fields make the final file length independent of payload size.
    segment_size = size - len(header) - 12
    void_size = segment_size - len(info) - 9
    if void_size < 0:
        raise ValueError("Matroska fixture is too small")
    prefix = (header + bytes.fromhex("18538067") + (segment_size | (1 << 56)).to_bytes(8, "big") +
              info + b"\xec" + (void_size | (1 << 56)).to_bytes(8, "big"))
    return _random_file(_directory(directory) / "fixture.mkv", size, prefix)


def iso_fixture(size=4 * MIB, directory=None):
    """Non-video .iso-named random payload; not a mountable disk image."""
    return _random_file(_directory(directory) / "fixture.iso", size)


def two_file_fixture(directory=None):
    directory = _directory(directory)
    first = _random_file(directory / "first.bin", 16384 + 123)
    second = _random_file(directory / "second.bin", 256 * 1024)
    return torrent_fixture(directory, [first, second], name="two-files")


def corrupt_byte(path, offset):
    path = Path(path)
    if not 0 <= offset < path.stat().st_size:
        raise ValueError("corruption offset is outside the payload")
    with path.open("r+b") as output:
        output.seek(offset)
        old = output.read(1)[0]
        output.seek(offset)
        output.write(bytes([old ^ 0xff]))
    return old, old ^ 0xff

class tracker_server:
    """A bounded local announce fixture; logs no URL or query-string secrets."""
    def __init__(self, fail_first=2, interval_s=5, peers=(), directory=None, echo_failure=False):
        import time
        self.root = _directory(directory)
        self.log = self.root / "tracker.log"
        self.records, self.lock = [], threading.Lock()
        self.echo_failure = echo_failure
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.connection.settimeout(2)
                query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
                # parse_qs decodes arbitrary hash bytes as Unicode; preserve the
                # raw percent-encoded value for a lossless v1 hash in the log.
                raw = next((part.split("=", 1)[1] for part in urllib.parse.urlsplit(self.path).query.split("&") if part.startswith("info_hash=")), "")
                info_hash = urllib.parse.unquote_to_bytes(raw).hex()
                row = dict(ts=time.time(), info_hash=info_hash,
                           event=query.get("event", ["none"])[0], port=int(query.get("port", [0])[0]))
                with fixture.lock:
                    count = sum(item["info_hash"] == info_hash for item in fixture.records)
                    fixture.records.append(row)
                    with fixture.log.open("a") as log:
                        log.write(json.dumps(row, sort_keys=True) + "\n")
                if count < fail_first:
                    reason = "fixture: not yet"
                    if echo_failure:
                        reason += " " + fixture.url + "?passkey=fixture-private-token"
                    body = bencode({"failure reason": reason})
                else:
                    compact = b"".join(socket.inet_aton(host) + struct.pack(">H", port) for host, port in peers)
                    body = bencode(dict(interval=interval_s, peers=compact))
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except OSError:
                    pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = False
        self.url = f"http://127.0.0.1:{self.server.server_port}/announce"
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs=dict(poll_interval=.05), daemon=True)
        self.thread.start()

    def snapshot(self):
        with self.lock:
            return list(self.records)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        if self.thread.is_alive():
            raise TimeoutError("tracker fixture did not stop")
