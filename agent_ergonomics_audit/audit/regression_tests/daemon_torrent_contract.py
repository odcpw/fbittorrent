"""Live torrent contract scenarios; the static field catalogue is not a timeline.

The catalogue deliberately covers mutually exclusive engine states and arbitrary
optional values. Keep validating it unchanged, and validate every original frame
from these real transfers separately. Never substitute catalogue values into a
live frame, manufacture ETA, or accept completion as a receiving sample.
"""
import json
import math
import os
from pathlib import Path
import shutil

import daemon as d
import fixtures


def run(binary, systemd):
    frames, states = [], set()
    with d.replay_owned("torrents", binary, systemd) as (case, box, h, process, root):
        catalogue = [json.loads(line) for line in
                     (d.REPO / "contracts/ipc/v1/torrents.ndjson").read_text().splitlines()]
        d.validate_live_contract(case, box, catalogue, "fixture")
        save = root / "downloads"
        save.mkdir(exist_ok=True)
        try:
            with d.ApiConn(h, trace=frames) as client:
                client.hello()
                client.call("app.client", dict(interactive=True, glance=False))

                def call(op, args=None, **kw):
                    return client.call(op, args, **kw)["result"]

                def get(hash_):
                    return call("torrents.get", dict(hash=hash_))["torrent"]

                def wait(hash_, predicate, label):
                    observed = [None]
                    def sample():
                        observed[0] = get(hash_)
                        return predicate(observed[0]), json.dumps(observed[0])
                    if not h.wait_for(sample, label):
                        raise RuntimeError(label)
                    states.add(observed[0]["state"])
                    return observed[0]

                def state(hash_, expected):
                    return wait(hash_, lambda row: row["state"] == expected,
                                "real torrent reaches " + expected)

                def fixture(name, sizes, progress=0, tracker=False, piece=65536):
                    directory = root / name
                    directory.mkdir()
                    paths = []
                    for index, size in enumerate(sizes):
                        path = directory / (name + str(index + 1) + ".bin")
                        path.write_bytes(os.urandom(size))
                        paths.append(path)
                    value = fixtures.torrent_fixture(directory, paths, name=name,
                        tracker=h.replay_network["tracker"] if tracker else None,
                        piece_length=piece)
                    for path in paths:
                        destination = save / path.name
                        shutil.copyfile(path, destination)
                        if progress < 1:
                            with destination.open("r+b") as output:
                                for offset in range(int(path.stat().st_size * progress),
                                                    path.stat().st_size, piece):
                                    output.seek(offset)
                                    byte = output.read(1)
                                    output.seek(offset)
                                    output.write(bytes([byte[0] ^ 1]))
                    return dict(fixture=value, hash=value.info_hash, pristine=directory)

                def add(item, **kw):
                    reply = call("torrents.add", dict(source=dict(path=str(item["fixture"].torrent)),
                                 save_path=str(save), origin="manual", **kw))
                    case.check(reply["hash"] == item["hash"] and reply["duplicate"] is False,
                               "add returns the real metainfo identity", json.dumps(reply))
                    return reply

                def exact(value, expected, label):
                    case.check(value == expected, label, json.dumps(value))

                def receiving(row):
                    return (row["state"] == "downloading" and row.get("dl_bps", 0) > 0
                            and 0 < row.get("completed_bytes", 0) < row.get("size", 0))

                def counters(row, label):
                    size, done, rate = row["size"], row["completed_bytes"], row["dl_bps"]
                    case.check(receiving(row) and row.get("eta_s") == (size - done) // rate
                               and math.isclose(row["progress"], done / size, abs_tol=1e-9),
                               label + ": real incomplete bytes, positive rate and computed ETA",
                               json.dumps(row))

                # Eight MiB cannot finish between the sampled reads at 32 KiB/s.
                # The original 4 KiB catalogue payload finished before its first
                # rate sample and correctly rejected the subsequent start.
                item = fixture("Receiving", [4 * fixtures.MIB] * 2, .25, tracker=True)
                digest = item["hash"]
                add(item, stopped=True)
                wait(digest, lambda row: row["checked"], "initial paused payload is checked")
                d.start_replay_seed(client, h, item)
                exact(call("torrents.start", dict(hashes=[digest])),
                      dict(state="done", started=[digest], skipped=[]), "start changes paused intent")
                detail = wait(digest, receiving, "8 MiB loopback transfer is receiving, not complete")
                counters(detail, "detail")
                listed = call("torrents.list", dict(origin=["manual"]))["torrents"]
                case.check(len(listed) == 1 and listed[0]["hash"] == digest,
                           "list contains exactly the owned receiving torrent", json.dumps(listed))
                counters(listed[0], "list")
                duplicate = call("torrents.add", dict(source=dict(magnet=item["fixture"].magnet)))
                case.check(duplicate["duplicate"] and duplicate["hash"] == digest,
                           "duplicate add retains the running torrent", json.dumps(duplicate))
                exact(call("torrents.start", dict(hashes=[digest])),
                      dict(state="done", started=[], skipped=[dict(hash=digest, reason="already_running")]),
                      "an incomplete running torrent is skipped as already_running")
                exact(call("torrents.stop", dict(hashes=[digest])),
                      dict(state="done", stopped=[digest]), "stop returns the stopped identity")
                paused = state(digest, "paused")
                case.check(paused["intent"] == "pause" and "eta_s" not in paused,
                           "paused torrent omits ETA", json.dumps(paused))
                files = call("torrents.files", dict(hash=digest))["files"]
                case.check([f["size"] for f in files] == [4 * fixtures.MIB] * 2
                           and all(f["included"] for f in files)
                           and abs(sum(f["size"] * f["progress"] for f in files) - paused["completed_bytes"]) < 1,
                           "file geometry and progress agree with verified torrent bytes", json.dumps(files))
                removed = call("torrents.remove", dict(hashes=[digest], delete_files=False))
                case.check(removed["state"] == "done" and "undo_until" in removed
                           and all((save / p.name).exists() for p in item["fixture"].files),
                           "keep-files remove preserves payload and offers undo", json.dumps(removed))
                restored = call("torrents.add", dict(source=dict(restore=digest)))
                case.check(restored["hash"] == digest and not restored["duplicate"],
                           "undo restores the same identity", json.dumps(restored))
                wait(digest, lambda row: row["checked"], "restored payload is checked")
                for include in ([0], [0, 1]):
                    exact(call("torrents.set_files", dict(hash=digest, include=include)),
                          dict(state="done"), "file selection is acknowledged")
                for on in (True, False):
                    exact(call("torrents.set_seed", dict(hashes=[digest], on=on)),
                          dict(state="done"), "seed intent is acknowledged")
                for update in (dict(bps=None), {}, dict(bps=32768)):
                    exact(call("torrents.set_upload_limit", dict(hashes=[digest], **update)),
                          dict(state="done"), "upload policy update is acknowledged")
                target = call("torrents.open_target", dict(hash=digest, file=0))
                exact(target, dict(kind="file", path=str(save / item["fixture"].files[0].name)),
                      "open_target resolves the actual owned file")
                folder = call("torrents.open_target", dict(hash=digest, folder=True))
                case.check(folder.get("kind") == "folder" and Path(folder["path"]).is_dir()
                           and Path(target["path"]).is_relative_to(Path(folder["path"]))
                           and "stream_url" not in folder,
                           "explicit folder target contains the selected file and creates no media stream", json.dumps(folder))
                call("torrents.stop", dict(hashes=[digest]))

                stalled = fixture("Stalled", [4096], .5, piece=1024)
                add(stalled)
                state(stalled["hash"], "stalled")
                queued = fixture("Queued", [4096], .5, piece=1024)
                add(queued)
                value = state(queued["hash"], "queued")
                case.check(value["reason"] == "queue_full" and value["intent"] == "run",
                           "a real occupied slot queues another incomplete torrent", json.dumps(value))
                call("torrents.stop", dict(hashes=[stalled["hash"], queued["hash"]]))

                magnet = "magnet:?xt=urn:btih:" + "17" * 20 + "&dn=Unresolved"
                unresolved = call("torrents.add", dict(source=dict(magnet=magnet)))
                state(unresolved["hash"], "metadata")

                checking = fixture("Checking", [64 * fixtures.MIB], .5, piece=1024)
                add(checking)
                value = state(checking["hash"], "checking")
                case.check(not value["checked"] and 0 <= value["completed_bytes"] <= value["size"],
                           "a real checksum pass reports unfinished verification", json.dumps(value))
                call("torrents.stop", dict(hashes=[checking["hash"]]))

                complete = fixture("Complete", [4096], 1, piece=1024)
                add(complete, seed=True)
                state(complete["hash"], "seeding")
                call("torrents.set_seed", dict(hashes=[complete["hash"]], on=False))
                state(complete["hash"], "done")
                failure = client.call("torrents.start", dict(hashes=[complete["hash"]]), error="invalid_state")
                case.check(failure["error"]["details"]["reason"] == "complete_no_seed",
                           "a completed non-seeding torrent is not falsely already_running", json.dumps(failure))

                # VPN parking is session-wide, so observe it in its own epoch,
                # never beside a downloading row in a fabricated list snapshot.
                config = Path(box.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
                saved = config.read_text()
                call("torrents.start", dict(hashes=[stalled["hash"]]))
                d.select_replay_interface(config, "fbmissing0")
                call("app.restart", dict(scope="engine"))
                state(stalled["hash"], "waiting_vpn")
                config.write_text(saved)
                call("app.restart", dict(scope="engine"))
                h.wait_for(lambda: ((info := call("app.info"))["state"] == "running", json.dumps(info)),
                           "real owner unparks before missing-file activation")
                d.replay_missing_payload(client, h, complete, save)
                state(complete["hash"], "error")
                case.check(states == {"downloading", "paused", "stalled", "queued", "metadata", "checking",
                                      "seeding", "done", "waiting_vpn", "error"},
                           "all ten torrent states were observed in physically possible scenarios", json.dumps(sorted(states)))
                stopping = call("app.shutdown", dict(reason="fixture end"))
                case.check(stopping["state"] == "pending" and bool(stopping.get("ref")),
                           "shutdown returns a pending acknowledgement with a reference", json.dumps(stopping))
                bye = next((event for event in client.events() if event.get("ev") == "bye"), None)
                exact(bye, dict(ev="bye", reason="shutdown"), "shutdown sends the frozen bye frame")
                process.wait()
            d.validate_live_contract(case, box, frames)
        finally:
            if process.child.poll() is None:
                h.stop_serve(process)
