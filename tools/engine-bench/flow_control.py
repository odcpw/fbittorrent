"""Actual daemon + deterministic seed, with a bounded TCP protocol test peer.

Controls reqq and choke on the wire; payload still comes from fbt-bench-seed and
is independently verified by the comparison harness. Not a timing benchmark.
The cancel case pauses with every requested response still held by the peer.
Use the usual benchmark lock/memory scope and a fresh --output directory.
"""
import argparse, contextlib, json, select, signal, socket, struct, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from head_to_head import Match, ProcessRun, decode, bencode, wait


def exact(sock, size):
    parts = bytearray()
    while len(parts) < size:
        data = sock.recv(size-len(parts))
        if not data:
            raise EOFError()
        parts.extend(data)
    return bytes(parts)


def frame(sock):
    length = struct.unpack("!I", exact(sock, 4))[0]
    assert length <= 1024**2
    return exact(sock, length) if length else b""


def send(sock, body):
    sock.sendall(struct.pack("!I", len(body))+body)


class Peer:
    def __init__(self, target, cap, mode):
        self.target, self.cap, self.mode = target, cap, mode
        self.listener = socket.create_server(("127.0.0.1", 0))
        self.listener.settimeout(.1)
        self.port = self.listener.getsockname()[1]
        self.stop = threading.Event()
        self.choked = threading.Event()
        self.errors = []
        self.peak = self.requests = self.responses = 0
        self.sockets = []
        self.stage = "initial"
        self.thread = threading.Thread(target=self.run)
        try:
            self.thread.start()
        except BaseException:
            self.listener.close()
            raise

    def run(self):
        try:
            while not self.stop.is_set():
                try:
                    client, _ = self.listener.accept()
                    break
                except socket.timeout:
                    continue
            else:
                return
            self.sockets = [client]
            seed = socket.create_connection(("127.0.0.1", self.target), timeout=3)
            self.sockets.append(seed)
            for stream in self.sockets:
                stream.settimeout(3)
                stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            seed.sendall(exact(client, 68))
            client.sendall(exact(seed, 68))
            outstanding = set()
            held = []
            last_release = time.monotonic()
            choke_at = None

            def release():
                for body in held:
                    send(client, body)
                    outstanding.discard(struct.unpack("!II", body[1:9]))
                    self.responses += 1
                held.clear()

            while not self.stop.is_set():
                now = time.monotonic()
                if self.stage == "initial" and len(held) == self.cap and now-last_release >= .15:
                    # Every earlier request has a held response. Choke precedes
                    # those responses, so no new request may refill their slots.
                    send(client, b"\0")
                    if self.mode != "cancel":
                        release()
                    self.stage = "choked"
                    choke_at = now
                    self.choked.set()
                elif self.stage == "choked" and self.mode != "cancel" and now-choke_at >= .3:
                    send(client, b"\1")
                    self.stage = "resumed"
                ready, _, _ = select.select([client, seed], [], [], .01)
                for stream in ready:
                    body = frame(stream)
                    if stream is client:
                        if body and body[0] == 6:
                            assert self.stage != "choked", "request sent after Choke"
                            outstanding.add(struct.unpack("!II", body[1:9]))
                            self.requests += 1
                            self.peak = max(self.peak, len(outstanding))
                            assert len(outstanding) <= self.cap, (len(outstanding), self.cap)
                        if body and body[0] in (1, 7):
                            raise AssertionError("upload-off client unchoked/uploaded")
                        seed.sendall(struct.pack("!I",len(body))+body)
                    else:
                        if body[:2] == b"\x14\0":
                            handshake = decode(body[2:])
                            handshake[b"reqq"] = self.cap
                            body = body[:2]+bencode(handshake)
                        if body and body[0] == 7:
                            if self.stage == "initial":
                                held.append(body)
                                assert len(held) <= self.cap
                                continue
                            outstanding.discard(struct.unpack("!II", body[1:9]))
                            self.responses += 1
                        send(client, body)
        except (EOFError, ConnectionResetError, BrokenPipeError):
            pass
        except BaseException as error:
            if not self.stop.is_set():
                self.errors.append(repr(error))
        finally:
            for stream in self.sockets:
                stream.close()

    def finish(self):
        self.stop.set()
        self.thread.join(4)
        self.listener.close()
        assert not self.thread.is_alive()
        assert not self.errors, self.errors


