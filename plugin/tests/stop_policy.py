#!/usr/bin/env python3
"""Actual popup client: power drains the owner; glance freezes its idle clock."""
import argparse
import json
import os
import shutil
import signal
from types import SimpleNamespace
from pathlib import Path
import sys
import time
from visual import Capture,ROOT
from dashboard import Session
from desktop_setup import prepare as prepare_setup,call as setup_call
import daemon as d
import fixtures
import private_net
import fbtlog
from fbtlog import Case,suite_status
from daemon_policy import hello,subscribe,idle_exit


def stopped(case,q,dashboard,phase):
    def empty():
        state=json.loads(q.ipc('activity'))
        return state if not json.loads(q.ipc('state'))['online'] and state['model']==state['rows']==0 else None
    state=q.wait(empty)
    texts=' '.join(state['texts'])
    case.check(not json.loads(q.ipc('groups')) and state['progress']==0 and state['summary']=={}
               and state['status']['active']==state['status']['seeding']==0
               and state['status'].get('dl') is None and state['status'].get('up') is None
               and 'Stopped' in texts and 'fbittorrent stopped' in texts
               and not any(word in texts for word in ('DOWNLOADING','UPLOADING','peers','↓','↑')),
               phase+': popup renders stopped with no live groups, rates, peers or progress',json.dumps(state))
    window=dashboard.wait(lambda:value if (value:=json.loads(dashboard.ipc('stoppedState')))['visible'] and not value['table'] else None)
    case.check(window['message']=='fbittorrent stopped' and window['model']==0
               and json.loads(dashboard.ipc('renderedRows'))==[],
               phase+': already-open dashboard renders stopped and unloads its table',json.dumps(window))
    frame=state['pendingFrame'];time.sleep(1)
    case.check(json.loads(q.ipc('activity'))['pendingFrame']==frame,
               phase+': no pending progress animation while stopped',str(frame))
    screenshot=case.path.with_suffix('.'+phase+'.png');q.capture(screenshot)


