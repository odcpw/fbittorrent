"""P1-16b: actual completed fixtures, pre-READY subscription and explicit actions."""
from contextlib import contextmanager
import json
from pathlib import Path
import re
import socket
import time

import daemon as d
from daemon_bus import RevisionStream
import fixtures


def checks(h,offset,hashes):
    return [line for line in h.stderr()[offset:].splitlines() if 'source=engine' in line and
            re.search(r'initializ|validat|check',line,re.I) and any(hash_ in line for hash_ in hashes)]


def states(frames,hashes):
    sequence={hash_:[] for hash_ in hashes}
    for frame in frames:
        for row in frame.get('items',[])+frame.get('added',[]):
            if row.get('hash') in sequence and 'state' in row:sequence[row['hash']].append(row['state'])
        for hash_,patch in frame.get('changed',{}).items():
            if hash_ in sequence and 'state' in patch:sequence[hash_].append(patch['state'])
    return sequence


@contextmanager
def restored(name,binary,systemd,missing=False):
    with d.Case('daemon',name) as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        fixtures.path_shim('xdg-open','exit 0',box.root)
        p=h.start_serve(idle_exit_secs=0)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('completion preparation not ready')
        items=[]
        with d.ApiConn(h) as client:
            client.hello()
            for index in range(5):
                payload=box.downloads/f'dormant-{index}.bin';payload.write_bytes(bytes([65+index])*262144)
                item=fixtures.torrent_fixture(box.root,[payload]);items.append((item,payload))
                client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads)))
            def verified():
                rows=client.call('torrents.list',{})['result']['torrents']
                return len(rows)==5 and all(row['state']=='done' and row['checked'] and row.get('completed_at',0)>0 for row in rows),json.dumps(rows)
            if not h.wait_for(verified,'five real fixtures finish their actual initial checks'):raise RuntimeError('preparation never completed')
        h.stop_serve(p);h.notify.close()
        if missing:items[0][1].unlink()
        offset=len(h.stderr())
        gate=Path(box.env['XDG_RUNTIME_DIR'])/'fbittorrent/ready.fixture-gate'
        box.env['FBT_TEST_READY_GATE']=str(gate);box.keys.append('FBT_TEST_READY_GATE')
        p=h.start_serve(idle_exit_secs=0)
        if not h.wait_for(lambda:(d.DaemonContract.socket_path(box).exists(),'private API listener exists before release'),'pre-READY listener'):raise RuntimeError('listener missing')
        with d.ApiConn(h) as client:
            client.hello();sub=client.call('subscribe',dict(topics=['torrents'],interval_ms=250))['result']['sub']
            try:early=h.notify.receive(timeout=0)
            except BlockingIOError:early=None
            case.check(early is None,'observer subscribes before daemon emits READY=1',repr(early))
            gate.touch();h.receive_ready();ready_at=h.notify.received_at
            stream=RevisionStream(client);frames=stream.snapshot(sub,['torrents'])
            elapsed=time.monotonic()-ready_at
            rows=[row for frame in frames for row in frame.get('items',[])]
            case.step('measure five dormant rows from READY=1','all five list without hashing within one second; unavailable selected data is an error').close(
                elapsed<=1 and len(rows)==5 and all(row['checked'] is False and (row['state']=='error' and row.get('reason')=='missing_files' and 'progress' not in row if missing and row['hash']==items[0][0].info_hash else row['state']=='done') for row in rows),
                json.dumps(dict(elapsed_s=elapsed,rows=rows)))
            try:yield case,box,h,p,client,items,offset,stream,frames
            finally:
                history=states(frames+client.pushed,[item.info_hash for item,_ in items])
                final_rows=client.call('torrents.list',{})['result']['torrents']
                for row in final_rows:
                    if row['hash'] in history:history[row['hash']].append(row['state'])
                case.step('record per-hash observed states and validation diagnostics','all five fixture histories and matching engine lines retained').ok(
                    json.dumps(dict(states=history,matching_logs=checks(h,offset,list(history)))))
                if p.child.poll() is None:h.stop_serve(p)
                h.lock_free()


def lazy_restore_no_checks(binary,systemd):
    with restored('lazy_restore_no_checks',binary,systemd) as (case,box,h,p,client,items,offset,stream,frames):
        end=time.monotonic()+1.1;client.socket.settimeout(.1)
        try:
            while time.monotonic()<end:
                try:
                    frame=client.receive()
                    if frame is not None:frames.append(frame)
                except socket.timeout:pass
        finally:client.socket.settimeout(8)
        hashes=[item.info_hash for item,_ in items];sequence=states(frames,hashes);logs=checks(h,offset,hashes)
        current=client.call('torrents.list',{})['result']['torrents']
        case.check(all(seq and 'checking' not in seq for seq in sequence.values()) and not logs and
                   all(row['state']=='done' and not row['checked'] for row in current),
                   'no completed fixture is ever admitted for startup checking',json.dumps(dict(states=sequence,matching_logs=logs,rows=current)))