class Controlled(Match):
    @contextlib.contextmanager
    def setup_match(self, *args, **kw):
        with super().setup_match(*args, **kw) as torrent:
            binding = json.loads((self.sandbox.root/"seed-ready.json").read_text())
            peer = self.peer = Peer(binding["peer_ports"][0], self.args.cap, self.args.mode)
            def record():
                (self.output/"wire-control.json").write_text(json.dumps(dict(
                    cap=peer.cap, mode=peer.mode, peak=peer.peak, requests=peer.requests,
                    responses=peer.responses, stage=peer.stage, errors=peer.errors)))
            with contextlib.ExitStack() as cleanup:
                cleanup.callback(record)
                cleanup.callback(peer.finish)
                class Tracker(BaseHTTPRequestHandler):
                    def do_GET(inner):
                        body = bencode({b"interval":1800, b"peers":
                            socket.inet_aton("127.0.0.1")+struct.pack("!H",peer.port)})
                        inner.send_response(200)
                        inner.send_header("Content-Length", str(len(body)))
                        inner.end_headers()
                        inner.wfile.write(body)
                    def log_message(inner, *_):
                        pass
                tracker = ThreadingHTTPServer(("127.0.0.1",0), Tracker)
                cleanup.callback(tracker.server_close)
                thread = threading.Thread(target=tracker.serve_forever)
                thread.start()
                cleanup.callback(thread.join, 2)
                cleanup.callback(tracker.shutdown)
                meta = decode(torrent.read_bytes())
                meta[b"announce"] = f"http://127.0.0.1:{tracker.server_port}/announce".encode()
                torrent.write_bytes(bencode(meta))
                yield torrent


if __name__ == "__main__":
    def interrupted(*_):
        raise KeyboardInterrupt()
    signal.signal(signal.SIGTERM, interrupted)
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--seed", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--cases", nargs="+", choices=["choke","cancel"],
                   default=["choke","cancel"])
    args = p.parse_args()
    args.binary = args.binary.resolve(strict=True)
    args.seed = args.seed.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir()
    args.disk_root = Path("/tmp")
    args.strace = args.public_torrent = None
    args.profile = False
    args.timeout = 60
    args.qbt = Path("/usr/bin/qbittorrent-nox")
    for mode, cap in [("choke",7), ("cancel",7)]:
        if mode not in args.cases:
            continue
        args.mode, args.cap = mode, cap
        run = Controlled(args, ProcessRun())
        if mode == "cancel":
            with run.setup_match(mode, "fbt", 64*1024**2, 1, 1) as torrent:
                run.client.add(torrent, run.info_hash)
                assert run.peer.choked.wait(5)
                time.sleep(.1)
                started = time.monotonic()
                run.client.call("torrents.stop", {"hashes":[run.info_hash]})
                wait(lambda:run.client.observation()["state"] == "paused", 5)
                assert time.monotonic()-started < 5
                assert run.peer.requests == cap
                assert run.peer.responses == 0, "pause must cancel outstanding requests"
        else:
            result = run.download(mode, "fbt", 64*1024**2, 1, 1)
            result.update(instrumented=True, functional_control=True)
            (run.output/"result.json").write_text(json.dumps(result, indent=2))
            last_piece = (run.size-1)//run.info[b"piece length"]
            assert result["seed"]["first_requests"][:2] == [0,last_piece]
            assert run.peer.stage == "resumed" and run.peer.requests > cap
        assert json.loads((run.output/"teardown.json").read_text())["qualified"]
        print(mode, "PASS", flush=True)
