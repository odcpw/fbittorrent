"""Unreadable local files stay local to their consumer.

A typo in config.toml keeps the last valid settings; a corrupt shows.json or
rss.json fails only its own topic. One real daemon, private XDG, one shared
dashboard-like subscription that must stay connected and live throughout.
"""
import json
from pathlib import Path
import time

import daemon as d
from daemon_bus import RevisionStream, fixture

TOPICS = ["summary", "torrents", "settings", "shows", "rss.feeds"]
BROKEN = {"config": "max_active_downloads = [\n", "shows": "{ not json", "rss": "{ not json"}
WARNING = {"config": "WARN daemon settings unreadable", "shows": "WARN shows store unreadable",
           "rss": "WARN rss store unreadable"}


def store(box, source):
    if source == "config":
        return Path(box.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
    return Path(box.env["XDG_DATA_HOME"]) / "fbittorrent" / (source + ".json")


def warned(frame, source):
    return (frame.get("ev") == "state" and frame.get("topic") == "summary"
            and any(w.get("source") == source for w in frame["data"]["daemon"].get("warnings", [])))


def unreadable_store_keeps_clients(name, source, binary, systemd):
    with d.api_owned(name, binary, systemd) as (case, box, h, process), \
            d.ApiConn(h, timeout=10) as client, d.ApiConn(h) as writer:
        client.hello(); writer.hello()
        sub = client.call("subscribe", dict(topics=TOPICS, interval_ms=250))["result"]["sub"]
        stream = RevisionStream(client)
        stream.snapshot(sub, TOPICS)
        item = fixture(writer, box, stopped=True, name="robust-" + source)
        stream.until(lambda f: f.get("topic") == "torrents"
                     and any(row.get("hash") == item.info_hash for row in f.get("added", []) + f.get("items", [])), seconds=10)
        settings = next(f["data"] for f in stream.frames if f.get("topic") == "settings" and f.get("ev") == "state")
        path = store(box, source)
        path.parent.mkdir(parents=True, exist_ok=True)
        original = path.read_bytes() if path.exists() else None
        offset = len(h.stderr())
        # The same atomic replacement an editor or the CLI performs.
        staged = path.with_name(path.name + ".broken")
        staged.write_text((original or b"").decode() + BROKEN[source] if source == "config" else BROKEN[source])
        staged.rename(path)
        report = stream.until(lambda f: warned(f, source), seconds=10)
        case.check(report["sub"] == sub, f"summary reports the unreadable {source} file on the open subscription",
                   json.dumps(report["data"]["daemon"]))
        # Torrents stays live: a real removal arrives as a structural delta.
        writer.call("torrents.remove", dict(hashes=[item.info_hash], delete_files=False))
        removed = stream.until(lambda f: f.get("topic") == "torrents" and item.info_hash in f.get("removed", []), seconds=10)
        case.check(removed["sub"] == sub, "torrents keeps delivering structural changes while the file is broken",
                   json.dumps(removed)[:600])
        # Three more seconds of sampling: the same connection still answers and
        # has received no subscription error or close.
        time.sleep(3)
        alive = client.call("app.info")
        errors = [f for f in client.pushed if f.get("ev") in ("error", "bye")]
        case.check(alive.get("ok") is True and alive["result"]["subscriptions"]["total"] == 1 and not errors,
                   "the shared connection stays open and subscribed; no subscription error or close",
                   json.dumps(dict(subscriptions=alive.get("result", {}).get("subscriptions"), errors=errors)))
        # A new subscription naming the broken topic is accepted, not refused.
        topic = {"config": "settings", "shows": "shows", "rss": "rss.feeds"}[source]
        second = client.call("subscribe", dict(topics=["summary", topic]))
        case.check(second.get("ok") is True, f"subscribing to {topic} succeeds while its file is unreadable",
                   json.dumps(second))
        if source == "config":
            state = stream.until(lambda f: f.get("sub") == second["result"]["sub"] and f.get("topic") == "settings", seconds=10)
            case.check(state["data"]["max_active_downloads"] == settings["max_active_downloads"]
                       and state["data"].get("network_interface") == settings.get("network_interface"),
                       "settings keep the last valid values instead of failing", json.dumps(state["data"]))
        lines = [line for line in h.stderr()[offset:].splitlines() if WARNING[source] in line]
        case.check(len(lines) == 1, "the unreadable file is logged once, not on every sample", json.dumps(lines))
        # Repair: the warning clears and the original subscription is unchanged.
        if original is None:
            path.unlink()
        else:
            staged.write_bytes(original); staged.rename(path)
        cleared = stream.until(lambda f: f.get("ev") == "state" and f.get("topic") == "summary" and f.get("sub") == sub
                               and not any(w.get("source") == source for w in f["data"]["daemon"].get("warnings", [])), seconds=10)
        info = client.call("app.info")["result"]["subscriptions"]
        case.check(info["total"] == 2, "repair clears the warning; both subscriptions remain",
                   json.dumps(dict(daemon=cleared["data"]["daemon"], subscriptions=info)))


def corrupt_config_keeps_clients(binary, systemd):
    unreadable_store_keeps_clients("corrupt_config_keeps_clients", "config", binary, systemd)


def corrupt_shows_keeps_clients(binary, systemd):
    unreadable_store_keeps_clients("corrupt_shows_keeps_clients", "shows", binary, systemd)


def corrupt_rss_keeps_clients(binary, systemd):
    unreadable_store_keeps_clients("corrupt_rss_keeps_clients", "rss", binary, systemd)


CASES = [corrupt_config_keeps_clients, corrupt_shows_keeps_clients, corrupt_rss_keeps_clients]
