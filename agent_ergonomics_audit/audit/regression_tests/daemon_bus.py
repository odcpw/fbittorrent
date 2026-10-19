"""P1-08 real subscription cases; no service outside daemon.Sandbox is used."""
import json
import os
from pathlib import Path
import socket
import threading
import time

import daemon as d
import fixtures
from fbtlog import REPO


def fixture(client, sandbox, *, stopped=False, name="bus-fixture"):
    source = sandbox.root / name
    source.mkdir()
    payload = source / (name + ".bin")
    payload.write_bytes(bytes(range(256)) * 64)
    torrent = fixtures.torrent_fixture(source, [payload], name=name)
    result = client.call("torrents.add", dict(source=dict(path=str(torrent.torrent)),
                         save_path=str(sandbox.downloads), stopped=stopped))
    if not result.get("ok"):
        raise RuntimeError("fixture add failed")
    def ready():
        row = client.call("torrents.get", dict(hash=torrent.info_hash))["result"]["torrent"]
        return row["state"] not in ("checking", "metadata"), json.dumps(row, sort_keys=True)
    if not client.harness.wait_for(ready, "bus fixture has a stable real engine state"):
        raise RuntimeError("fixture never ready")
    return torrent


class RevisionStream:
    """The same strict consecutive-revision rule as the frozen Rust client."""
    def __init__(self, client):
        self.client = client
        self.revisions, self.open = {}, set()
        self.frames = []

    def receive(self):
        frame = self.client.pushed.pop(0) if self.client.pushed else self.client.receive()
        if frame is None:
            raise EOFError("subscription closed")
        self.frames.append(frame)
        if "rev" in frame:
            sub, rev = frame["sub"], frame["rev"]
            expected = self.revisions.get(sub, 0) + 1
            if rev != expected:
                raise ValueError(f"subscription {sub} gap: expected {expected}, observed {rev}")
            self.revisions[sub] = rev
            if frame["ev"] == "snapshot.begin":
                if sub in self.open:
                    raise ValueError("nested subscription snapshot")
                self.open.add(sub)
            elif frame["ev"] == "snapshot.end":
                self.open.remove(sub)
            elif frame["ev"] in ("state", "delta") and sub in self.open:
                raise ValueError("live update interleaved with snapshot")
        return frame

    def snapshot(self, sub, topics):
        remaining = set(topics)
        frames = []
        while remaining:
            frame = self.receive()
            frames.append(frame)
            if frame.get("sub") == sub and frame["ev"] in ("state", "snapshot.end"):
                remaining.discard(frame["topic"])
        return frames

    def until(self, predicate, seconds=10):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            frame = self.receive()
            if predicate(frame):
                return frame
        raise TimeoutError("subscription observation deadline")


