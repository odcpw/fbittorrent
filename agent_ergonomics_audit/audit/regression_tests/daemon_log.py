"""Real P1-14 stderr ring, API/CLI history, follow, and connection counters."""
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import time

import daemon as d
import fixtures


@contextmanager
def owned(name,binary,systemd,inject=False):
    with d.Case('daemon',name) as case, d.Sandbox() as box:
        payload=box.downloads/'log.bin';payload.write_bytes(b'local log fixture'*1024)
        item=fixtures.torrent_fixture(box.root,[payload]);fixtures.corrupt_byte(payload,0)
        if inject:
            box.env['FBT_TEST_INJECT_ERROR']=item.info_hash
            box.keys.append('FBT_TEST_INJECT_ERROR')
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve()
            if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private log daemon did not start')
            try:
                with d.ApiConn(h) as client:
                    client.trace=[]
                    client.hello()
                    yield case,box,h,p,client,item
            finally:
                if p.child.poll() is None:h.stop_serve(p)
                h.lock_free()


def admit(client,box,item):
    client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads),stopped=True))


def log_injected_error(binary,systemd):
    with owned('log_injected_error',binary,systemd,True) as (case,box,h,p,client,item):
        admit(client,box,item)
        def present():
            reply=h.cli('log','--level','error','--source','engine')
            entries=[entry for entry in reply.get('entries',[]) if entry.get('hash')==item.info_hash]
            return len(entries)==1,json.dumps(reply)
        h.wait_for(present,'one injected engine error is visible through CLI JSON')
        result=client.call('app.log',dict(hash=item.info_hash,level='error',source='engine',limit=1000))['result']
        case.check(len(result['entries'])==1 and result['entries'][0]['source']=='engine' and
                   'injected engine error' in result['entries'][0]['message'],
                   'the actual engine maintenance failure carries its exact source and hash',json.dumps(result))
        time.sleep(1.1)
        repeated=client.call('app.log',dict(hash=item.info_hash,level='error'))['result']
        case.check(repeated['entries']==result['entries'],'injected error appears exactly once across later maintenance ticks',json.dumps(repeated))
        empty=client.call('app.log',dict(since_seq=result['next_seq'],hash=item.info_hash))['result']
        case.check(empty['entries']==[],'exclusive next_seq cursor never repeats the previous entry',json.dumps(empty))
        frames=client.trace;client.trace=None
        bad=client.call('app.log',dict(limit=1001),error='invalid_args')
        case.check(bad['error']['details']['key']=='limit','oversized log query names the exact rejected limit',json.dumps(bad))
        if d.os.environ.get('FBT_API_CONTRACT_VALIDATOR'):d.validate_live_contract(case,box,frames)
        case.check(not (Path(box.env['XDG_DATA_HOME'])/'fbittorrent/engine/last-error.txt').exists(),
                   'the retired last-error file is never created',str(box.root))
        case.check(h.stderr().count('ERROR engine queue maintenance failed')==1,
                   'original stderr retains the same engine diagnostic exactly once',h.stderr())


def log_follow(binary,systemd):
    with owned('log_follow',binary,systemd,True) as (case,box,h,p,client,item):
        output=case.path.with_suffix('.follow.ndjson')
        with output.open('w') as stream:
            child=case.popen([str(binary),'--no-start','log','--follow','--json','--level','error','--source','engine'],
                             env=box.env,cwd=box.root,stdout=stream)
            try:
                h.wait_for(lambda:((stats:=client.call('app.stats')['result'])['active_clients']['agent']==1,json.dumps(stats)),
                           'read-only follow CLI connects without creating an owner')
                admit(client,box,item)
                h.wait_for(lambda:('ERROR engine queue maintenance failed' in h.stderr(),h.stderr()),'real injected diagnostic reaches original stderr')
                emitted=time.monotonic()
                def received():
                    rows=[json.loads(line) for line in output.read_text().splitlines()]
                    return any(row.get('hash')==item.info_hash for row in rows),json.dumps(rows)
                h.wait_for(received,'new log entry reaches NDJSON follow',timeout=1)
                elapsed=time.monotonic()-emitted
                rows=[json.loads(line) for line in output.read_text().splitlines()]
                case.step('measure stderr-to-follow delay','new entry arrives within one second').close(
                    elapsed<=1 and len(rows)==1 and rows[0]['source']=='engine',f'{elapsed:.6f} seconds; rows={rows}',[output])
                client.call('app.shutdown',{})
                step=case.step('wait for follow after owner shutdown','read-only stream exits 0 without restarting an owner')
                code=child.wait(timeout=10);step.close(code==0,f'exit={code}; owner launcher={p.child.poll()}')
            finally:
                if child.poll() is None:child.terminate();child.wait(timeout=5)


def hello_counters(binary,systemd):
    with owned('hello_counters',binary,systemd) as (case,box,h,p,client,item):
        before=client.call('app.info')['result']['clients']
        h.wait_for(lambda:((stats:=client.call('app.stats')['result'])['active_clients']['test']==1 and sum(stats['active_clients'].values())==1,json.dumps(stats)), 'closed API startup probes retire before live-counter baseline')
        before_active=client.call('app.stats')['result']['active_clients']
        sessions=[]
        try:
            for kind in ['cli','agent','tui','bar','test']:
                session=d.ApiConn(h);session.hello(kind);sessions.append(session)
            after=client.call('app.info')['result']['clients']
            stats=client.call('app.stats')['result']
            case.check(all(after[k]==before[k]+1 for k in before) and stats['connections']==after and
                       all(stats['active_clients'][k]==before_active[k]+1 for k in before_active),
                       'each successful hello increments its own kind once',json.dumps(dict(before=before,after=after,stats=stats)))
        finally:
            for session in sessions:session.close()
        def detached():
            stats=client.call('app.stats')['result']
            return stats['active_clients']==before_active,json.dumps(stats)
        h.wait_for(detached,'all five real sockets disappear from live counts')
        total=client.call('app.info')['result']['clients']
        case.check(total==after,'usage counters retain connections after detach',json.dumps(total))


def log_down_never_starts(binary,systemd):
    with d.Case('daemon','log_down_never_starts') as case, d.Sandbox() as box, d.Harness(case,binary,box) as h:
        reply=h.cli('log',code=4)
        case.check(reply.get('code')==4 and 'fbittorrent up' in reply.get('hint','') and 'journalctl --user -u fbittorrent' in reply.get('hint','')
                   and not d.DaemonContract.socket_path(box).exists(),
                   'closed log command exits 4 with both short and journal history hints, without an owner',json.dumps(reply))
