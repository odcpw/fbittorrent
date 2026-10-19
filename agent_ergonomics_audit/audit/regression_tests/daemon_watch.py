"""P5-05 real verified pieces, loopback transfers, CLI and desktop argv."""
from contextlib import contextmanager, nullcontext
import base64
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib

import daemon as d
import daemon_notify as notify
import fixtures

MIB=1024*1024

def cli(*args, **kwargs):
    # daemon registers these cases while importing; load the CLI helper only
    # when a case runs so direct agent_stream imports remain acyclic.
    from agent_contract import cli as run
    return run(*args, **kwargs)


def header():
    return bytes.fromhex('1a45dfa38018538067ff1549a9668b448988')+__import__('struct').pack('>d',64000.)

def row(client,hash_):return client.call('torrents.get',dict(hash=hash_))['result']['torrent']

def history(client):
    client.call('app.info')
    events={}
    for frame in client.pushed:
        for event in [frame,*frame.get('items',[]),*frame.get('added',[])]:
            if 'seq' in event:events[event['seq']]=event
    return list(events.values())

@contextmanager
def pair(name,binary,systemd,extension='mkv',size=64*MIB,action='',case_override=None,prepared_prefix=0):
    binary=Path(binary).resolve()
    with (nullcontext(case_override) if case_override else d.Case('daemon',name)) as case,d.Sandbox() as box,d.Sandbox() as seedbox:
        notify.shims(box,action);notify.config(box,notify=True)
        with d.Harness(case,binary,box,systemd) as h,d.Harness(case,binary,seedbox,systemd) as seed:
            p=h.start_serve(idle_exit_secs=0);sp=seed.start_serve(idle_exit_secs=0)
            h.receive_ready();h.wait_engine();seed.receive_ready();seed.wait_engine()
            try:
                with d.ApiConn(h) as client,d.ApiConn(seed) as peer:
                    client.hello();peer.hello();client.call('subscribe',dict(topics=['events']))
                    port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
                    import socket
                    peers=socket.inet_aton('127.0.0.1')+port.to_bytes(2,'big')
                    body=fixtures.bencode(dict(interval=1,peers=peers))
                    with fixtures.HttpFixture(seedbox.root,{'/announce':dict(base64=base64.b64encode(body).decode())}) as tracker:
                        payload=seedbox.downloads/('Fixture.'+extension)
                        with payload.open('wb') as f:
                            f.write(header() if extension=='mkv' else b'local iso fixture')
                            while f.tell()<size:f.write(os.urandom(min(MIB,size-f.tell())))
                        item=fixtures.torrent_fixture(seedbox.root,[payload],tracker=tracker.base+'/announce',piece_length=MIB)
                        peer.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(seedbox.downloads),seed=True))
                        seed.wait_for(lambda:((v:=row(peer,item.info_hash))['state']=='seeding',json.dumps(v)),'loopback seed verifies fixture payload')
                        peer.call('transfer.set_limits',dict(upload_bps=MIB))
                        if prepared_prefix:
                            # Real existing verified bytes make the transition
                            # case quick without changing its 64 MiB/bitrate bar.
                            with payload.open('rb') as source,(box.downloads/payload.name).open('wb') as target:
                                target.truncate(size);target.write(source.read(prepared_prefix))
                                source.seek(-MIB,2);target.seek(-MIB,2);target.write(source.read())
                        client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads)))
                        yield case,box,h,p,client,peer,item,payload
            finally:
                for owner,process in [(h,p),(seed,sp)]:
                    if process.child.poll() is None:owner.stop_serve(process)
                    owner.lock_free()


