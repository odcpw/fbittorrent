"""Owned real-engine UI preconditions; no simulated API or edited torrent state."""
from contextlib import ExitStack, contextmanager
import base64
import json
import os
from pathlib import Path
import socket
import struct
import sys
import tomllib
import daemon
import fixtures
from fbtlog import Case, suite_status
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'plugin/tests'))
import private_net

STATES=('empty','downloading','paused','table-200','ready','may-stall')


def stop_owned(harness,process):
    if process.child.poll() is None:harness.stop_serve(process)
    else:
        process.wait(timeout=10,code=0)
        harness.case.check(not harness.engine_path.exists(),'client-initiated shutdown removes the owned socket','owned process exited before fixture cleanup')
        harness.lock_free()


@contextmanager
def running(binary,state,case):
    private_net.require()
    if state in ('ready','may-stall'):
        from daemon_watch import pair,row,MIB
        with pair('visual_watch',binary,False,case_override=case) as (_,box,h,p,client,peer,item,payload):
            if not h.wait_for(lambda:((v:=row(client,item.info_hash)).get('playable',{}).get('ready') is True,json.dumps(v)),
                              'real watch prefix reaches the playback threshold',timeout=75):raise RuntimeError('watch threshold not reached')
            peer.call('transfer.set_limits',dict(upload_bps=MIB//2 if state=='may-stall' else 3*MIB//2))
            desired=state=='ready'
            if not h.wait_for(lambda:((v:=row(client,item.info_hash)).get('playable',{}).get('stall_free') is desired,json.dumps(v)),
                              'real watch fixture has truthful '+state+' rate',timeout=10):raise RuntimeError('watch rate not observed')
            yield box,h,client,row(client,item.info_hash)
        return
    with ExitStack() as stack:
        sandbox=stack.enter_context(daemon.Sandbox())
        harness=stack.enter_context(daemon.Harness(case,Path(binary).resolve(),sandbox,False))
        process=harness.start_serve()
        stack.callback(stop_owned,harness,process)
        if not (harness.receive_ready() and harness.wait_engine()):raise RuntimeError('UI fixture daemon did not become ready')
        client=stack.enter_context(daemon.ApiConn(harness));client.hello()
        row=None
        if state!='empty':
            source=sandbox.root/'pristine';source.mkdir()
            payload=source/'UI fixture.bin';payload.write_bytes(os.urandom(8*fixtures.MIB))
            torrent=fixtures.torrent_fixture(source,[payload])
            if state in ('downloading','table-200'):
                seedbox=stack.enter_context(daemon.Sandbox())
                seed=stack.enter_context(daemon.Harness(case,Path(binary).resolve(),seedbox,False))
                seed_process=seed.start_serve();stack.callback(seed.stop_serve,seed_process)
                if not (seed.receive_ready() and seed.wait_engine()):raise RuntimeError('UI seed did not become ready')
                port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
                peers=fixtures.bencode(dict(interval=1,complete=1,incomplete=0,peers=socket.inet_aton('127.0.0.1')+struct.pack('!H',port)))
                http=fixtures.HttpFixture(source,{'/announce':dict(base64=base64.b64encode(peers).decode())});stack.callback(http.close)
                torrent.torrent.write_bytes(fixtures.bencode(dict(info=torrent.info,announce=http.base+'/announce')))
                harness.seed_harness,harness.replay_seedbox=seed,seedbox
                item=dict(hash=torrent.info_hash,fixture=torrent,pristine=source)
                daemon.start_replay_seed(client,harness,item)
            client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(sandbox.downloads),stopped=state=='paused'))
            def ready():
                nonlocal row
                row=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
                desired='downloading' if state=='table-200' else state
                ok=row['state']==desired and (desired!='downloading' or (row.get('dl_bps',0)>0 and 0<row.get('completed_bytes',0)<row['size']))
                return ok,json.dumps(row)
            if not harness.wait_for(ready,'real UI precondition '+state,timeout=15):raise RuntimeError('UI state was not observed')
        if state=='table-200':
            import table_fixture
            table_fixture.add_paused(client,sandbox)
        yield sandbox,harness,client,row


def serve(binary,state):
    private_net.enter()
    with Case('qs_fixture',state.replace('-','_')) as case,running(binary,state,case) as (sandbox,harness,client,row):
        print('FBTFIXTURE '+json.dumps(dict(runtime=sandbox.env['XDG_RUNTIME_DIR'],root=str(sandbox.root),state=state,row=row)),flush=True)
        # EOF is the owner's teardown signal. No polling or retained children.
        sys.stdin.buffer.read(1)
    return suite_status('qs_fixture')