def power(binary,plugin_source=None):
    with Case('plugin_policy','power') as case,fixtures.daemon_pair(case,binary,False,65536) as pair,fixtures.Sandbox() as dashboard_box:
        c=pair.client;c.call('subscribe',dict(topics=['summary'],interval_ms=250))
        # Exercise the actual embedded install, not a separate source plugin.
        prepare_setup(pair.box);setup_call(case,binary,pair.box,'--plugin=copy')
        source=Path(pair.box.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents'
        if plugin_source:
            shutil.copytree(plugin_source,source,dirs_exist_ok=True)
            print('Focused QML source overlay in private installation: '+str(plugin_source),flush=True)
        fixtures.path_shim('fbittorrent','exit 0',pair.box.root)
        dashboard_source=dashboard_box.root/'installed-plugin';shutil.copytree(source,dashboard_source)
        q=Capture(case,pair.box,'owner',source=source)
        dashboard=None
        try:
            q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open')
            q.wait(lambda:json.loads(q.ipc('cursor'))['rows']==1)
            dashboard_case=SimpleNamespace(path=case.path.with_name('power_dashboard.jsonl'),check=case.check)
            dashboard=Session(dashboard_case,dashboard_box,source_link=dashboard_source)
            dashboard.wait(lambda:dashboard.state())
            dashboard.wait(lambda:dashboard.native_hook('useRuntimeDir',pair.box.env['XDG_RUNTIME_DIR'])=='ok')
            dashboard.wait(lambda:dashboard.state()['online'])
            dashboard.shell('summon','{}')
            dashboard.wait(lambda:len(json.loads(dashboard.ipc('renderedRows')))==1)
            active=json.loads(q.ipc('activity'))
            case.check(active['progress']>0 and active['status']['active']>0
                       and any('peers' in text for text in active['texts']),
                       'real download renders live rows, progress and peer observations before stop',json.dumps(active))
            case.check(q.ipc('clickPower')=='true','click the real popup power control')
            q.wait(lambda:json.loads(q.ipc('cursor')).get('stopping'))
            cursor=json.loads(q.ipc('cursor'))
            case.check(cursor['confirmFocused']=='cancel' and cursor['selected']==0,
                       'active transfer stop confirmation defaults to Cancel',json.dumps(cursor))
            before=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent']['completed_bytes']
            case.check(q.ipc('clickStopChoice','Cancel')=='true','click the actual Cancel button')
            q.wait(lambda:not json.loads(q.ipc('cursor')).get('stopping'))
            assert pair.h.wait_for(lambda:((r:=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'])['completed_bytes']>before,json.dumps(r)), 'Cancel leaves the real download moving',timeout=5)
            case.check(q.ipc('clickPower')=='true','reopen real power confirmation')
            q.wait(lambda:json.loads(q.ipc('cursor')).get('stopping'))
            case.check(q.ipc('clickStopChoice','Stop')=='true','explicit Stop clicks the actual confirmation button')
            terminal=next(frame for frame in c.events() if frame.get('ev')=='bye')
            pair.p.wait(timeout=10,code=0)
            q.wait(lambda:not json.loads(q.ipc('state'))['online'])
            case.check(terminal.get('reason')=='shutdown' and not pair.h.engine_path.exists() and not json.loads(q.ipc('state'))['idle'],
                       'plugin power produces shutdown bye, clean daemon exit, unlinked socket and cleared countdown',json.dumps(terminal))
            pair.h.lock_free()
            stopped(case,q,dashboard,'shutdown')
            case.check(json.loads(q.ipc('activity'))['power']=='Start fbittorrent',
                       'stopped power control advertises Start')
            case.check(q.ipc('clickPower')=='true','click the stopped power control')
            launcher=pair.box.root/'shims/fbittorrent.log'
            q.wait(lambda:launcher.exists())
            calls=[json.loads(line) for line in launcher.read_text().splitlines()]
            case.check(calls==[['up']] and not json.loads(q.ipc('cursor'))['stopping']
                       and json.loads(q.ipc('activity'))['startup'],
                       'offline power requests up immediately without a Stop confirmation',json.dumps(calls))
            # The recording launcher stands in for the user manager; the harness
            # owns the real serve process it launches in this private namespace.
            pair.h.notify.close();restarted=pair.h.start_serve(idle_exit_secs=0)
            assert pair.h.receive_ready() and pair.h.wait_engine()
            q.wait(lambda:json.loads(q.ipc('state'))['online'])
            dashboard.wait(lambda:dashboard.state()['online'])
            q.wait(lambda:json.loads(q.ipc('activity'))['status']['active']>0)
            dashboard.wait(lambda:len(json.loads(dashboard.ipc('renderedRows')))==1)
            case.check(not json.loads(q.ipc('cursor'))['stopping'],
                       'requested start reconnects the plugin and restores live download rows')
            case.check(q.ipc('clickPower')=='true','online power still opens Stop confirmation after restart')
            q.wait(lambda:json.loads(q.ipc('cursor'))['stopping'])
            # No bye frame: EOF must clear observations and cancel an open Stop
            # prompt too. SIGKILL affects only this fixture's peer-credential PID.
            os.kill(pair.h.daemon_pid(),signal.SIGKILL);restarted.wait(timeout=10,code=-signal.SIGKILL)
            stopped(case,q,dashboard,'unexpected_exit')
            case.check(not json.loads(q.ipc('cursor'))['stopping'],
                       'unexpected exit cancels the stale Stop confirmation')
            pair.h.lock_free()
        finally:
            if dashboard:dashboard.close()
            q.close()


def glance(binary):
    with Case('plugin_policy','glance') as case,d.Sandbox() as box,d.Harness(case,binary,box,False) as h:
        # Keep startup unhurried. Configure the short real timer only after the
        # actual plugin is online and its popup presence has been acknowledged.
        p=h.start_serve(idle_exit_secs=30)
        assert h.receive_ready() and h.wait_engine()
        with d.ApiConn(h) as c:
            hello(c,'bar');stream,_=subscribe(c)
            q=Capture(case,box,'owner')
            try:
                q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open')
                q.wait(lambda:json.loads(q.ipc('state'))['opened'])
                c.call('app.settings.set',dict(idle_exit_secs=30))
                # The normal public setting minimum is 30 seconds. No test-only
                # production hook or shortened policy bypass is used here.
                def summary():
                    return next(f['data']['daemon'] for f in c.events() if f.get('topic')=='summary')
                began=time.monotonic();first=summary();samples=[]
                while time.monotonic()-began<31:
                    samples.append(summary())
                values=[s.get('idle_exit_in_s') for s in samples]
                case.check(p.child.poll() is None and values and all(v==values[0] for v in values) and not any(s['busy_reasons'] for s in samples),
                           'real popup glance freezes the countdown beyond the configured 30-second idle delay',json.dumps(dict(first=first,values=values,elapsed=time.monotonic()-began)))
                q.ipc('close');q.wait(lambda:not json.loads(q.ipc('state'))['opened'])
                elapsed=idle_exit(case,h,p,c,timeout=36)
                case.check(25<=elapsed<=36,'closing the popup resumes its remaining real countdown and exits idle',json.dumps(dict(elapsed=elapsed)))
                h.lock_free()
            finally:q.close()


def default_delay(binary):
    with Case('plugin_policy','default_delay') as case,d.Sandbox() as box,d.Harness(case,binary,box,False) as h:
        p=h.start_serve(idle_exit_secs=None)
        assert h.receive_ready() and h.wait_engine()
        with d.ApiConn(h) as c:
            hello(c,'bar');_,snapshot=subscribe(c)
            config=c.call('app.settings.get')['result']
            case.check(config['idle_exit_secs']==300 and 290<=snapshot.get('idle_exit_in_s',0)<=300,
                       'serve without any idle override uses the actual 300-second config default',json.dumps(dict(settings=config,snapshot=snapshot)))
            c.call('app.settings.set',dict(idle_exit_secs=29),error='invalid_args')
            c.call('app.settings.set',dict(idle_exit_secs=30))
            case.check(c.call('app.settings.get')['result']['idle_exit_secs']==30,
                       'public config rejects 29 seconds and persists its allowed 30-second minimum')
        h.stop_serve(p);h.lock_free()


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--case',choices=['power','glance','default_delay'],required=True);parser.add_argument('--log-root',type=Path)
    parser.add_argument('--plugin-source',type=Path,help='power case only: explicitly test source QML instead of the embedded copy')
    args=parser.parse_args()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    private_net.enter()
    if args.plugin_source and args.case!='power':parser.error('--plugin-source requires --case power')
    if args.case=='power':power(args.binary.resolve(),args.plugin_source.resolve() if args.plugin_source else None)
    else:globals()[args.case](args.binary.resolve())
    sys.exit(suite_status('plugin_policy'))
