#!/usr/bin/env python3
"""A/B the real widget against the same installed bar and an empty popup.

Only the measured Quickshell is sampled. The compositor, daemon and sampler
are excluded. Voluntary context switches measure sleep/wake cycles, not kernel
wakeup tracepoints (sched_schedstats is disabled on the owner's host).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
from types import SimpleNamespace
import subprocess
import time

from dashboard import Session, ROOT
from local_magnet import LocalMagnet
import fbtlog
from fbtlog import Case, suite_status
import private_net


def instrument(source,destination):
    import shutil
    shutil.copytree(source,destination)
    p=destination/'Service.qml';s=p.read_text()
    s=s.replace('  id: root\n','  id: root\n  property var perfCounters:({ticks:0,frames:0,syncRows:0})\n',1)
    if 'root.client.tick();' in s:
        assert s.count('root.client.tick();')==1
        s=s.replace('root.client.tick();','root.perfCounters.ticks++;root.client.tick();')
    else:
        clock=destination/'ServiceClock.qml';clock_text=clock.read_text()
        assert clock_text.count('s.client.tick()')==1
        clock.write_text(clock_text.replace('s.client.tick()','{s.perfCounters.ticks++;s.client.tick()}'))
    s=s.replace('  function applyFrame(frame, request) {','  function applyFrame(frame, request) {\n    perfCounters.frames++')
    s=s.replace('  function syncRows() {','  function syncRows() {\n    perfCounters.syncRows++')
    p.write_text(s)


def sample(pid):
    proc=Path('/proc')/str(pid)
    fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
    threads={}
    for task in (proc/'task').iterdir():
        try:
            status=dict(line.split(':',1) for line in (task/'status').read_text().splitlines() if ':' in line)
            stat=(task/'stat').read_text().rsplit(')',1)[1].split()
            threads[task.name]=dict(start=stat[19],name=status['Name'].strip(),runtime_ns=int((task/'schedstat').read_text().split()[0]),
                sleep=int(status['voluntary_ctxt_switches']),preempt=int(status['nonvoluntary_ctxt_switches']))
        except FileNotFoundError:pass
    return dict(cpu_s=sum(int(fields[i]) for i in (11,12))/os.sysconf('SC_CLK_TCK'),threads=threads)


def measure(q,seconds):
    before=q.state();initial=sample(q.child.pid);last=initial;cycles={};preempt=0;exited=0;runtime_ns=0;started=time.monotonic()
    while time.monotonic()-started<seconds:
        time.sleep(max(0,min(1,seconds-(time.monotonic()-started))))
        point=sample(q.child.pid)
        for tid,row in point['threads'].items():
            old=last['threads'].get(tid)
            if old and old['start']==row['start']:
                cycles[row['name']]=cycles.get(row['name'],0)+max(0,row['sleep']-old['sleep'])
                preempt+=max(0,row['preempt']-old['preempt'])
                runtime_ns+=max(0,row['runtime_ns']-old['runtime_ns'])
        exited+=len(last['threads'].keys()-point['threads'].keys());last=point
    elapsed=time.monotonic()-started;after=q.state()
    return dict(seconds=elapsed,cpu_pct=100*runtime_ns/1e9/elapsed,tick_cpu_pct=100*(last['cpu_s']-initial['cpu_s'])/elapsed,
        sleep_wake_cycles_per_s=sum(cycles.values())/elapsed,thread_cycles=cycles,preemptions=preempt,thread_exits=exited,
        bar_frames=after['barFrames']-before['barFrames'],popup_frames=after['popupFrames']-before['popupFrames'],
        counters={key:after['counters'].get(key,0)-value for key,value in before['counters'].items()},before=before,after=after)


def run(args):
    result=dict(revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),binary=str(args.binary),
        binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),kernel=platform.release(),cpus=os.cpu_count(),
        load_before=os.getloadavg(),wake_metric='per-thread voluntary context switches (sleep/wake-cycle proxy; kernel schedstats disabled)',
        source=str(args.plugin_source),source_sha256=hashlib.sha256(b''.join(p.relative_to(args.plugin_source).as_posix().encode()+p.read_bytes() for p in sorted(args.plugin_source.rglob('*')) if p.is_file())).hexdigest(),samples=[])
    args.output.parent.mkdir(parents=True,exist_ok=True)
    def persist():args.output.write_text(json.dumps(result,indent=2)+'\n')
    with Case('qs_idle_cost',args.label) as case,LocalMagnet(case,args.binary) as swarm:
        source=swarm.box.root/'measured-plugin';instrument(args.plugin_source,source)
        geometry={}
        for scenario in args.scenarios:
            if scenario=='down':swarm.stop()
            elif swarm.p.child.poll() is not None:swarm.start()
            # Fill the daemon's 60-sample history before calling an empty
            # popup steady-state idle; growing that chart is real startup work.
            if scenario=='idle' and not args.check_only:time.sleep(65)
            active=None
            if scenario=='active':
                swarm.payload.write_bytes(os.urandom(64*1024*1024))
                active,_,_=swarm.magnet(0)
                # Measure steady transfer/UI cost, independent of BEP 9
                # metadata discovery. Peers and payload still use the real
                # local tracker; do not relax the payload-readiness check.
                swarm.client.call('torrents.add',dict(source=dict(path=str(active.torrent)),seed=False))
                assert swarm.h.wait_for(lambda:((r:=swarm.client.call('torrents.get',dict(hash=active.info_hash))['result']['torrent']).get('completed_bytes',0)>0,json.dumps(r)),'local payload arrives')
            for repeat in range(args.repeats):
                for opened in (False,True):
                    # Reverse A/B order on the second repeat to reveal drift.
                    for enabled in ((True,False) if repeat%2==0 else (False,True)):
                        swarm.box.env.update(FBT_IDLE_PLUGIN='1' if enabled else '0',FBT_IDLE_SCENARIO=scenario)
                        proxy=SimpleNamespace(path=case.path.with_name(f'{scenario}_{repeat}_{int(opened)}_{int(enabled)}.jsonl'),check=case.check)
                        q=Session(proxy,swarm.box,recipe='idle-cost',source_link=source)
                        try:
                            q.wait(lambda:q.ipc('ready')=='true')
                            if not enabled and scenario in geometry:q.ipc('size',*geometry[scenario])
                            q.ipc('popup',str(opened).lower())
                            q.wait(lambda:q.state()['opened']==opened)
                            time.sleep(args.settle)
                            if enabled and opened:
                                layout=q.state();geometry[scenario]=(round(layout['width']),round(layout['height']))
                            prior=swarm.client.call('torrents.get',dict(hash=active.info_hash))['result']['torrent'] if active else None
                            if args.check_only:
                                before=q.state()
                                if args.assert_quiet:time.sleep(2.2)
                                row=dict(before=before,after=q.state())
                                if args.assert_quiet and enabled:
                                    assert row['after']['counters']['ticks']==before['counters']['ticks'],row
                                    if scenario=='down':assert row['after']['counters']==before['counters'],row
                            else:row=measure(q,args.seconds)
                            row.update(scenario=scenario,popup=opened,plugin=enabled,repeat=repeat)
                            row['socket_warning_lines']=sum('QLocalSocket::' in line for line in q.read_log().splitlines())
                            assert row['socket_warning_lines']==0,row
                            if active:
                                after=swarm.client.call('torrents.get',dict(hash=active.info_hash))['result']['torrent']
                                assert (args.check_only or after['completed_bytes']>prior['completed_bytes']) and not after['staged'] and not after['upload_enabled'] and after.get('uploaded_bytes',0)==0 and after['completed_bytes']<after['size'],after
                                row['payload_bytes']=after['completed_bytes']-prior['completed_bytes']
                            assert not (swarm.box.root/'shims/fbittorrent.log').exists(),'measurement must not launch the daemon'
                            result['samples'].append(row);persist();print(json.dumps({k:v for k,v in row.items() if k not in ('before','after')}),flush=True)
                        finally:
                            q.close()
                            # A new shell owns fresh watchers, sockets and widgets.
                            import shutil
                            shutil.rmtree(q.config)
            if active:swarm.remove(active)
        result['load_after']=os.getloadavg();persist()
        case.check(True,'A/B idle matrix uses real first-party bar surfaces and real local transfers',str(args.output))
    return suite_status('qs_idle_cost')


if __name__=='__main__':
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--plugin-source',type=Path,default=ROOT/'plugin/oliver.torrents')
    parser.add_argument('--seconds',type=float,default=30);parser.add_argument('--settle',type=float,default=3);parser.add_argument('--repeats',type=int,default=2)
    parser.add_argument('--scenarios',nargs='+',choices=['down','idle','active'],default=['down','idle','active'])
    parser.add_argument('--check-only',action='store_true',help='functional wiring only; collect no CPU or wakeup measurements')
    parser.add_argument('--assert-quiet',action='store_true',help='with --check-only, reject recurring client ticks in a healthy session')
    parser.add_argument('--label',default='baseline');parser.add_argument('--output',type=Path,required=True);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();args.binary=args.binary.resolve();args.plugin_source=args.plugin_source.resolve()
    if args.seconds<=0 or args.settle<0 or args.repeats<1:parser.error('positive window/repeats and nonnegative settle required')
    if args.assert_quiet and not args.check_only:parser.error('--assert-quiet requires --check-only')
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    raise SystemExit(run(args))
