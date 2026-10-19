#!/usr/bin/env python3
"""Frame-presented interaction/CPU measurements in an owned 200-row shell."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import socket
import statistics
import subprocess
import time
import traceback

from dashboard import Session, ROOT
from visual_fixture import running
from fbtlog import Case, suite_status


class PerformanceSession(Session):
    """Use only this owned shell's current IPC; no process-per-sample overhead."""
    def ipc(self,method,*args):
        import private_net
        private_net.require()
        identity=(self.env['OMARCHY_PATH']+'/shell\n'+self.env['WAYLAND_DISPLAY']).encode()
        path=Path(self.env['XDG_RUNTIME_DIR'])/('omarchy-shell-'+hashlib.md5(identity).hexdigest()[:16]+'.sock')
        fields=['fbt-dashboard',method,*map(str,args)]
        assert all('\x1e' not in f and '\x1f' not in f for f in fields)
        try:
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as channel:
                channel.settimeout(3);channel.connect(str(path));channel.sendall(('\x1f'.join(fields)+'\x1e').encode())
                reply=b''
                while not reply.endswith(b'\x1e'):
                    chunk=channel.recv(65536)
                    if not chunk or len(reply)+len(chunk)>1048576:raise RuntimeError('incomplete or oversized private IPC reply')
                    reply+=chunk
        except OSError as error:raise RuntimeError(str(error)) from error
        if not reply.startswith(b'OK\x1f'):raise RuntimeError(repr(reply))
        return reply[3:-1].decode()


def process_sample(pid):
    fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
    return dict(cpu=sum(int(fields[i]) for i in (11,12))/os.sysconf('SC_CLK_TCK'),major_faults=int(fields[9]))


def pressure():return {name:Path('/proc/pressure/'+name).read_text() for name in ('cpu','memory','io')}


def measure_cpu(q,seconds):
    before=json.loads(q.ipc('counts'));start=process_sample(q.child.pid)['cpu'];at=time.monotonic()
    time.sleep(seconds)
    elapsed=time.monotonic()-at
    after=json.loads(q.ipc('counts'))
    return dict(seconds=elapsed,cpu_pct=(process_sample(q.child.pid)['cpu']-start)/elapsed*100,
                sync_rows=after['syncRows']-before['syncRows'],frames=after['frames']-before['frames'],
                popup_delegates=after['popupDelegates'])


def summarize(samples):
    result={}
    for name in dict.fromkeys(s['name'] for s in samples):
        values=sorted(s['ms'] for s in samples if s['name']==name)
        result[name]=dict(n=len(values),first_ms=next(s['ms'] for s in samples if s['name']==name),
                          p50_ms=statistics.median(values),p95_ms=values[math.ceil(len(values)*.95)-1],max_ms=max(values))
    return result


