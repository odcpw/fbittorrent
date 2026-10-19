#!/usr/bin/env python3
"""Exercise app VPN loss/reconnect in an owned Linux network namespace.

The parent captures its network namespace, then unshare creates a private user
and network namespace. All interface mutations and all XDG files belong to
that fixture. The user's VPN, files and running app are untouched.
"""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
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
    """Current daemon + attached TUI; every device exists only in this netns."""
    import daemon as d
    import fixtures
    import native
    from fbtlog import Case,suite_status

    assert os.readlink("/proc/self/ns/net") != os.environ["FBT_TEST_HOST_NETNS"]
    def ip(*args):subprocess.run(["ip",*args],check=True,capture_output=True)
    ip("link","set","lo","up")
    assert [row["ifname"] for row in json.loads(subprocess.check_output(["ip","-j","link","show"]))]==["lo"]
    with Case('vpn','loss_reconnect') as case,d.Sandbox() as box,d.Harness(case,binary,box,False) as h:
        config=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('network_interface = "lo"','network_interface = "proton"')+
                          'max_active_downloads = 1\n')
        before=config.read_bytes()
        process=h.start_serve(idle_exit_secs=0)
        assert h.receive_ready() and h.wait_engine()
        ui=None
        try:
            with d.ApiConn(h) as client:
                client.hello();expected={}
                for number,paused in [(1,False),(2,False),(3,True)]:
                    payload=box.root/f'vpn-fixture-{number}.bin';payload.write_bytes(b'fixture!')
                    item=fixtures.torrent_fixture(box.root,[payload],name=f'vpn-{number}')
                    result=client.call('torrents.add',dict(source=dict(path=str(item.torrent)),stopped=paused))['result']
                    case.check(result['deferred'],'absent VPN durably accepts metadata without starting transfer',json.dumps(result))
                    expected[item.info_hash]=paused
                ledger=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json'
                native.prepare_imdb_fixture(box.env,[f'vpn-fixture-{n}.bin' for n in range(1,4)])
                ui=native.AttachedTui(case,str(binary),box,h,session='fbt-m1-vpn');target,done=ui.start()
                ui.assert_pane(target,'vpn-fixture-')
                inode=h.engine_path.stat().st_ino;pid=h.daemon_pid()

                def sockets():
                    owned=set()
                    for fd in (Path('/proc')/str(pid)/'fd').iterdir():
                        try:
                            link=os.readlink(fd)
                            if link.startswith('socket:['):owned.add(link[8:-1])
                        except FileNotFoundError:pass
                    inet=set()
                    for protocol in ('tcp','tcp6','udp','udp6'):
                        for line in (Path('/proc')/str(pid)/'net'/protocol).read_text().splitlines()[1:]:
                            parts=line.split()
                            if len(parts)>9:inet.add(parts[9])
                    return owned & inet

                def preserved():
                    assert config.read_bytes()==before,'VPN transitions rewrote settings'
                    rows=json.loads(ledger.read_text())['records']
                    assert {row['hash']:row['paused'] for row in rows}==expected,'saved pause/queue intent changed'
                    assert h.engine_path.stat().st_ino==inode and h.daemon_pid()==pid,'VPN transition replaced the daemon listener/owner'

                def settle(label,ready,index=None,previous=None):
                    observed_sockets=set()
                    def observed():
                        nonlocal observed_sockets
                        network=client.call('app.network')['result']['state']
                        rows=client.call('torrents.list',{})['result']['torrents']
                        observed_sockets=sockets();preserved()
                        valid=network['ready'] is ready and len(rows)==3
                        valid=valid and all(row['intent']==('pause' if expected[row['hash']] else 'run') for row in rows)
                        if ready:
                            valid=valid and bool(observed_sockets) and (index is None or socket.if_nametoindex(network['interface'])==index)
                            if previous is not None:valid=valid and observed_sockets.isdisjoint(previous)
                        else:
                            valid=valid and not observed_sockets and all(row['state']=='waiting_vpn' and 'progress' not in row for row in rows)
                        return valid,json.dumps(dict(network=network,rows=rows,sockets=sorted(observed_sockets)))
                    assert h.wait_for(observed,label,timeout=10)
                    print('PASS '+label,flush=True)
                    return observed_sockets

                settle('initial absent Proton',False)
                ip('link','add','proton0','type','dummy');settle('Proton device down',False)
                ip('address','add','198.18.0.1/32','dev','proton0');ip('link','set','proton0','up')
                first=socket.if_nametoindex('proton0');old=settle('Proton becomes available',True,first)
                ip('link','set','proton0','down');settle('live Proton goes down',False)
                ip('link','set','proton0','up');old=settle('same Proton recovers',True,first,old)
                ip('link','delete','proton0');settle('live Proton disappears',False)
                for label in ['same-name replacement starts new session','rapid replacement also drains old session']:
                    ip('link','add','proton0','type','dummy');ip('address','add','198.18.0.1/32','dev','proton0');ip('link','set','proton0','up')
                    replacement=socket.if_nametoindex('proton0');assert replacement!=first
                    old=settle(label,True,replacement,old);first=replacement
                    if label.startswith('same-name'):ip('link','delete','proton0')
                ui.keys(target,'q');assert h.wait_for(lambda:(done.exists() and done.read_text()=='0',ui.capture(target)),'q detaches without stopping the VPN-bound daemon')
                case.check(process.child.poll() is None,'attached TUI never owns the engine lifetime')
                client.call('app.shutdown',{});process.wait(timeout=10,code=0)
                case.check(not h.engine_path.exists(),'explicit shutdown drains and removes api.sock')
                case.check(config.read_bytes()==before and {r['hash']:r['paused'] for r in json.loads(ledger.read_text())['records']}==expected,
                           'shutdown retains config and every saved pause/queue intent')
        finally:
            if ui:ui.close()
            if process.child.poll() is None:h.stop_serve(process)
            h.lock_free()
    return suite_status('vpn')