def bus_subscriptions(binary, systemd):
    with d.api_owned("bus_subscriptions", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as client:
        frames = []
        client.trace = frames
        hello = client.hello()["result"]
        case.check("subscribe" in hello["caps"], "subscribe is negotiated", str(hello["caps"]))
        item = fixture(client, sandbox, stopped=True)
        client.trace = None
        client.call("subscribe", dict(topics=["unknown"]), error="invalid_args")
        missing = client.call("subscribe", dict(topics=["torrent:" + "f" * 40 + ".files"]), error="not_found")
        case.check(missing["error"]["details"]["hash"] == "f" * 40, "unknown detail names its hash", str(missing))
        for interval in (249, 10001):
            normalized = client.call("subscribe", dict(topics=["torrents"], interval_ms=interval))["result"]["sub"]
            client.call("unsubscribe", dict(sub=normalized))
        client.pushed.clear()
        client.trace = frames
        topics = ["summary", "network", "settings", "torrents", f"torrent:{item.info_hash}.files", "events"]
        sub = client.call("subscribe", dict(topics=topics))["result"]["sub"]
        stream = RevisionStream(client)
        actual = stream.snapshot(sub, topics)
        case.check(len(stream.revisions) == 1 and not stream.open,
                   "one subscription has consecutive revisions across all six topics and atomic snapshots", str(actual))
        full = [value for frame in actual for value in frame.get("items", [])]
        def no_null(value):
            return value is not None and (all(no_null(v) for v in value.values()) if isinstance(value, dict) else all(no_null(v) for v in value) if isinstance(value, list) else True)
        case.check(all(no_null(v) for v in full), "full objects omit unknown fields", str(full))
        client.call("unsubscribe", dict(sub=sub))
        client.call("app.info")
        # The captured live transcript uses the frozen typed validator, rather
        # than manufacturing frames from fixture expectations.
        if os.environ.get("FBT_API_CONTRACT_VALIDATOR"):
            d.validate_live_contract(case, sandbox, frames)


def structural_frame_within_100ms(binary, systemd):
    with d.api_owned("structural_frame_within_100ms", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as client, d.ApiConn(harness) as writer:
        client.hello(); writer.hello()
        item = fixture(writer, sandbox)
        sub = client.call("subscribe", dict(topics=["torrents"], interval_ms=10000))["result"]["sub"]
        stream = RevisionStream(client); stream.snapshot(sub, ["torrents"])
        started = time.monotonic()
        identifier = writer.send("torrents.stop", dict(hashes=[item.info_hash]))
        actual = stream.until(lambda f: f.get("ev") == "delta" and f.get("changed", {}).get(item.info_hash, {}).get("state") == "paused")
        elapsed = (time.monotonic() - started) * 1000
        response = writer.response(identifier)
        case.check(response.get("ok") is True and elapsed <= 100,
                   "pause state delta arrives within 100 ms despite 10000 ms rate interval",
                   json.dumps(dict(ms=elapsed, response=response, delta=actual), sort_keys=True))


def detail_topic_released(binary, systemd):
    with d.api_owned("detail_topic_released", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as client, d.ApiConn(harness) as observer:
        client.hello(); observer.hello()
        item = fixture(observer, sandbox, stopped=True)
        topic = f"torrent:{item.info_hash}.files"
        sub = client.call("subscribe", dict(topics=[topic]))["result"]["sub"]
        RevisionStream(client).snapshot(sub, [topic])
        info = observer.call("app.info")["result"]["subscriptions"]
        case.check(info == dict(total=1, by_topic={topic: 1}), "detail sampling has one owner", str(info))
        client.call("unsubscribe", dict(sub=sub))
        info = observer.call("app.info")["result"]["subscriptions"]
        case.check(info == dict(total=0, by_topic={}), "unsubscribe releases the last detail sampler", str(info))
        client.call("subscribe", dict(topics=[topic]))
        client.close()
        def released():
            value = observer.call("app.info")["result"]["subscriptions"]
            return value == dict(total=0, by_topic={}), json.dumps(value)
        harness.wait_for(released, "connection drop also releases detail sampling")


def engine_restart_keeps_connection(binary, systemd):
    with d.api_owned("engine_restart_keeps_connection", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as client, d.ApiConn(harness) as writer:
        identity = client.hello()["result"]; writer.hello()
        item = fixture(writer, sandbox, stopped=True)
        topics = ["torrents", "summary", f"torrent:{item.info_hash}.files"]
        sub = client.call("subscribe", dict(topics=topics))["result"]["sub"]
        stream = RevisionStream(client); stream.snapshot(sub, topics)
        writer.call("app.restart", dict(scope="engine"))
        event = stream.until(lambda f: f.get("ev") == "engine.changed")
        seen, resets = set(), []
        while seen != set(topics):
            frame = stream.receive()
            if frame.get("ev") == "state" and frame.get("topic") == "summary":
                seen.add("summary")
            if frame.get("ev") == "snapshot.begin" and frame.get("reason") == "engine_restart":
                resets.append(frame)
            if frame.get("ev") == "snapshot.end" and any(r["topic"] == frame["topic"] for r in resets):
                seen.add(frame["topic"])
        info = client.call("app.info")["result"]
        case.check(info["instance"] == identity["instance"] and info["engine_epoch"] > identity["engine_epoch"] and not stream.open,
                   "restart keeps the connection and owner identity, advances epoch and refreshes every subscribed collection",
                   str(dict(info=info, event=event, reset_topics=sorted(seen), snapshots=resets)))
        writer.call("torrents.remove", dict(hashes=[item.info_hash], delete_files=False))
        writer.call("app.restart", dict(scope="engine"))
        detail = f"torrent:{item.info_hash}.files"
        empty = stream.until(lambda f: f.get("ev") == "snapshot.begin" and f.get("reason") == "engine_restart" and f.get("topic") == detail and f.get("count") == 0)
        stream.until(lambda f: f.get("ev") == "snapshot.end" and f.get("topic") == detail)
        client.call("unsubscribe", dict(sub=sub))
        info = client.call("app.info")["result"]
        case.check(info["subscriptions"] == dict(total=0, by_topic={}),
                   "a detail missing after restart receives an empty snapshot and can close without dropping its connection",
                   str(dict(snapshot=empty, subscriptions=info["subscriptions"])))


def slow_client_closed_others_live(binary, systemd):
    with d.api_owned("slow_client_closed_others_live", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as slow, d.ApiConn(harness) as healthy, d.ApiConn(harness) as writer:
        slow_name = "slow\nprobe"
        slow.call("hello", dict(proto=1, client=dict(name=slow_name, version="0.2.0", kind="test")))
        healthy.hello(); writer.hello()
        item = fixture(writer, sandbox)
        slow.socket.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
        sub = slow.call("subscribe", dict(topics=["torrents"]))["result"]["sub"]
        RevisionStream(slow).snapshot(sub, ["torrents"])
        sub = healthy.call("subscribe", dict(topics=["torrents"], interval_ms=10000))["result"]["sub"]
        live = RevisionStream(healthy); live.snapshot(sub, ["torrents"])
        # Stop reading completely; pipelined acknowledgements fill the same
        # bounded connection outbox used by snapshots and pushed deltas.
        payload = b"".join(json.dumps(dict(id=10000+i, op="app.client", args=dict(glance=False))).encode()+b"\n" for i in range(6000))
        send_result = []
        def flood():
            try: slow.socket.sendall(payload)
            except OSError as error: send_result.append(str(error))
        thread = threading.Thread(target=flood); thread.start()
        def overflow():
            text = case.daemon_path.read_text(errors="replace") if case.daemon_path.exists() else ""
            # The daemon log is finalized by Case; inspect the currently open
            # per-suite stderr file during the case, then retain it at final.
            text += case.suite.stderr_path.read_text(errors="replace")
            lines = text.splitlines()
            matched = any("WARN api slow client" in line and "queued=512" in line
                          and "name=" + json.dumps(slow_name) in line for line in lines)
            return matched, text[-1000:]
        harness.wait_for(overflow, "non-reading connection reaches the 512-frame bound", timeout=10)
        writer.call("torrents.stop", dict(hashes=[item.info_hash]))
        actual = live.until(lambda f: f.get("changed", {}).get(item.info_hash, {}).get("state") == "paused")
        case.check(actual["ev"] == "delta", "healthy client keeps receiving after slow client overflows", str(actual))
        slow.socket.settimeout(3)
        closed = False; terminal = None
        try:
            while (frame := slow.receive()) is not None:
                if frame.get("ev") == "bye": terminal = frame
            closed = True
        except (ConnectionResetError, BrokenPipeError): closed = True
        thread.join(timeout=4)
        case.check(closed and not thread.is_alive(), "slow client is closed and its sender terminates", str(dict(closed=closed, bye=terminal, send=send_result)))


def rev_soak_10min(binary, systemd):
    if os.environ.get("FBT_TEST_SOAK") != "1":
        return
    with d.api_owned("rev_soak_10min", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness, timeout=20) as client, d.ApiConn(harness) as writer:
        client.hello(); writer.hello()
        item = fixture(writer, sandbox, stopped=True)
        topics = ["torrents", "summary", "events", f"torrent:{item.info_hash}.files"]
        sub = client.call("subscribe", dict(topics=topics, interval_ms=250))["result"]["sub"]
        stream = RevisionStream(client); stream.snapshot(sub, topics)
        started = time.monotonic(); stopped = True; next_change = started
        sample = case.step("soak subscription for ten minutes", "every incoming revision is exactly its predecessor plus one, without interleaved snapshots")
        artifact = case.path.with_suffix(".frames.ndjson")
        with artifact.open("w") as output:
            while time.monotonic() - started < 600:
                if time.monotonic() >= next_change:
                    writer.call("torrents.start" if stopped else "torrents.stop", dict(hashes=[item.info_hash]))
                    stopped = not stopped; next_change += 2
                frame = stream.receive()
                output.write(json.dumps(frame, sort_keys=True) + "\n")
                # Do not retain an unbounded duplicate in Python during a soak.
                stream.frames.clear()
        sample.close(not stream.open and stream.revisions[sub] > 300,
                     json.dumps(dict(seconds=time.monotonic()-started, last_rev=stream.revisions[sub], gaps=0)), [artifact])


def subscribe_gap_recovery(binary, systemd):
    with d.api_owned("subscribe_gap_recovery", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as client, d.ApiConn(harness) as writer:
        client.hello(); writer.hello()
        item = fixture(writer, sandbox, stopped=True)
        sub = client.call("subscribe", dict(topics=["torrents"], interval_ms=10000))["result"]["sub"]
        stream = RevisionStream(client); stream.snapshot(sub, ["torrents"])
        last = stream.revisions[sub]
        writer.call("torrents.start", dict(hashes=[item.info_hash]))
        discarded = stream.until(lambda f: "rev" in f and f.get("topic") == "torrents")
        writer.call("torrents.stop", dict(hashes=[item.info_hash]))
        next_frame = stream.until(lambda f: f.get("changed", {}).get(item.info_hash, {}).get("state") == "paused")
        # Lose one actual frame at the client boundary, exactly as the frozen
        # subscribe-gap fixture does; never alter the server's revisions.
        stream.revisions[sub] = last
        client.pushed.insert(0, next_frame)
        detected = False
        try: stream.receive()
        except ValueError: detected = True
        case.check(detected and next_frame["rev"] > last + 1,
                   "a missing actual frame triggers the frozen consecutive-revision rule",
                   str(dict(last=last, lost=discarded, incoming=next_frame)))
        client.call("unsubscribe", dict(sub=sub))
        replacement = client.call("subscribe", dict(topics=["torrents"]))["result"]["sub"]
        stream = RevisionStream(client)
        frames = stream.snapshot(replacement, ["torrents"])
        rows = [row for frame in frames for row in frame.get("items", [])]
        current = writer.call("torrents.get", dict(hash=item.info_hash))["result"]["torrent"]
        case.check(replacement != sub and stream.revisions[replacement] == 3 and rows[0]["state"] == current["state"] == "paused",
                   "gap recovery unsubscribes, resubscribes and replaces the store with a consistent fresh snapshot", str(frames))


def state_watcher_cancellation_stress(binary, systemd):
    with d.api_owned("state_watcher_cancellation_stress", binary, systemd) as (case, sandbox, harness, process), d.ApiConn(harness) as client, d.ApiConn(harness) as writer:
        client.hello(); writer.hello()
        item = fixture(writer, sandbox, stopped=True)
        sub = client.call("subscribe", dict(topics=["torrents"], interval_ms=10000))["result"]["sub"]
        stream = RevisionStream(client); stream.snapshot(sub, ["torrents"])
        for index in range(30):
            writer.call("torrents.start", dict(hashes=[item.info_hash]))
            stream.until(lambda f: f.get("changed", {}).get(item.info_hash, {}).get("intent") == "run")
            writer.call("torrents.stop", dict(hashes=[item.info_hash]))
            stream.until(lambda f: f.get("changed", {}).get(item.info_hash, {}).get("state") == "paused")
            stream.frames.clear()
        case.check(True, "60 ledger mutations replace and cancel watchers without starving the engine worker",
                   json.dumps(dict(cycles=30, last_rev=stream.revisions[sub], gaps=0)))
