"""Owner robustness and cost, observed on real private daemons.

A typo in config.toml keeps the last valid settings; a corrupt shows.json or
rss.json fails only its own topic (one shared dashboard-like subscription must
stay connected and live throughout). Record edits made while parked take
effect when the engine starts; deleted payload of a live torrent is reported
as Files missing; an idle owner with dormant torrents does not reread them.
"""
import json
from pathlib import Path
import time

import daemon as d
from daemon_bus import RevisionStream, fixture
import fixtures

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


def complete_fixture(client, box, name):
    """A real torrent whose full payload already sits in the save path."""
    payload = box.downloads / (name + ".bin")
    payload.write_bytes(bytes(range(256)) * 128)
    metainfo = box.root / "metainfo"
    metainfo.mkdir(exist_ok=True)
    item = fixtures.torrent_fixture(metainfo, [payload], name=name)
    client.call("torrents.add", dict(source=dict(path=str(item.torrent)), save_path=str(box.downloads)))
    return item, payload


def row(client, hash_):
    return client.call("torrents.get", dict(hash=hash_))["result"]["torrent"]


def parked_record_edits_apply_on_start(binary, systemd):
    with d.api_owned("parked_record_edits_apply_on_start", binary, systemd) as (case, box, h, process), \
            d.ApiConn(h) as client:
        client.hello()
        item, _ = complete_fixture(client, box, "parked-edits")
        h.wait_for(lambda: ((r := row(client, item.info_hash))["state"] == "done" and r.get("completed_at", 0) > 0, json.dumps(r)),
                   "fixture completes its real check and saves its completion")
        client.call("app.settings.set", dict(network_interface="fbtqmissing"))
        h.wait_for(lambda: ((i := client.call("app.info")["result"])["state"] == "parked", json.dumps(i)),
                   "park the engine on a deliberately absent private-test interface")
        one = dict(hashes=[item.info_hash])
        client.call("torrents.set_upload_limit", dict(one, bps=49152))
        client.call("torrents.set_download_limit", dict(one, bps=65536))
        client.call("torrents.set_seed", dict(one, on=True))
        parked = row(client, item.info_hash)
        case.check(parked["state"] == "waiting_vpn" and parked.get("upload_limit_bps") == 49152
                   and parked.get("download_limit_bps") == 65536 and parked["seed"],
                   "caps and upload permission apply to the saved record while parked", json.dumps(parked))
        refused = client.call("torrents.move", dict(one, save_path=str(box.root / "elsewhere")), error="engine_unavailable")
        case.check(refused["error"]["details"]["state"] == "parked",
                   "moving payload still needs the engine and says it is parked", json.dumps(refused))
        client.call("app.settings.set", dict(network_interface="lo"))
        h.wait_for(lambda: ((i := client.call("app.info")["result"])["state"] == "running", json.dumps(i)),
                   "the engine starts when the interface is back")
        def applied():
            r = row(client, item.info_hash)
            return (r["state"] == "seeding" and r.get("upload_limit_bps") == 49152
                    and r.get("download_limit_bps") == 65536 and r["seed"] and r["upload_enabled"]), json.dumps(r)
        h.wait_for(applied, "the started engine seeds with the caps saved while parked", timeout=20)


def live_files_missing_reported(binary, systemd):
    with d.api_owned("live_files_missing_reported", binary, systemd) as (case, box, h, process), \
            d.ApiConn(h) as client:
        client.hello()
        item, payload = complete_fixture(client, box, "live-missing")
        h.wait_for(lambda: ((r := row(client, item.info_hash))["state"] == "done" and r.get("completed_at", 0) > 0, json.dumps(r)),
                   "fixture completes its real check and saves its completion")
        before = row(client, item.info_hash)
        payload.unlink()
        missing = row(client, item.info_hash)
        case.check(missing["state"] == "error" and missing.get("reason") == "missing_files"
                   and missing.get("error", "").startswith("Files missing:") and "progress" not in missing
                   and not payload.exists(),
                   "a live torrent's deleted payload is Files missing on the next explicit read, and is not recreated",
                   json.dumps(dict(before=before, after=missing)))
        payload.write_bytes(bytes(range(256)) * 128)
        restored = row(client, item.info_hash)
        case.check(restored["state"] == before["state"] and restored.get("reason") == before.get("reason"),
                   "restoring the file clears the report", json.dumps(restored))


def proc_io(pid):
    fields = dict(line.split(": ") for line in Path(f"/proc/{pid}/io").read_text().splitlines())
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return int(fields["syscr"]), int(fields["rchar"]), int(stat[11]) + int(stat[12])


def idle_owner_reads_no_metainfo(binary, systemd):
    with d.api_owned("idle_owner_reads_no_metainfo", binary, systemd) as (case, box, h, process):
        with d.ApiConn(h) as client:
            client.hello()
            items = [complete_fixture(client, box, f"idle-{index}")[0] for index in range(8)]
            def completed():
                rows = [row(client, i.info_hash) for i in items]
                return all(r["state"] == "done" and r.get("completed_at", 0) > 0 for r in rows), json.dumps(rows)
            h.wait_for(completed, "eight real fixtures complete and save their completion", timeout=30)
            epoch = client.call("app.info")["result"]["engine_epoch"]
            client.call("app.restart", dict(scope="engine"))
            h.wait_for(lambda: ((i := client.call("app.info")["result"])["engine_epoch"] > epoch and i["state"] == "running",
                                json.dumps(i)), "engine restarts; completed records are dormant")
            dormant = [row(client, i.info_hash) for i in items]
            case.check(all(r["state"] == "done" and not r["checked"] for r in dormant), "eight dormant rows", json.dumps(dormant))
        pid = h.daemon_pid()
        time.sleep(2)
        windows = {}
        for label, topics in (("no subscriber", None), ("bar summary", ["summary"])):
            connection = None
            if topics:
                connection = d.ApiConn(h, timeout=20); connection.hello(kind="bar")
                connection.call("subscribe", dict(topics=topics, interval_ms=5000))
                time.sleep(1)
            before = proc_io(pid); time.sleep(10); after = proc_io(pid)
            windows[label] = dict(read_syscalls=after[0] - before[0], read_bytes=after[1] - before[1], cpu_ticks=after[2] - before[2])
            if connection:
                connection.close()
        # Each dormant row's .torrent used to be reread (and parsed twice) on
        # every one-second sample: ~25 KB and ~470 read syscalls per 10 s for
        # these eight rows. What remains is runtime wakeup reads (~2 KB).
        case.check(all(w["read_bytes"] <= 8192 for w in windows.values()),
                   "an idle owner with eight dormant torrents rereads no saved metainfo (<= 8 KiB read per 10 s)",
                   json.dumps(windows))


CASES = [corrupt_config_keeps_clients, corrupt_shows_keeps_clients, corrupt_rss_keeps_clients,
         parked_record_edits_apply_on_start, live_files_missing_reported, idle_owner_reads_no_metainfo]