def dormant_seed_checks(binary,systemd):
    with restored('dormant_seed_checks',binary,systemd) as (case,box,h,p,client,items,offset,stream,frames):
        item,_=items[0]
        h.cli('upload',item.info_hash)
        def seeding():
            rows=client.call('torrents.list',{})['result']['torrents']
            selected=next(row for row in rows if row['hash']==item.info_hash)
            return selected['state']=='seeding' and selected['checked'] and selected['seed'],json.dumps(rows)
        h.wait_for(seeding,'CLI seed checks and seeds only its dormant record')
        h.wait_for(lambda:(len(logs:=checks(h,offset,[item.info_hash]))==1,json.dumps(logs)), 'actual seed validation diagnostic reaches captured stderr')
        rows=client.call('torrents.list',{})['result']['torrents'];logs=checks(h,offset,[item.info_hash])
        case.check(len(logs)==1 and all(row['state']=='done' and not row['checked'] for row in rows if row['hash']!=item.info_hash),
                   'one real validation admission is the positive control; four other rows remain dormant',json.dumps(dict(matching_logs=logs,rows=rows)))
        client.call('torrents.start',dict(hashes=[items[1][0].info_hash]),error='invalid_state')
        untouched=client.call('torrents.get',dict(hash=items[1][0].info_hash))['result']['torrent']
        case.check(untouched['state']=='done' and not untouched['checked'],'ordinary start preserves completed stop-on-complete dormancy',json.dumps(untouched))


def dormant_missing_files(binary,systemd):
    with restored('dormant_missing_files',binary,systemd,missing=True) as (case,box,h,p,client,items,offset,stream,frames):
        item,path=items[0]
        before=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
        error=client.call('torrents.set_seed',dict(hashes=[item.info_hash],on=True),error='invalid_state')
        row=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
        logs=checks(h,offset,[item.info_hash])
        case.check(before['state']=='error' and before.get('reason')=='missing_files' and 'progress' not in before and not before['checked'] and row['state']=='error' and row.get('reason')=='missing_files'
                   and not row['checked'] and error['error']['details']['reason']=='missing_files' and not logs and not path.exists(),
                   'listing already reports missing data before explicit seed; no admission or file creation',json.dumps(dict(before=before,error=error,after=row,matching_logs=logs)))


def dormant_open_no_check(binary,systemd):
    with restored('dormant_open_no_check',binary,systemd) as (case,box,h,p,client,items,offset,stream,frames):
        item,path=items[0];began=time.monotonic()
        target=client.call('torrents.open_target',dict(hash=item.info_hash))
        elapsed=time.monotonic()-began
        h.cli('open',item.info_hash)
        row=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
        logs=checks(h,offset,[item.info_hash])
        case.step('measure dormant open_target','round trip below 100 ms; saved file path; checked stays false').close(
            elapsed<.1 and target['result']['path']==str(path) and row['state']=='done' and not row['checked'] and not logs,
            json.dumps(dict(roundtrip_ms=elapsed*1000,target=target,row=row,matching_logs=logs)))
        path.unlink();error=client.call('torrents.open_target',dict(hash=item.info_hash),error='invalid_state')
        after=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
        case.check(error['error']['details'].get('reason')=='missing_files' and after['state']=='error' and after.get('reason')=='missing_files' and not after['checked'],
                   'a missing dormant open target becomes a structured error without a check',json.dumps(dict(error=error,row=after,matching_logs=checks(h,offset,[item.info_hash]))))
        second=items[1][0]
        client.call('torrents.set_files',dict(hash=second.info_hash,include=[0]))
        h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=second.info_hash))['result']['torrent'])['checked'],json.dumps(row)), 'nonempty file inclusion activates actual dormant verification')
        third,third_path=items[2]
        h.cli('delete',third.info_hash,'--files','--yes')
        remaining=client.call('torrents.list',{})['result']['torrents']
        case.check(not third_path.exists() and all(row['hash']!=third.info_hash for row in remaining) and items[3][1].exists(),
                   'CLI delete-files removes only its dormant record/payload through the typed API',json.dumps(remaining))
