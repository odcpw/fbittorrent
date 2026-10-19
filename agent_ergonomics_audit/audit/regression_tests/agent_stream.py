"""P1-18 live CLI event streams, raw envelopes, catalogue and startup policy."""
from contextlib import contextmanager
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import time
import tomllib

import daemon as d
from behavior_checks import well_formed_version
import fixtures
from daemon_bus import fixture


def cli(case,box,binary,*args,code=0):
    d.private_environment(box)
    step=case.step('CLI '+d.shlex.join(list(map(str,args))),f'exit {code}; capture exact stdout envelope/NDJSON')
    result=subprocess.run([str(binary),*map(str,args)],cwd=box.root,env=box.env,capture_output=True,text=True,timeout=30)
    step.close(result.returncode==code,json.dumps(dict(exit=result.returncode,stdout=result.stdout,stderr=result.stderr)))
    return result.stdout


def ndjson(case,text):
    rows=[json.loads(line) for line in text.splitlines()]
    for row in rows:
        case.check(isinstance(row.get('seq'),int) and len(row.get('instance',''))==32,
                   'each streamed line retains sequence and process instance',json.dumps(row))
    return rows


@contextmanager
def owned(name,binary,systemd,parked=False):
    with d.Case('agent_contract',name) as case,d.Sandbox() as box:
        if parked:
            path=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
            path.write_text(path.read_text().replace('network_interface = "lo"','network_interface = "fbt-no-such-vpn"'))
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(idle_exit_secs=0)
            if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private stream fixture not ready')
            try:
                with d.ApiConn(h) as client:
                    client.hello();yield case,box,h,p,client
            finally:
                if p.child.poll() is None:h.stop_serve(p)
                h.lock_free()


@contextmanager
def follow(case,box,binary,*args):
    path=case.path.with_suffix('.events.ndjson')
    with path.open('w') as output:
        child=case.popen([str(binary),'events','--follow','--json',*args],cwd=box.root,env=box.env,stdout=output)
        try:yield child,path
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                step=case.step('interrupt owned event reader','SIGINT exits 0 without stopping daemon')
                step.close(child.wait(timeout=5)==0,f'pid={child.pid}; exit={child.returncode}',[path])


def subscribed(h,client):
    return h.wait_for(lambda:((info:=client.call('app.info')['result'])['subscriptions']['by_topic'].get('events',0)==1,json.dumps(info)),
                      'one actual agent events subscription is attached')


def events_follow_completion(binary,systemd):
    with owned('events_follow_completion',binary,systemd) as (case,box,h,p,client),d.Sandbox() as source,d.Harness(case,binary,source) as seed:
        cfg=tomllib.loads((Path(source.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())
        body=fixtures.bencode(dict(interval=1,peers=socket.inet_aton('127.0.0.1')+struct.pack('>H',cfg['peer_port'])))
        with fixtures.HttpFixture(source.root,{'/announce':dict(base64=base64.b64encode(body).decode())}) as tracker:
            payload=source.downloads/'stream-complete.bin';payload.write_bytes(os.urandom(65536))
            item=fixtures.torrent_fixture(source.root,[payload],tracker=tracker.base+'/announce')
            sp=seed.start_serve()
            if not(seed.receive_ready() and seed.wait_engine()):raise RuntimeError('private peer not ready')
            try:
                with d.ApiConn(seed) as peer,follow(case,box,binary,'--kinds','torrent.completed') as (reader,path):
                    peer.hello();peer.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(source.downloads),seed=True))
                    seed.wait_for(lambda:((row:=peer.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)), 'real local seed verifies')
                    subscribed(h,client)
                    client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads)))
                    h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='done',json.dumps(row)), 'real peer transfer completes',timeout=20)
                    completed=time.monotonic()
                    h.wait_for(lambda:((rows:=[json.loads(line) for line in path.read_text().splitlines()])!=[],json.dumps(rows)), 'completion event reaches agent stdout',timeout=10)
                    rows=ndjson(case,path.read_text());elapsed=time.monotonic()-completed
                    case.check(elapsed<=10 and len(rows)==1 and rows[0]['hash']==item.info_hash and rows[0]['ev']=='torrent.completed'
                               and (box.downloads/payload.name).read_bytes()==payload.read_bytes(),
                               'one filtered completion arrives within ten seconds under load with exact downloaded bytes',f'elapsed={elapsed:.6f}s; rows={rows}')
                case.check(p.child.poll() is None,'Ctrl+C detaches the agent while the daemon survives',h.stderr())
            finally:
                if sp.child.poll() is None:seed.stop_serve(sp)