def playable_before_complete(binary,systemd):
    with pair('playable_before_complete',binary,systemd,action='open') as (case,box,h,p,client,peer,item,payload):
        command=[str(binary),'wait',item.info_hash,'--until','playable','--timeout','75','--interval','1','--json']
        waiter=case.popen(command,env=box.env,cwd=box.root,stdout=subprocess.PIPE,text=True)
        ready=[]
        def reached():
            v=row(client,item.info_hash);ready[:]=[v]
            return v.get('playable',{}).get('ready') is True,json.dumps(v)
        if not h.wait_for(reached,'verified 32 MiB prefix and video tail become ready before completion',timeout=75):raise RuntimeError('threshold not reached')
        v=ready[0];play=v['playable'];events=history(client)
        case.check(v['completed_bytes']<v['size'] and play['prefix_bytes']>=32*MIB and play.get('buffered_s',0)>=32,
                   '64 MiB Matroska is ready with a measured buffer while incomplete',json.dumps(v))
        case.check(sum(e['ev']=='torrent.playable' and e.get('hash')==item.info_hash for e in events)==1 and not any(e['ev']=='torrent.completed' and e.get('hash')==item.info_hash for e in events),
                   'one torrent.playable arrives before torrent.completed',json.dumps(events))
        output,_=waiter.communicate(timeout=5)
        answer=json.loads(output)
        case.check(waiter.returncode==0 and answer['playable'] is True and answer['complete'] is False and answer['torrents'][0]['open_target']==str(box.downloads/'Fixture.mkv'),'wait --until playable exits 0 before completion',output)
        h.wait_for(lambda:(any(str(box.downloads/'Fixture.mkv') in call for call in notify.calls(box,'xdg-open')),json.dumps(notify.calls(box,'xdg-open'))),'notification Open invokes xdg-open with the video file')
        h.wait_for(lambda:((v:=row(client,item.info_hash))['playable']['acked'],json.dumps(v)),'notification Open acknowledges ready state in the shared model')
        notify.observe(case,box);notify.observe(case,box,'xdg-open')
        peer.call('transfer.set_limits',dict(upload_bps=MIB//2))
        h.wait_for(lambda:((v:=row(client,item.info_hash))['playable'].get('stall_free') is False,json.dumps(v)),'slower-than-playback link truthfully flips stall_free false',timeout=12)
        peer.call('transfer.set_limits',dict(upload_bps=4*MIB))
        h.wait_for(lambda:((v:=row(client,item.info_hash))['playable'].get('stall_free') is True,json.dumps(v)),'faster link flips stall_free true',timeout=2)
        peer.call('transfer.set_limits',dict(upload_bps=MIB))
        cli(case,box,binary,'wait',item.info_hash,'--until','stall-free','--timeout','2','--json')
        client.call('torrents.stop',dict(hashes=[item.info_hash]))
        before=row(client,item.info_hash)
        case.check(before['state']=='paused' and before['playable']['ready'],'pause preserves verified readiness',json.dumps(before))
        target=client.call('torrents.open_target',dict(hash=item.info_hash))['result']['path']
        case.check(target==str(box.downloads/'Fixture.mkv'),'open_target selects the ready video',target)
        h.stop_serve(p);h.notify.close();replacement=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        try:
            with d.ApiConn(h) as fresh:
                fresh.hello();fresh.call('subscribe',dict(topics=['events']))
                h.wait_for(lambda:((v:=row(fresh,item.info_hash)).get('playable',{}).get('ready') is True,json.dumps(v)),'paused verified prefix survives daemon restart',timeout=15)
                events=history(fresh)
                case.check(not any(e['ev']=='torrent.playable' for e in events),'restart never repeats an already-ready incomplete notification',json.dumps(events))
        finally:
            if replacement.child.poll() is None:h.stop_serve(replacement)
        # Dirty payload invalidates trusted resume. A real startup check loses
        # the prefix; a subsequent seed repair may fire once in the new daemon.
        path=box.downloads/'Fixture.mkv'
        with path.open('r+b') as f:f.write(b'\0')
        h.notify.close();last=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        try:
            with d.ApiConn(h) as fresh:
                fresh.hello();fresh.call('subscribe',dict(topics=['events']))
                h.wait_for(lambda:((v:=row(fresh,item.info_hash)).get('playable',{}).get('ready') is False,json.dumps(v)),'startup check detects the lost first piece',timeout=15)
                fresh.call('torrents.start',dict(hashes=[item.info_hash]))
                h.wait_for(lambda:(any(e['ev']=='torrent.playable' for e in history(fresh)),json.dumps(history(fresh))),'repair re-arms exactly one playable event after prefix loss',timeout=15)
                events=history(fresh);case.check(sum(e['ev']=='torrent.playable' for e in events)==1,'prefix loss allows one new event in the restarted daemon',json.dumps(events))
        finally:
            if last.child.poll() is None:h.stop_serve(last)


def playable_stall_free_flips(binary,systemd):
    with pair('playable_stall_free_flips',binary,systemd,prepared_prefix=32*MIB) as (case,box,h,p,client,peer,item,payload):
        peer.call('transfer.set_limits',dict(upload_bps=4*MIB))
        client.call('transfer.set_limits',dict(download_bps=4*MIB,alt_download_bps=200*1024))
        client.call('transfer.set_alt',dict(on=True))
        def may_stall():
            v=row(client,item.info_hash)
            return v.get('playable',{}).get('ready') is True and v['playable'].get('stall_free') is False,json.dumps(v)
        assert h.wait_for(may_stall,'ready video truthfully reports may stall with turtle below its bitrate',timeout=12)
        slow=row(client,item.info_hash)
        case.check(slow['completed_bytes']<slow['size'] and slow.get('effective_download_limit_bps')==200*1024,
                   'turtle is the real active download cap while verified video is incomplete',json.dumps(slow))
        began=time.monotonic();client.call('transfer.set_alt',dict(on=False))
        def stall_free():
            v=row(client,item.info_hash)
            return v.get('playable',{}).get('ready') is True and v['playable'].get('stall_free') is True,json.dumps(v)
        assert h.wait_for(stall_free,'lifting turtle restores stall-free within two seconds',timeout=2)
        case.check(time.monotonic()-began<=2,'stall-free update meets the two-second transition budget',json.dumps(row(client,item.info_hash)))


def playable_never_for_iso(binary,systemd):
    with pair('playable_never_for_iso',binary,systemd,extension='iso',size=8*MIB) as (case,box,h,p,client,peer,item,payload):
        def complete():
            v=row(client,item.info_hash)
            case.check('playable' not in v,'ISO never has playback metadata at any transfer state',json.dumps(v))
            return v.get('progress')==1.,json.dumps(v)
        h.wait_for(complete,'real ISO transfer completes without a playable event',timeout=25)
        case.check(not any(e['ev']=='torrent.playable' for e in history(client)),'ISO has no torrent.playable event',json.dumps(history(client)))
        cli(case,box,binary,'wait',item.info_hash,'--until','playable','--timeout','1','--json',code=8)
        cli(case,box,binary,'wait',item.info_hash,'--until','stall-free','--timeout','1','--json',code=8)


def playable_two_file_mid_piece(binary,systemd):
    with d.api_owned('playable_two_file_mid_piece',binary,systemd) as (case,box,h,p),d.ApiConn(h) as client:
        source=box.root/'source';source.mkdir();lead=source/'readme.bin';lead.write_bytes(os.urandom(MIB+MIB//2))
        video=source/'video.mkv'
        with video.open('wb') as f:f.write(header()+os.urandom(64*MIB-len(header())))
        item=fixtures.torrent_fixture(source,[lead,video],piece_length=MIB)
        target=box.downloads;out=target/lead.name
        with out.open('wb') as f:f.truncate(lead.stat().st_size);f.seek(MIB);f.write(lead.read_bytes()[MIB:])
        with (target/video.name).open('wb') as f:
            f.truncate(64*MIB);f.seek(0)
            with video.open('rb') as inp:f.write(inp.read(33*MIB));inp.seek(-MIB,2);f.seek(-MIB,2);f.write(inp.read())
        client.hello();client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(target),stopped=True))
        captured=[]
        def ready():
            v=row(client,item.info_hash);captured[:]=[v]
            return v.get('playable',{}).get('ready') is True,json.dumps(v)
        h.wait_for(ready,'video ready despite absent torrent piece zero; own first piece is shared with previous file',timeout=15)
        play=captured[0]['playable']
        case.check(play['file']==1 and play['prefix_bytes']==32*MIB+MIB//2,'mid-piece video prefix clips both the first and final verified boundary',json.dumps(captured[0]))
        files=client.call('torrents.files',dict(hash=item.info_hash))['result']['files']
        case.check('ready' not in files[0] and files[1]['ready'] is True,'only the chosen video gets its per-file readiness dot',json.dumps(files))
        cli(case,box,binary,'wait',item.info_hash,'--until','playable','--json')
        cli(case,box,binary,'wait',item.info_hash,'--until','stall-free','--timeout','1','--json',code=8)


CASES=(playable_before_complete,playable_stall_free_flips,playable_never_for_iso,playable_two_file_mid_piece)
