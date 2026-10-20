"""P5-07: real 64 MiB tmpfs and 100 MiB local peer payload over api.sock."""
from contextlib import ExitStack, contextmanager
import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time

import daemon as d
import fbtlog
import fixtures

MIB=1024*1024


def run_case(name,binary,systemd):
    # The daemon and its clients share this owned user/mount namespace. Units
    # outside it cannot see the private tmpfs, so these disk cases use foreground
    # serve even when selected by the outer --systemd matrix.
    with d.Case('daemon',name) as case,d.Sandbox() as box:
        d.private_environment(box)
        check=subprocess.run(['unshare','-Urm','--','true'],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=5)
        if check.returncode:
            case.step('SKIP owned mount namespace','record platform refusal without touching a host filesystem').ok('SKIP: '+check.stderr.strip())
            return
        root=case.path.parents[1]/'disk-workers'/name/str(time.time_ns())
        cmd=['unshare','-Urm','--',sys.executable,str(Path(__file__).resolve()),'--namespace',name,str(binary),str(root)]
        out=case.path.with_suffix('.namespace.stdout.log')
        with out.open('w') as stream:
            child=case.popen(cmd,env=box.env,cwd=box.root,stdout=stream)
            try:code=child.wait(timeout=100)
            except subprocess.TimeoutExpired:
                child.send_signal(d.signal.SIGINT);code=child.wait(timeout=25)
        log=root/'daemon'/f'{name}.jsonl'
        rows=[json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        case.step('run isolated tmpfs acceptance','child completes with a passing final; all child/space observations retained').close(
            code==0 and bool(rows) and rows[-1].get('final') and all(row.get('ok') is not False for row in rows),
            json.dumps(dict(exit=code,requested_systemd=systemd,actual='owned mount namespace foreground',final=rows[-1] if rows else None)),[log,out])


@contextmanager
def mounted(case,box):
    d.private_environment(box)
    result=subprocess.run(['mount','-t','tmpfs','-o','size=64M,mode=700','tmpfs',str(box.downloads)],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=5)
    if result.returncode:
        case.step('SKIP tmpfs mount','record platform refusal').ok('SKIP: '+result.stderr.strip())
        yield False;return
    try:
        stat=os.statvfs(box.downloads)
        case.check(stat.f_blocks*stat.f_frsize==64*MIB,'actual destination is a 64 MiB tmpfs',str(stat))
        yield True
    finally:
        result=subprocess.run(['umount',str(box.downloads)],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=5)
        case.check(result.returncode==0,'unmount only the owned destination after stopping daemons',result.stderr or 'owned tmpfs unmounted')
        if result.returncode:box.retain()


def summary(client):
    sub=client.call('subscribe',dict(topics=['summary'],interval_ms=250))['result']['sub']
    def matches(frame):
        return frame.get('sub')==sub and frame.get('ev')=='state' and frame.get('topic')=='summary'
    try:
        for index,frame in enumerate(client.pushed):
            if matches(frame):return client.pushed.pop(index)['data']
        while True:
            frame=client.receive()
            if frame is None:raise EOFError('summary EOF')
            if matches(frame):return frame['data']
            client.pushed.append(frame)
    finally:
        # A queued state from another subscription must neither satisfy this
        # sample nor choose its cleanup ID. Drop our trailing frames only.
        client.call('unsubscribe',dict(sub=sub))
        client.pushed[:]=[frame for frame in client.pushed if frame.get('sub')!=sub]


def disk_summary_subscription_cleanup(binary,systemd):
    with d.api_owned('disk_summary_subscription_cleanup',binary,systemd) as (case,box,h,p),d.ApiConn(h) as client:
        client.hello()
        other=client.call('subscribe',dict(topics=['summary'],interval_ms=250))['result']['sub']
        # Retain a real state frame from a different, still-live subscription.
        # The old helper accepted it and unsubscribed its ID instead of its own.
        while not any(frame.get('sub')==other and frame.get('ev')=='state' for frame in client.pushed):
            frame=client.receive()
            if frame is None:raise EOFError('initial summary EOF')
            client.pushed.append(frame)
        original=next(frame for frame in client.pushed if frame.get('sub')==other and frame.get('ev')=='state')
        for index in range(24):
            value=summary(client)
            counts=client.call('app.info')['result']['subscriptions']
            case.check('disk' in value and counts['by_topic'].get('summary',0)==1 and original in client.pushed,
                       'sample releases only its own subscription and preserves the unrelated queued state',
                       json.dumps(dict(sample=index+1,subscriptions=counts,original_queued=original in client.pushed)))
        client.call('unsubscribe',dict(sub=other))
        counts=client.call('app.info')['result']['subscriptions']
        case.check(counts['by_topic'].get('summary',0)==0,'all summary subscriptions are released',json.dumps(counts))


def namespace_case(name,binary):
    with d.Case('daemon',name) as case,d.Sandbox() as box,mounted(case,box) as ok:
        if not ok:return
        # The fake notifier only records argv; it never opens a real window.
        fixtures.path_shim('notify-send','exit 0',box.root)
        config=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('notify = false','notify = true'))
        with d.Sandbox() as seedbox,d.Harness(case,binary,seedbox) as seed,d.Harness(case,binary,box) as h,ExitStack() as stack:
            payload=seedbox.downloads/'disk-100MiB.bin'
            with payload.open('wb') as stream:
                for _ in range(100):stream.write(os.urandom(MIB))
            port=d.tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
            body=fixtures.bencode(dict(interval=1,peers=socket.inet_aton('127.0.0.1')+port.to_bytes(2,'big')))
            http=stack.enter_context(fixtures.HttpFixture(seedbox.root,{'/announce':dict(base64=base64.b64encode(body).decode())}))
            item=fixtures.torrent_fixture(seedbox.root,[payload],tracker=http.base+'/announce')
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            c=stack.enter_context(d.ApiConn(h));c.hello()
            if name=='add_warns_disk':
                result=h.cli('add',str(item.torrent),'--stopped')
                case.check(result.get('ok') is True and result.get('disk',{}).get('needed_bytes')==100*MIB and result['disk']['free_bytes']<=64*MIB and any(row.get('code')=='disk_low' for row in result.get('warnings',[])),
                           'add exits 0 with real destination capacity and a binary-unit disk_low warning',json.dumps(result))
                h.stop_serve(p);return
            filler=box.downloads/'filler'
            with filler.open('wb') as stream:
                for _ in range(32):stream.write(b'F'*MIB)
                stream.flush();os.fsync(stream.fileno())
            sp=seed.start_serve(idle_exit_secs=0);seed.receive_ready();seed.wait_engine()
            peer=stack.enter_context(d.ApiConn(seed));peer.hello()
            peer.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(seedbox.downloads),seed=True))
            seed.wait_for(lambda:((row:=peer.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'actual 100 MiB seed verifies its payload',timeout=20)
            peer.call('transfer.set_limits',dict(upload_bps=2*MIB))
            c.call('torrents.add',dict(source=dict(path=str(item.torrent))))
            def low():
                value=summary(c);row=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                return value['disk']['low'] and row['state']!='error',json.dumps(dict(summary=value,row=row))
            h.wait_for(low,'low-space summary arrives before the first ENOSPC',timeout=8)
            counts=c.call('app.info')['result']['subscriptions']
            case.check(counts['by_topic'].get('summary',0)==0,'low-space polling leaves no summary subscriptions',json.dumps(counts))
            if name=='disk_low_banner':
                h.wait_for(lambda:((log:=box.root/'shims/notify-send.log').exists() and 'Disk almost full' in log.read_text(),log.read_text() if log.exists() else 'not notified'),'one Disk almost full notice',timeout=3)
                time.sleep(.2)
                notices=[json.loads(l) for l in (box.root/'shims/notify-send.log').read_text().splitlines()]
                case.check(sum('Disk almost full' in row for row in notices)==1 and any('window=Open window' in row for row in notices),'one normal disk notification offers the window action',json.dumps(notices))
            else:
                def full():
                    row=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                    st=os.statvfs(box.downloads)
                    return row['state']=='error' and row.get('reason')=='disk_full',json.dumps(dict(row=row,free_bytes=st.f_bavail*st.f_frsize,needed_bytes=100*MIB-row.get('completed_bytes',0)))
                h.wait_for(full,'real storage ENOSPC becomes error/disk_full',timeout=35)
                row=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                case.check('Disk full' in row.get('error',''),'disk-full error keeps the human phrase and OS diagnostic',json.dumps(row))
                h.wait_for(lambda:('WARN engine disk_full hash='+item.info_hash in h.stderr(),h.stderr()[-700:]),'ENOSPC free-space baseline is captured')
                if name=='disk_full_recovers':
                    previous=row.get('completed_bytes',0)
                    allocated=(box.downloads/payload.name).stat().st_blocks
                    with filler.open('wb') as stream:stream.flush();os.fsync(stream.fileno())
                    st=os.statvfs(box.downloads)
                    case.step('truncate only the owned filler','free space rises by at least 16 MiB; no start request sent').ok(json.dumps(dict(free_bytes=st.f_bavail*st.f_frsize,previous_completed=previous)))
                    def resumed():
                        row=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                        return 'disk_full_recovered hash='+item.info_hash in h.stderr() and row['state']=='downloading' and row.get('completed_bytes',0)>previous and (box.downloads/payload.name).stat().st_blocks>allocated,json.dumps(dict(row=row,allocated_blocks=(box.downloads/payload.name).stat().st_blocks,logs=h.stderr()[-900:]))
                    h.wait_for(resumed,'five-second sampler retries and downloads more bytes without start',timeout=15)
                else:
                    c.call('torrents.start',dict(hashes=[item.info_hash]))
            h.stop_serve(p);seed.stop_serve(sp)


def disk_low_banner(binary,systemd):run_case('disk_low_banner',binary,systemd)
def disk_full_reason(binary,systemd):run_case('disk_full_reason',binary,systemd)
def disk_full_recovers(binary,systemd):run_case('disk_full_recovers',binary,systemd)
def add_warns_disk(binary,systemd):run_case('add_warns_disk',binary,systemd)

if __name__=='__main__':
    if sys.argv[1]!='--namespace':raise SystemExit('owned namespace entry only')
    fbtlog.LOG_ROOT=Path(sys.argv[4]);namespace_case(sys.argv[2],Path(sys.argv[3]));sys.exit(fbtlog.suite_status('daemon'))