def events_since_seq_replay(binary,systemd):
    with owned('events_since_seq_replay',binary,systemd) as (case,box,h,p,client):
        with follow(case,box,binary,'--kinds','torrent.added') as (reader,path):
            subscribed(h,client);first=fixture(client,box,stopped=True,name='first-sequence')
            h.wait_for(lambda:('torrent.added' in path.read_text(),path.read_text()), 'first event is consumed by actual agent')
            rows=ndjson(case,path.read_text());cursor=rows[-1]['seq']
            reader.kill();code=reader.wait(timeout=5)
            case.check(code==-signal.SIGKILL,'kill the agent before creating later events',f'exit={code}; cursor={cursor}')
        later=[fixture(client,box,stopped=True,name=name).info_hash for name in ['second-sequence','third-sequence']]
        latest=[]
        def replayed():
            nonlocal latest
            latest=ndjson(case,cli(case,box,binary,'events','--since-seq',str(cursor),'--kinds','torrent.added','--json'))
            return [r.get('hash') for r in latest]==later,json.dumps(latest)
        h.wait_for(replayed,'resume prints exactly the two additions after the stored cursor')
        case.check(all(r['seq']>cursor for r in latest),'replayed seq values advance within the same instance',json.dumps(latest))


def events_resync_when_older(binary,systemd):
    with owned('events_resync_when_older',binary,systemd,parked=True) as (case,box,h,p,client):
        # Actual ledger additions while parked do not resolve metadata or reach
        # any public tracker. They exercise the production 1000-entry cap.
        for index in range(1002):
            hash_=hashlib.sha1(f'private-event-{index}'.encode()).hexdigest()
            client.call('torrents.add',dict(source=dict(magnet=f'magnet:?xt=urn:btih:{hash_}&dn=private-{index}')))
        latest=[]
        def old_cursor():
            nonlocal latest
            latest=ndjson(case,cli(case,box,binary,'events','--since-seq','0','--json'))
            return bool(latest and latest[0].get('reason')=='resync' and latest[0].get('oldest_seq',0)>1),json.dumps(latest[:1])
        h.wait_for(old_cursor,'old cursor receives explicit resync and actual oldest retained sequence',timeout=15)
        case.check(len(latest)<=1001 and all(r['seq']>=latest[0]['oldest_seq'] for r in latest[1:]),
                   'ring retains at most 1000 actual events after its resync marker',f'lines={len(latest)}; first={latest[:2]}')


def events_down_exit_4(binary,systemd):
    with d.Case('agent_contract','events_down_exit_4') as case,d.Sandbox() as box,d.Harness(case,binary,box) as h:
        error=json.loads(cli(case,box,binary,'events','--follow','--json',code=4))
        bad=json.loads(cli(case,box,binary,'events','--kinds','bad.kind','--json',code=2))
        case.check(error.get('code')==4 and error.get('hint')=='fbittorrent up' and bad.get('code')==2
                   and not d.DaemonContract.socket_path(box).exists(),'closed reads and bad kinds never start a daemon',json.dumps([error,bad]))


def api_list_matches(binary,systemd):
    with owned('api_list_matches',binary,systemd) as (case,box,h,p,client):
        fixture(client,box,stopped=True,name='shape-proof')
        frame=json.loads(cli(case,box,binary,'api','torrents.list','--json'))
        case.check(frame.get('id')==2 and frame.get('ok') is True,'raw API success preserves the exact wire envelope',json.dumps(frame))
        normal=json.loads(cli(case,box,binary,'list','--json'))['torrents']
        input_=box.root/'normalized.json';output=box.root/'shapes.json'
        input_.write_text(json.dumps(frame['result']['torrents']))
        validator=os.environ.get('FBT_API_CONTRACT_VALIDATOR')
        if not validator:
            candidates=[p for p in (d.REPO/'target/debug/deps').glob('fbittorrent-*') if p.is_file() and os.access(p,os.X_OK)]
            validator=str(max(candidates,key=lambda p:p.stat().st_mtime))
        env=dict(box.env,FBT_TEST_SHAPES_INPUT=str(input_),FBT_TEST_SHAPES_OUTPUT=str(output))
        step=case.step('apply actual Rust cli::shapes::rows to live API objects','bridge exits 0 and all CLI torrent fields equal its output')
        child=case.popen([validator,'cli::shapes::tests::live_shape_bridge','--exact','--ignored','--nocapture'],env=env,cwd=box.root,stdout=subprocess.DEVNULL)
        code=child.wait(timeout=10)
        expected=json.loads(output.read_text()) if output.exists() else None
        step.close(code==0 and normal==expected,json.dumps(dict(exit=code,cli=normal,shaped=expected)))
        hello=json.loads(cli(case,box,binary,'api','hello','--json'))
        case.check(hello.get('ok') is True and hello.get('id')==1,'raw hello uses the real first handshake once',json.dumps(hello))
        error=json.loads(cli(case,box,binary,'api','torrents.get','--arg','hash=bad','--json',code=2))
        case.check(error.get('error',{}).get('code')=='invalid_args' and error['error']['details']['key']=='hash',
                   'raw errors retain code, retryable and field details instead of a generic CLI message',json.dumps(error))


