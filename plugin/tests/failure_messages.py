#!/usr/bin/env python3
"""Real failure/recovery journeys on a private daemon and all four clients."""
import argparse
from contextlib import contextmanager, ExitStack
import json
from pathlib import Path
import re
import subprocess
import time

from dashboard import Session
from convergence import evidence
import daemon
import fixtures
from fbtlog import Case, suite_status
from tui_smoke import PaneUi


@contextmanager
def surfaces(binary, name):
    with Case('failure_messages', name) as case, ExitStack() as stack:
        box=stack.enter_context(daemon.Sandbox())
        h=stack.enter_context(daemon.Harness(case,binary,box,False))
        process=h.start_serve(idle_exit_secs=0)
        assert h.receive_ready() and h.wait_engine()
        client=stack.enter_context(daemon.ApiConn(h));client.hello()
        q=Session(case,box,recipe='bar-host');stack.callback(q.close)
        keyboard=subprocess.Popen(['wtype','-s','600000'],env=q.env)
        stack.callback(lambda:(keyboard.terminate(),keyboard.wait(timeout=3)))
        ui=PaneUi(case,str(binary),box,h,session='fbt-failures-'+name);stack.callback(ui.close)
        ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','120','-y','40')
        stack.enter_context(evidence(case,q,ui))
        q.wait(lambda:q.ipc('ready')=='true')
        q.shell('summon','{"view":"torrents"}')
        q.wait(lambda:(q.state().get('view') or {}).get('window'))
        yield case,box,h,client,q,ui


def cli(case,binary,box,*args):
    step=case.step('CLI '+' '.join(args),'bounded private command returns one JSON result')
    result=subprocess.run([str(binary),'--no-start',*args,'--json'],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=12)
    try:value=json.loads(result.stdout)
    except ValueError:
        step.close(False,repr((result.returncode,result.stdout,result.stderr)));raise
    step.close(True,json.dumps(dict(exit=result.returncode,result=value,stderr=result.stderr)))
    return result.returncode,value


def clean(text):
    return not re.search(r'unknown error|panicked|stack backtrace|\b(?:anyhow|bencoded|metainfo)\b|os error \d|Result<',text,re.I)


def actionable(text):
    return bool(re.search(r'\b(copy|choose|select|check|retry|restore|free|fix|download .*again|use |open |connect|update)\b',text,re.I))


def source_inputs(binary):
    with surfaces(binary,'inputs') as (case,box,h,c,q,ui):
        broken=box.root/'broken.torrent';broken.write_bytes(b'This is not a torrent.\n')
        observations=[]
        for label,source in [('invalid_magnet','magnet:?xt=urn:btih:not-a-hash'),('corrupt_torrent',str(broken)),('missing_torrent',str(box.root/'missing.torrent'))]:
            code,value=cli(case,binary,box,'add',source)
            q.compositor.ipc('[title="^fbittorrent$"] focus')
            q.keys(['a'],lambda:((q.state().get('view') or {}).get('add') or {}).get('phase')=='input')
            q.keys([source,'-k','Return'],lambda:((q.state().get('view') or {}).get('add') or {}).get('phase')=='error')
            sheet=q.state()['view']['add'];dashboard=sheet.get('error','')
            previous=ui.capture(ui.target).rstrip().splitlines()[-1]
            ui.contains('a','streaming automatic');ui.press(source,'type the failing source');ui.press('Enter','submit failing source',literal=False)
            assert ui.observe('observe the source refusal','this failed add replaces the previous result',lambda p:'streaming automatic' not in p and 'Adding…' not in p and p.rstrip().splitlines()[-1]!=previous)
            pane=ui.capture(ui.target)
            q.ipc('popup');popup=q.state()['popup']
            row=dict(kind=label,cli=value,dashboard=dashboard,tui=pane,popup=popup)
            observations.append(row)
            case.path.with_suffix('.observations.json').write_text(json.dumps(observations,indent=2))
            message=value.get('error','')+' '+value.get('hint','')
            case.check(code!=0 and not c.call('torrents.list',{})['result']['torrents'],'failed source never creates a transfer',json.dumps(row))
            case.check(clean(message) and actionable(message) and clean(dashboard) and actionable(dashboard) and clean(pane)
                       and value.get('error','') in ' '.join(pane.split()),
                       label+' explains the problem and a next action without implementation jargon',json.dumps(row))
            q.native_hook('close')
            q.compositor.ipc('[title="^fbittorrent$"] focus')
            q.keys(['-k','Escape'],lambda:not ((q.state().get('view') or {}).get('add') or {}).get('visible'))
        fixtures.path_shim('wl-paste',"printf '%s' 'magnet:?xt=urn:btih:not-a-hash'",box.root)
        q.ipc('popup');assert q.ipc('popupAdd')=='true'
        state=q.wait(lambda:s if actionable((s:=q.state())['feedback']['action']) else None)
        case.check('Invalid magnet' in state['feedback']['action'] and clean(state['feedback']['action'])
                   and any(state['feedback']['action']==text for text in state['popup']['texts']),
                   'popup invalid clipboard link offers the same actionable correction',json.dumps(state))


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--case',choices=['inputs'],default='inputs')
    parser.add_argument('--log-root',type=Path)
    args=parser.parse_args()
    if args.log_root:
        import fbtlog
        fbtlog.LOG_ROOT=args.log_root.resolve()
    source_inputs(args.binary.resolve())
    raise SystemExit(suite_status('failure_messages'))
