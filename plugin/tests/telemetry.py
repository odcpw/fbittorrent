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


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("binary",type=Path);parser.add_argument("--log-root",type=Path)
    args=parser.parse_args();binary=args.binary.resolve()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    with fbtlog.Case('telemetry_ui','history_live') as case,fixtures.daemon_pair(case,binary,False,131072) as pair:
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
                case.check(ipc('open','torrents')=='true','actual dashboard opens with the shared status chart',ipc('state'))
                ui=native.AttachedTui(case,str(binary),pair.box,pair.h,'fbt-fbt-p6-03-n6s-ui')
                target,done=ui.start()
                def drawn():
                    pane=ui.capture(target);return any(c in pane.splitlines()[0] for c in '▁▂▃▄▅▆▇█'),pane
                assert pair.h.wait_for(drawn,'real attached TUI draws nonzero history without covering tabs/rates')
                pane=ui.capture(target);artifact=case.path.with_suffix('.pane.txt');artifact.write_text(pane)
                before=state()['history'];time.sleep(1.2);after=state()['history']
                case.check(len(after)>=len(before) and after!=before and all(n>=0 for n in after),'QML history advances from pushed daemon samples',json.dumps(dict(before=before,after=after)))
                image=case.path.with_suffix('.png');ipc('grab',image)
                pair.h.wait_for(lambda:(image.exists(),str(image)),'live status row image is captured')
                ui.keys(target,'q');assert pair.h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches only the TUI')
                ipc('hide');case.check(pair.p.child.poll() is None,'closing both feature surfaces leaves transfers running',json.dumps(pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']))
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
