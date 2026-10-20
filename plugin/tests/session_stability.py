#!/usr/bin/env python3
"""Long real-shell session on a capped localhost swarm; all state is private."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from dashboard import Session,ROOT
import daemon as d
import fixtures
import private_net
import fbtlog
from fbtlog import Case,suite_status


def sample(pid):
    fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
    status=dict(line.split(':',1) for line in Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line)
    return dict(cpu_s=sum(int(fields[i]) for i in (11,12))/os.sysconf('SC_CLK_TCK'),
                rss_kib=int(status['VmRSS'].split()[0]),hwm_kib=int(status['VmHWM'].split()[0]))


def run(args):
    with Case('qs_stability','long_session') as case,fixtures.daemon_pair(case,args.binary,False,16384) as pair:
        q=Session(case,pair.box,recipe='bar-host',journal_tag='fbt-stability-'+pair.box.root.name)
        events=[];samples=[];current=pair.p;client=pair.client;restarted=False;cycles=0
        seedbox=pair.owned[1][0]
        metainfo=seedbox.root/'fixture.torrent'
        expected=hashlib.sha256(pair.payload.read_bytes()).hexdigest()
        output=args.output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
        started=time.monotonic();last_action=started;last_bytes=0;last_progress=started
        def health():return json.loads(q.ipc('health'))
        def persist():
            output.write_text(json.dumps(dict(binary=str(args.binary),source=str(ROOT/'plugin/oliver.torrents'),
                revision=args.revision,plugin_sha256=args.plugin_hash,elapsed_s=time.monotonic()-started,seconds=args.seconds,
                samples=samples,events=events,cycles=cycles,journal=q.journal_tag),indent=2)+'\n')
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            q.shell('summon','{}');q.wait(lambda:health()['dashboard'])
            q.ipc('popup');q.wait(lambda:q.state()['opened'])
            assert Path(f'/proc/{q.child.pid}/exe').resolve().name in ('qs','quickshell')
            started=time.monotonic();last_action=started;last_progress=started
            while time.monotonic()-started<args.seconds:
                now=time.monotonic();row=client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
                if row.get('completed_bytes',0)>last_bytes:last_progress=now
                last_bytes=row.get('completed_bytes',0)
                state=health();point=sample(q.child.pid)
                point.update(elapsed_s=round(now-started,2),state=row['state'],completed_bytes=last_bytes,
                             online=state['online'],rows=state['rows'],pending=state['pending'])
                if samples:
                    point['cpu_pct']=100*(point['cpu_s']-samples[-1]['cpu_s'])/(point['elapsed_s']-samples[-1]['elapsed_s'])
                samples.append(point)
                assert state['online'] and state['dashboard'] and state['popup'],state
                assert state['rows']==1 and state['pending']<=8,state
                assert row['upload_enabled'] is False and row['uploaded_bytes']==0,row
                if row['state']=='done':
                    target=Path(row['content_path'])
                    assert target.is_relative_to(pair.box.downloads)
                    assert hashlib.sha256(target.read_bytes()).hexdigest()==expected
                    # Delete only the just-verified fixture payload; the seed
                    # and every owner path remain outside this operation.
                    client.call('torrents.remove',dict(hashes=[pair.hash],delete_files=True))
                    assert pair.h.wait_for(lambda:(not target.exists(),str(target)), 'owned completed payload deletion finishes')
                    client.call('torrents.add',dict(source=dict(path=str(metainfo)),save_path=str(pair.box.downloads)))
                    cycles+=1;last_bytes=0;last_progress=now
                    events.append(dict(at=now-started,event='verified_download_restarted',cycle=cycles))
                else:assert now-last_progress<120,dict(stalled_s=now-last_progress,row=row)
                if not restarted and now-started>=args.seconds/2:
                    pair.h.stop_serve(current);q.wait(lambda:not q.state()['online'])
                    cleared=health();assert cleared['rows']==0 and cleared['model']==0,cleared
                    pair.h.notify.close();client.close()
                    current=pair.h.start_serve(idle_exit_secs=0)
                    assert pair.h.receive_ready() and pair.h.wait_engine()
                    client=pair.stack.enter_context(d.ApiConn(pair.h));client.hello()
                    q.wait(lambda:q.state()['online']);q.wait(lambda:health()['rows']==1)
                    restarted=True;last_progress=now
                    events.append(dict(at=now-started,event='restart_with_both_windows_open'))
                if now-last_action>=args.action_interval:
                    # Actual native bar clicks exercise popup toggling. The
                    # final explicit open leaves both surfaces visible.
                    for _ in range(12):assert q.ipc('clickWidget')=='true'
                    for _ in range(5):
                        q.ipc('hideWindow');q.shell('summon','{}')
                        q.wait(lambda:health()['dashboard'])
                    q.ipc('popup');q.wait(lambda:q.state()['opened'])
                    events.append(dict(at=now-started,event='12_bar_clicks_5_window_reopens'))
                    last_action=now
                log=q.read_log()
                errors=[line for line in log.splitlines() if any(marker in line for marker in (
                    'TypeError:','ReferenceError:','SyntaxError:','Binding loop detected','Unable to assign',
                    "Can't create role",'Cannot create delegate','Required property','Error calling method'))]
                assert not errors,errors
                persist();print(json.dumps(point),flush=True)
                time.sleep(min(args.interval,max(0,args.seconds-(time.monotonic()-started))))
            case.check(restarted and bool(events),'long session exercises restart, rapid clicks and reopen loops',json.dumps(events))
            if args.seconds>=600:
                case.check(cycles>0,'at least one complete localhost payload is independently verified before restarting it',str(cycles))
            else:
                case.check(last_bytes>0,'short diagnostic observes real payload progress',str(last_bytes))
            pair.h.stop_serve(current);q.wait(lambda:not q.state()['online'])
            state=health();case.check(state['rows']==state['model']==0,'final stop clears the long-lived shell model',json.dumps(state))
        finally:
            persist();q.close()
    return suite_status('qs_stability')


if __name__=='__main__':
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--seconds',type=float,default=3600)
    parser.add_argument('--interval',type=float,default=60);parser.add_argument('--action-interval',type=float,default=300)
    parser.add_argument('--output',required=True,type=Path);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();args.binary=args.binary.resolve()
    if args.seconds<1 or args.interval<=0 or args.action_interval<=0:parser.error('positive durations required')
    args.revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
    source=ROOT/'plugin/oliver.torrents'
    args.plugin_hash=hashlib.sha256(b''.join(p.relative_to(source).as_posix().encode()+p.read_bytes() for p in sorted(source.rglob('*')) if p.is_file())).hexdigest()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    raise SystemExit(run(args))