def run(args):
    with Case('qs_responsiveness',args.label) as case,running(args.binary,'table-200',case) as (box,h,c,row):
        # Measurement aliases/counters exist only in this disposable copy, never
        # the installed plugin. No timers or synthetic models drive the timings.
        source=box.root/'instrumented-plugin';shutil.copytree(args.plugin_source,source)
        path=source/'Dashboard.qml';text=path.read_text().replace('  id:root\n','  id:root\n  readonly property var perfWindow:window\n  property var perfPublish:({calls:0,ms:0})\n',1)
        method='publishNow' if '  function publishNow() {' in text else 'publish'
        # Deferred publication can run during destruction. Preserve the production
        # disposal guard before entering the extra measurement helper.
        text=text.replace('  function '+method+'() {','  function '+method+'() {if(disposing)return;var at=Date.now();try{return publishMeasured()}finally{perfPublish.calls++;perfPublish.ms+=Date.now()-at}}\n  function publishMeasured() {',1);path.write_text(text)
        path=source/'Service.qml';text=path.read_text().replace('  id: root\n','  id: root\n  property int perfSyncRows:0\n  property int perfFrames:0\n',1)
        text=text.replace('  function syncRows() {','  function syncRows() {\n    perfSyncRows++')
        text=text.replace('  function applyFrame(frame, request) {','  function applyFrame(frame, request) {\n    perfFrames++');path.write_text(text)
        q=PerformanceSession(case,box,recipe='responsiveness',source_link=source)
        keyboard=subprocess.Popen(['wtype','-s','600000'],env=q.env)
        result=dict(label=args.label,git=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
                    kernel=platform.release(),cpu=platform.processor(),load_before=os.getloadavg(),
                    renderer='private Sway pixman / Qt software',binary=str(args.binary),rows=200,samples=[],
                    pressure_before=pressure(),plugin_sha256=hashlib.sha256(b''.join(p.relative_to(args.plugin_source).as_posix().encode()+p.read_bytes() for p in sorted(args.plugin_source.rglob('*')) if p.is_file())).hexdigest())
        def state():return q.state().get('view') or {}
        def sample(name,spec,action,external=False):
            old=len(json.loads(q.ipc('samples')))
            assert q.ipc('arm',name,json.dumps(spec))=='true'
            cpu_before=process_sample(q.child.pid);at=time.time()*1000;action()
            q.wait(lambda:q.ipc('done')=='true',timeout=8)
            cpu_after=process_sample(q.child.pid)
            found=json.loads(q.ipc('samples'))[old:]
            for item in found:
                item['gui_ms']=item['ms']
                if external:item['ms']=item['frame_at']-at
                item.update(shell_cpu_ms=1000*(cpu_after['cpu']-cpu_before['cpu']),major_faults=cpu_after['major_faults']-cpu_before['major_faults'])
                result['samples'].append(item)
                print(item['name'],round(item['ms'],1),flush=True)
        def key(key,mods=0):q.ipc('key',key,mods)
        child=None
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.compositor.ipc('output * mode 1600x900')
            def details():
                q.ipc('focus');key(47);key(85,0x02000000);key(73,0x02000000)
                q.wait(lambda:state().get('rows')==1);key(0x01000004);key(0x01000000)
                q.wait(lambda:state().get('rows')==200 and state().get('cursor')==row['hash']);chosen=row['hash']
                sample('details_open',dict(kind='detail',hash=chosen,tab='overview',data=True),lambda:key(0x01000004))
                for _ in range(args.runs):
                    for tab in ('files','peers','trackers','pieces','log','overview'):
                        sample('details_'+tab,dict(kind='detail',hash=chosen,tab=tab,data=True),lambda tab=tab:q.ipc('tab',tab))
            def interactions():
                for _ in range(args.runs):
                    sample('dashboard_open',dict(kind='open',count=200),lambda:q.shell('summon','{"view":"torrents"}'),external=True)
                    q.wait(lambda:state().get('rows')==200)
                    q.ipc('hide');q.wait(lambda:q.state()['view'] is None)
                sample('popup_open',dict(kind='popup'),lambda:q.ipc('popup'))
                for _ in range(args.runs-1):
                    q.ipc('closePopup');sample('popup_open',dict(kind='popup'),lambda:q.ipc('popup'))
                q.ipc('closePopup');q.shell('summon','{"view":"torrents"}');q.wait(lambda:state().get('rows')==200)
                q.ipc('rememberTable')
                for _ in range(args.runs):
                    for name,code in [('search',50),('shows',51),('rss',52),('torrents',49),('settings',44)]:
                        sample('view_'+name,dict(kind='view',view=name),lambda code=code:key(code))
                        if name=='search':key(0x01000000) # Leave the query before global view keys.
                        if args.verify and name=='search':
                            check=json.loads(q.ipc('tableState'));assert not check['visible'] and not check['topics'],check
                        if args.verify and name=='torrents':assert json.loads(q.ipc('tableState'))['same']
                        if name=='settings':key(0x01000000)
                    sample('view_log',dict(kind='view',view='log'),lambda:key(76,0x02000000))
                    key(49);q.wait(lambda:state().get('window',{}).get('view')=='torrents')
                details()
                key(0x01000000)
                for _ in range(args.runs):
                    key(47)
                    key(85,0x02000000)
                    sample('filter_key',dict(kind='filter',text='UI',count=1),lambda:key(73,0x02000000))
                    key(0x01000000)
                key(50);q.wait(lambda:state().get('window',{}).get('view')=='search')
                began=time.monotonic();q.ipc('focus')
                q.wait(lambda:not json.loads(q.ipc('focusState'))['pending'],timeout=3)
                result['focus_settle_ms']=(time.monotonic()-began)*1000
                if args.verify:assert json.loads(q.ipc('focusState'))['active']
                for _ in range(args.runs):
                    q.ipc('focus')
                    sample('search_key',dict(kind='search',text='x'),lambda:key(88))
                    key(0x01000003)
            if args.settings_only:
                q.shell('summon','{"view":"torrents"}')
                q.wait(lambda:state().get('rows')==200)
                for _ in range(args.runs):
                    sample('view_settings',dict(kind='view',view='settings'),lambda:key(44))
                    key(0x01000000)
            elif args.details_only:
                q.shell('summon','{"view":"torrents"}')
                q.wait(lambda:state().get('rows')==200);details()
            elif args.handler_only:
                q.shell('summon','{"view":"search"}')
                q.wait(lambda:state().get('window',{}).get('view')=='search')
            else:interactions()
            if not args.details_only and not args.settings_only:
                # Real handler invocation, including CLI + shell summon overhead.
                link=Path(box.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents';link.parent.mkdir(parents=True);link.symlink_to(q.config/'Plugin',target_is_directory=True)
                shim=box.root/'shims/omarchy-shell'
                shim.write_text('#!/bin/sh\nexec /usr/share/omarchy/bin/omarchy-shell "$@"\n');shim.chmod(0o755)
                for i in range(args.runs*(2 if args.handler_only else 1)):
                    cold=args.handler_only and i>=args.runs
                    if cold:
                        q.ipc('hide');q.wait(lambda:q.state()['view'] is None)
                    hash_=f'{i+1:040x}';name='Responsiveness '+str(i)
                    magnet='magnet:?xt=urn:btih:'+hash_+'&dn=Responsiveness%20'+str(i)
                    child=None
                    def handler():
                        nonlocal child
                        child=subprocess.Popen([str(args.binary),'add','--origin','handler',magnet,'--json'],env=q.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                    sample('magnet_review_cold' if cold else 'magnet_review',dict(kind='review',name=name),handler,external=True)
                    key(0x01000000);q.wait(lambda:not (state().get('add') or {}).get('visible'))
                    out,err=child.communicate(timeout=8);assert child.returncode==0,(out,err)
                    assert hash_ not in [t['hash'] for t in c.call('torrents.list',{})['result']['torrents']]
            if not args.handler_only and not args.details_only and not args.settings_only:
                q.ipc('hide');time.sleep(1)
                before=c.call('torrents.get',dict(hash=row['hash']))['result']['torrent']
                result['bar_downloading']=measure_cpu(q,args.seconds)
                after=c.call('torrents.get',dict(hash=row['hash']))['result']['torrent']
                result['transfer_samples']=[before,after]
                assert after['completed_bytes']>before['completed_bytes'],result['transfer_samples']
                c.call('torrents.stop',dict(hashes=[row['hash']]))
                time.sleep(1);result['bar_idle']=measure_cpu(q,args.seconds)
            result.update(timings=summarize(result['samples']),load_after=os.getloadavg(),pressure_after=pressure(),
                          shell_memory=[line for line in Path(f'/proc/{q.child.pid}/status').read_text().splitlines() if line.startswith(('VmHWM:','VmRSS:'))])
            args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(result,indent=2)+'\n')
            print(json.dumps({k:v for k,v in result.items() if k not in ('samples','transfer_samples')},indent=2),flush=True)
        except BaseException:
            print('Last state:',q.state(),flush=True);traceback.print_exc();raise
        finally:
            if child is not None and child.poll() is None:
                child.terminate()
                try:child.communicate(timeout=3)
                except subprocess.TimeoutExpired:child.kill();child.communicate(timeout=3)
            keyboard.terminate();keyboard.wait(timeout=3);q.close()
    return suite_status('qs_responsiveness')


if __name__=='__main__':
    import private_net
    private_net.enter()
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--plugin-source',type=Path,default=ROOT/'plugin/oliver.torrents');p.add_argument('--label',default='baseline');p.add_argument('--runs',type=int,default=20);p.add_argument('--seconds',type=float,default=30);p.add_argument('--verify',action='store_true')
    group=p.add_mutually_exclusive_group();group.add_argument('--handler-only',action='store_true');group.add_argument('--details-only',action='store_true');group.add_argument('--settings-only',action='store_true')
    p.add_argument('--output',type=Path,required=True);args=p.parse_args();args.binary=args.binary.resolve()
    raise SystemExit(run(args))