def api_list_matches_ipc_md(binary,systemd):
    with d.Case('agent_contract','api_list_matches_ipc_md') as case,d.Sandbox() as box,d.Harness(case,binary,box):
        catalog=json.loads(cli(case,box,binary,'api','--list','--json'))['operations']
        robot=json.loads(cli(case,box,binary,'robot-docs','api','--json'))['operations']
        caps=json.loads(cli(case,box,binary,'capabilities','--json'))
        import re
        names=[]
        for line in (d.REPO/'docs/IPC.md').read_text().splitlines():
            cells=re.split(r'(?<!\\)\|',line)
            if len(cells)==7 and cells[4].strip() in ['yes','no']:names.append(cells[1].strip().strip('`'))
        case.check(catalog==robot==caps['api_operations'] and sorted(names)==sorted(r['name'] for r in catalog)
                   and all(all(key in row for key in ['args','result','mutating','idempotent','since']) for row in catalog),
                   'catalogue, robot docs, capabilities and IPC op list agree',json.dumps(catalog))
        config=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text('save_path = '+json.dumps(str(box.downloads))+'\ninvalid = [')
        d.private_environment(box)
        step=case.step('read static catalogue with invalid private config','exit 0 without reading app config or launching an owner')
        result=subprocess.run([str(binary),'api','--list','--json'],cwd=box.root,env=box.env,capture_output=True,text=True,timeout=5)
        step.close(result.returncode==0,json.dumps(dict(exit=result.returncode,stdout=result.stdout,stderr=result.stderr)))
        independent=json.loads(result.stdout)
        case.check(independent['operations']==catalog,'static catalogue works even with an invalid private config',json.dumps(independent))
        case.check(well_formed_version(caps.get('contract_version')),
                   'capabilities exposes a well-formed contract version',json.dumps(caps.get('contract_version')))


def api_unknown_op_hint(binary,systemd):
    with d.Case('agent_contract','api_unknown_op_hint') as case,d.Sandbox() as box,d.Harness(case,binary,box):
        typo=json.loads(cli(case,box,binary,'api','torrents.lsit','--json',code=2))
        none=json.loads(cli(case,box,binary,'api','nope','--json',code=2))
        case.check(typo['error']['code']=='unknown_op' and 'torrents.list' in typo['error']['details']['nearest'] and 'torrents.list' in typo['error']['hint']
                   and none['error']['code']=='unknown_op' and none['error']['details']['nearest']==[] and none['error']['hint']=='fbittorrent api --list',
                   'near typo suggests actual distance-two name; unrelated nope has no fabricated neighbour',json.dumps([typo,none]))


def api_mutating_starts_daemon_read_does_not(binary,systemd):
    with d.Case('agent_contract','api_mutating_starts_daemon_read_does_not') as case,d.Sandbox() as box,d.Harness(case,binary,box) as h:
        if systemd:
            with d.InstalledUnit(h) as unit:
                unit.autostart()
                cli(case,box,binary,'api','torrents.list','--json',code=4)
                case.check(unit.state()=='inactive','raw read leaves isolated unit stopped',unit.state())
                item=fixtures.two_file_fixture(box.root)
                args=json.dumps(dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads),stopped=True))
                first=json.loads(cli(case,box,binary,'api','torrents.add','--json-args',args,'--ref','raw-add-once','--json'))
                unit.verify_child()
                again=json.loads(cli(case,box,binary,'api','torrents.add','--json-args',args,'--ref','raw-add-once','--json'))
                case.check(first['ok'] and first['result']==again['result'] and unit.state()=='active','raw mutation starts unit and explicit ref replays the same cached response',json.dumps([first,again]))
        else:
            cli(case,box,binary,'api','torrents.list','--json',code=4)
            cli(case,box,binary,'--no-start','api','torrents.add','--json-args','{"source":{"magnet":"magnet:?xt=urn:btih:'+'a'*40+'"}}','--json',code=4)
            case.check(not d.DaemonContract.socket_path(box).exists(),'raw read and prohibited write leave foreground daemon absent',str(box.root))
