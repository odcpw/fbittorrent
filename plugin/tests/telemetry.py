#!/usr/bin/env python3
"""Real daemon -> QML status row and attached tmux TUI, on a private compositor."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
import daemon as d
import fixtures
import native
import fbtlog
from qs_env import prepare
from headless import Headless


def statistics_journey(case,pair,ui,target,done,ipc,state):
    def complete():
        row=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        value=pair.client.call('app.stats')['result']
        return row['state']=='done' and value['all_time']['completed']>=1,json.dumps(dict(torrent=row,stats=value))
    assert pair.h.wait_for(complete,'actual transfer finishes before the durable-totals journey',timeout=20)
    saved=pair.client.call('app.stats')['result']['all_time']
    def displayed():
        value=state();return (value.get('statistics') or {}).get('all_time')==saved and '8.0 MiB' in value['sessionText'] and '8.0 MiB' in value['allTimeText'],json.dumps(value)
    assert pair.h.wait_for(displayed,'dashboard Settings displays confirmed session and all-time bytes',timeout=8)
    ui.keys(target,'L');assert ui.assert_pane(target,'session ↓ 8.0 MiB');assert ui.assert_pane(target,'1 completed')
    case.path.with_suffix('.before-restart.pane.txt').write_text(ui.capture(target))
    ui.keys(target,'Escape',literal=False);ui.keys(target,'q');assert pair.h.wait_for(lambda:(done.exists(),ui.capture(target)),'TUI detaches before the explicit daemon restart')
    pair.h.stop_serve(pair.p);pair.h.notify.close()
    replacement=pair.h.start_serve(idle_exit_secs=0);pair.h.receive_ready();pair.h.wait_engine()
    def restored():
        value=state();stats=value.get('statistics') or {}
        return stats.get('all_time')==saved and stats.get('session',{}).get('downloaded')==0 and '0 B' in value['sessionText'],json.dumps(value)
    assert pair.h.wait_for(restored,'dashboard reconnect preserves all-time totals with a fresh zero-byte session',timeout=10)
    target,done=ui.start();ui.keys(target,'L')
    assert ui.assert_pane(target,'session ↓ 0 B');assert ui.assert_pane(target,'all time ↓ 8.0 MiB');assert ui.assert_pane(target,'1 completed')
    case.path.with_suffix('.after-restart.pane.txt').write_text(ui.capture(target))
    ui.keys(target,'Escape',literal=False);ui.keys(target,'q');assert pair.h.wait_for(lambda:(done.exists(),ui.capture(target)),'second TUI detaches without stopping the daemon')
    with d.ApiConn(pair.h) as fresh:
        fresh.hello();actual=fresh.call('app.stats')['result']
        status=pair.h.cli('status')
        case.check(actual['all_time']==saved==status.get('all_time') and actual['session']['downloaded']==0,'CLI and both UIs match actual durable totals after restart',json.dumps(dict(saved=saved,actual=actual,cli=status,dashboard=state())))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("binary",type=Path);parser.add_argument("--log-root",type=Path);parser.add_argument("--feature",choices=["history","statistics"],default="history")
    args=parser.parse_args();binary=args.binary.resolve()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    with fbtlog.Case('telemetry_ui',args.feature+'_live') as case,fixtures.daemon_pair(case,binary,False,1048576 if args.feature=='statistics' else 131072) as pair:
        with d.Sandbox() as ui_box:
            d.private_environment(ui_box)
            compositor=Headless(ui_box,case)
            config,env=prepare(ui_box,Path(__file__).with_name('telemetry'))
            env.update(compositor.env,FBT_TEST_BINARY=str(binary))
            # The UI has its own HOME/config, and connects only to the fixture's runtime.
            env['XDG_RUNTIME_DIR']=pair.box.env['XDG_RUNTIME_DIR']
            log=case.path.with_suffix('.qs.log')
            stream=log.open('w');child=subprocess.Popen(['qs','--no-color','-p',str(config)],env=env,stdout=stream,stderr=subprocess.STDOUT)
            def ipc(method,*args):
                result=subprocess.run(['qs','ipc','-p',str(config),'call','telemetry',method,*map(str,args)],env=env,capture_output=True,text=True,timeout=3)
                if result.returncode:raise ValueError(result.stderr)
                return result.stdout.strip()
            def state():return json.loads(ipc('state'))
            ui=None
            try:
                def ready():
                    value=state();return value.get('online') and len(value.get('history',[]))>=3 and any(value['history']),json.dumps(value)
                assert pair.h.wait_for(ready,'production QML receives real nonzero rate samples',timeout=15)
                case.check(ipc('open','settings' if args.feature=='statistics' else 'torrents')=='true','actual dashboard opens with the shared status chart',ipc('state'))
                ui=native.AttachedTui(case,str(binary),pair.box,pair.h,'fbt-fbt-p6-03-n6s-ui')
                target,done=ui.start()
                def drawn():
                    pane=ui.capture(target);return any(c in pane.splitlines()[0] for c in '▁▂▃▄▅▆▇█'),pane
                assert pair.h.wait_for(drawn,'real attached TUI draws nonzero history without covering tabs/rates')
                pane=ui.capture(target);artifact=case.path.with_suffix('.pane.txt');artifact.write_text(pane)
                if args.feature=='statistics':
                    statistics_journey(case,pair,ui,target,done,ipc,state)
                before=state()['history'];time.sleep(1.2);after=state()['history']
                if args.feature=='history':
                    case.check(len(after)>=len(before) and after!=before and all(n>=0 for n in after),'QML history advances from pushed daemon samples',json.dumps(dict(before=before,after=after)))
                else:
                    case.check(bool(after) and all(n==0 for n in after),'fresh idle session has actual zero-rate samples after restart',json.dumps(dict(before=before,after=after)))
                image=case.path.with_suffix('.png');ipc('grab',image)
                pair.h.wait_for(lambda:(image.exists(),str(image)),'live status row image is captured')
                if not done.exists():ui.keys(target,'q')
                assert pair.h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches only the TUI')
                ipc('hide')
                with d.ApiConn(pair.h) as current:
                    current.hello();row=current.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
                    case.check(pair.h.daemon_pid()>0,'closing both feature surfaces leaves the daemon and torrent record alive',json.dumps(row))
            finally:
                if ui:ui.close()
                subprocess.run(['qs','kill','--pid',str(child.pid)],env=env,capture_output=True,timeout=3)
                try:child.wait(timeout=3)
                except subprocess.TimeoutExpired:child.kill();child.wait()
                stream.close();compositor.close()
                errors=[line for line in log.read_text().splitlines() if any(x in line for x in ['TypeError:','ReferenceError:','SyntaxError:','Cannot assign','Binding loop detected','Failed to load configuration'])]
                case.check(child.returncode==0 and not errors,'private QML smoke exits cleanly',json.dumps(dict(exit=child.returncode,errors=errors)))
    return fbtlog.suite_status('telemetry_ui')
if __name__=='__main__':sys.exit(main())
