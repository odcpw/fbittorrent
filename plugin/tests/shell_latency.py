#!/usr/bin/env python3
"""xdg-open/Enter/popup/window frame timings with genuine 0/50/500 record sets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time
import traceback

from dashboard import ROOT
from responsiveness import PerformanceSession,summarize,pressure
from perf_support import instrument
from local_magnet import LocalMagnet
from desktop_setup import prepare as prepare_setup,call as setup_call
import private_net
import fbtlog
from fbtlog import Case,suite_status


class TimedSession(PerformanceSession):
    launcher=None
    def wait(self,fn,timeout=12):
        def observed():
            if self.launcher:self.launcher()
            return fn()
        return super().wait(observed,timeout)


def start_requested_daemon(swarm,launches):
    code=swarm.p.child.poll()
    if code is None:return
    # Only the deliberately stopped initial owner is adapted. A warm-session
    # death must retain its exit evidence, not attempt another notify.sock bind.
    if launches:raise RuntimeError(f'owned daemon exited {code} during warm session')
    log=swarm.box.root/'shims/fbittorrent.log'
    if not log.exists():return
    calls=[json.loads(line) for line in log.read_text().splitlines()]
    assert calls==[['up']],calls
    at=time.monotonic();swarm.start()
    launches.append(dict(daemon_ready_ms=(time.monotonic()-at)*1000,adapter='recorded up -> harness-owned real serve'))


def run(args,count):
    name=args.label+'_'+str(count)
    with Case('qs_latency',name) as case,LocalMagnet(case,args.binary) as swarm:
        swarm.paused(count)
        server=swarm.client.call('app.info')['result']['server']
        prepare_setup(swarm.box);setup_call(case,args.binary,swarm.box,'--magnet','--plugin=copy')
        config=Path(swarm.box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('autostart = false','autostart = true'))
        source=swarm.box.root/'measured-plugin';instrument(args.plugin_source,source)
        swarm.stop()
        q=TimedSession(case,swarm.box,recipe='responsiveness',source_link=source)
        keyboard=subprocess.Popen(['wtype','-s','600000'],env=q.env)
        launches=[];samples=[];child=None
        result=dict(label=args.label,rows=count,plugin_source=str(args.plugin_source),plugin_sha256=args.source_hash,
                    revision=args.revision,binary=str(args.binary),binary_sha256=args.binary_hash,server=server,
                    kernel=platform.release(),load_before=os.getloadavg(),pressure_before=pressure(),
                    renderer='private headless Sway pixman / Qt software',samples=samples,launches=launches)
        def launch():
            start_requested_daemon(swarm,launches)
        q.launcher=launch
        def state():return q.state().get('view') or {}
        def timed(name,spec,action,external=False):
            old=len(json.loads(q.ipc('samples')))
            assert q.ipc('arm',name,json.dumps(spec))=='true'
            began=time.time()*1000;action()
            q.wait(lambda:q.ipc('done')=='true',timeout=15)
            found=json.loads(q.ipc('samples'))[old:]
            assert found,name
            for row in found:
                row['gui_ms']=row['ms']
                if external:row['ms']=row['frame_at']-began
                encoded=json.dumps(dict(row,record_count=count))
                if args.check_only:
                    samples.append(dict(name=row['name'],observed=True))
                    continue
                samples.append(row);print(encoded,flush=True)
        try:
            q.wait(lambda:q.ipc('state'))
            # Desktop dispatch and shell summon are real; only user-manager
            # startup is adapted so the actual serve child stays in this net/XDG.
            shim=swarm.box.root/'shims/omarchy-shell'
            shim.write_text('#!/bin/sh\nexec /usr/share/omarchy/bin/omarchy-shell "$@"\n');shim.chmod(0o700)
            installed=Path(swarm.box.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents'
            assert installed.is_relative_to(swarm.box.root)
            shutil.rmtree(installed);installed.symlink_to(q.config/'Plugin',target_is_directory=True)
            for i in range(args.runs+1):
                item,magnet,title=swarm.magnet(i)
                def click():
                    nonlocal child
                    env=dict(q.env,DE='generic',XDG_CURRENT_DESKTOP='generic')
                    child=subprocess.Popen(['/usr/bin/xdg-open',magnet],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                timed('magnet_cold' if i==0 else 'magnet_warm',dict(kind='review',name=title),click,external=True)
                out,err=child.communicate(timeout=10);assert child.returncode==0,(out,err)
                q.ipc('focus')
                q.wait(lambda:(focus:=json.loads(q.ipc('focusState')))['active'] and not focus['pending'],timeout=3)
                timed('enter_cold' if i==0 else 'enter_warm',dict(kind='download',hash=item.info_hash),lambda:q.ipc('key',0x01000004,0),external=True)
                row=swarm.client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                assert not row['staged'] and row['state']=='downloading' and row['upload_enabled'] is False,row
                swarm.remove(item)
                q.wait(lambda:state().get('rows')==count)
                q.ipc('hide');q.wait(lambda:q.state()['view'] is None)
                timed('popup_open',dict(kind='popup',count=count),lambda:q.ipc('popup'),external=True)
                q.ipc('closePopup')
                timed('dashboard_open',dict(kind='open',count=count),lambda:q.shell('summon','{}'),external=True)
                q.ipc('hide');q.wait(lambda:q.state()['view'] is None)
            result.update(timings=None if args.check_only else summarize(samples),load_after=os.getloadavg(),pressure_after=pressure(),check_only=args.check_only)
            path=args.output/('latency-'+name+'.json');path.parent.mkdir(parents=True,exist_ok=True)
            path.write_text(json.dumps(result,indent=2)+'\n')
            case.check(len(launches)==1 and len(samples)>4*args.runs,
                       'measured real dispatch, Download, popup and window frames at '+str(count)+' records',str(path))
        except BaseException:
            traceback.print_exc()
            failure=dict(state=q.state(),focus=json.loads(q.ipc('focusState')),samples=json.loads(q.ipc('samples')),launches=launches,
                         child_exit=child.poll() if child else None)
            if child and child.poll() is not None:
                out,err=child.communicate(timeout=2);failure.update(stdout=out,stderr=err)
            case.path.with_suffix('.failure.json').write_text(json.dumps(failure,indent=2)+'\n')
            print(json.dumps(failure),flush=True)
            raise
        finally:
            q.launcher=None
            if child and child.poll() is None:child.terminate();child.communicate(timeout=3)
            keyboard.terminate();keyboard.wait(timeout=3);q.close()
    return suite_status('qs_latency')


if __name__=='__main__':
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--plugin-source',type=Path,default=ROOT/'plugin/oliver.torrents')
    parser.add_argument('--counts',nargs='+',type=int,default=[0,50,500]);parser.add_argument('--runs',type=int,default=20)
    parser.add_argument('--label',default='baseline');parser.add_argument('--output',required=True,type=Path);parser.add_argument('--log-root',type=Path)
    parser.add_argument('--check-only',action='store_true',help='functional frame/dispatch oracle only; do not publish latency measurements')
    args=parser.parse_args();args.binary=args.binary.resolve();args.plugin_source=args.plugin_source.resolve()
    if args.runs<1 or any(n<0 for n in args.counts):parser.error('positive runs and nonnegative record counts required')
    args.binary_hash=hashlib.sha256(args.binary.read_bytes()).hexdigest()
    args.revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
    args.source_hash=hashlib.sha256(b''.join(p.relative_to(args.plugin_source).as_posix().encode()+p.read_bytes() for p in sorted(args.plugin_source.rglob('*')) if p.is_file())).hexdigest()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    for count in args.counts:
        if run(args,count):raise SystemExit(1)