def headless_park_unpark(binary):
    """P1-03's opt-in daemon case; every link belongs to this new netns."""
    import daemon as harness_module
    import fixtures
    from fbtlog import Case, suite_status

    assert os.readlink("/proc/self/ns/net") != os.environ["FBT_TEST_HOST_NETNS"]
    with Case("daemon", "headless_park_unpark") as case, fixtures.Sandbox() as sandbox, \
            harness_module.Harness(case, binary, sandbox) as harness:
        def ip(*arguments):
            subprocess.run(["ip", *arguments], check=True, capture_output=True)

        ip("link", "set", "lo", "up")
        config = Path(sandbox.env["XDG_CONFIG_HOME"]) / "fbittorrent/config.toml"
        config.write_text(config.read_text().replace('network_interface = "lo"', 'network_interface = "fbtvpn0"'))
        directory = Path(sandbox.env["XDG_DATA_HOME"]) / "fbittorrent/engine"
        directory.mkdir(parents=True)
        records = []
        for number, paused in [(1, False), (2, False), (3, True)]:
            name = f"daemon-vpn-{number}.bin"
            info = {b"name": name.encode(), b"length": 8, b"piece length": 16384,
                    b"pieces": hashlib.sha1(b"fixture!").digest(), b"private": 1}
            digest = hashlib.sha1(fixtures.bencode(info)).hexdigest()
            (directory / f"{digest}.torrent").write_bytes(fixtures.bencode({b"info": info}))
            records.append(dict(hash=digest, name=name, source="", output_folder=str(sandbox.downloads),
                                only_files=None, upload_limit=32768, ratio_limit=0.0, category="fixture", tags="",
                                added_on=number, completion_on=0, paused=paused, resume_after_migration=False))
        ledger = directory / "torrents.json"
        ledger.write_text(json.dumps(dict(migrated_legacy=True, records=records)))
        expected = {row["hash"]: row["paused"] for row in records}
        process = harness.start_serve()
        if harness.receive_ready() and harness.wait_engine():
            inode, pid = harness.engine_path.stat().st_ino, harness.daemon_pid()

            def settle(label, ready, epoch, index=None):
                def observed():
                    network = harness.api("app.network")["state"]
                    rows = harness.api("torrents.list",{})["torrents"]
                    saved = {row["hash"]: row["paused"] for row in json.loads(ledger.read_text())["records"]}
                    all_rows = len(rows) == 3 and saved == expected
                    if not ready:
                        all_rows = all_rows and all("progress" not in row for row in rows)
                    final_state = f"state={'running' if ready else 'parked'} engine_epoch={epoch}"
                    settled = network.get("ready") is ready and (index is None or socket.if_nametoindex(network["interface"]) == index) and all_rows and final_state in harness.stderr()
                    return settled, f"network={network!r}; rows={rows!r}; saved={saved!r}; expect {final_state}."
                harness.wait_for(observed, label)
                case.check(harness.engine_path.stat().st_ino == inode and harness.daemon_pid() == pid,
                           "network transition retains the daemon listener and PID", f"inode={inode}; PID={pid}.")
            settle("daemon starts parked", False, 1)
            ip("link", "add", "fbtvpn0", "type", "dummy")
            ip("address", "add", "198.18.0.1/32", "dev", "fbtvpn0")
            ip("link", "set", "fbtvpn0", "up")
            settle("interface return starts an engine", True, 2, socket.if_nametoindex("fbtvpn0"))
            ip("link", "delete", "fbtvpn0")
            settle("interface loss drains back to parked", False, 3)
            ip("link", "add", "fbtvpn0", "type", "dummy")
            ip("address", "add", "198.18.0.1/32", "dev", "fbtvpn0")
            ip("link", "set", "fbtvpn0", "up")
            settle("replacement interface resumes saved intent", True, 4, socket.if_nametoindex("fbtvpn0"))
            harness.stop_serve(process)
            harness.lock_free()
    return suite_status("daemon")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--headless-child":
        sys.exit(headless_park_unpark(sys.argv[2]))
    elif len(sys.argv) > 1 and sys.argv[1] == "--headless":
        if os.environ.get("FBT_TEST_VPN") != "1":
            raise SystemExit("--headless requires explicit FBT_TEST_VPN=1 (owned user/network namespace only)")
        binary = str(Path(sys.argv[2] if len(sys.argv) > 2 else "target/debug/fbittorrent").resolve())
        sys.exit(subprocess.run(["unshare", "-Urn", "env", f"FBT_TEST_HOST_NETNS={os.readlink('/proc/self/ns/net')}",
                                 sys.executable, str(Path(__file__).resolve()), "--headless-child", binary]).returncode)
    elif len(sys.argv) > 1 and sys.argv[1] == "--child":
        sys.exit(child(sys.argv[2], sys.argv[3]))
    else:
        binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/fbittorrent").resolve())
        assert Path(binary).is_file(), "build the app before running its network fixture"
        with tempfile.TemporaryDirectory(prefix="fbittorrent-vpn-fixture-") as directory:
            subprocess.run(["unshare", "-Urn", "env", f"FBT_TEST_HOST_NETNS={os.readlink('/proc/self/ns/net')}",
                            sys.executable, str(Path(__file__).resolve()), "--child", binary, directory], check=True)
