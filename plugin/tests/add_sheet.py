#!/usr/bin/env python3
"""Actual add-sheet review/actions in private Wayland/XDG/network namespaces."""
import argparse
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
from dashboard import Session,ROOT
from visual_fixture import running
import fixtures
from fbtlog import Case,suite_status

# Preserve earlier native failures before fbtlog resets a suite.
import time
def archive():
    old=ROOT/'target/test-logs/qs_dashboard_add'
    if old.exists():shutil.copytree(old,ROOT/'target/test-logs/qs_dashboard_add_runs'/str(time.time_ns()))

def state(q):return (q.state().get('view') or {}).get('add') or {}
def wait_sheet(q,phase):return q.wait(lambda:s if (s:=state(q)).get('visible') and s.get('phase')==phase else None)
def fixture(sandbox):
    source=sandbox.root/'add source';source.mkdir()
    files=[]
    for name in ('e2.mkv','e10.mkv'):
        file=source/name;file.write_bytes((name.encode()*32768)[:65536]);files.append(file)
    return fixtures.torrent_fixture(source,files),files

def record(case,q,action):case.check(True,action,json.dumps(state(q)))

def sheet(binary,commit):
    name='add_sheet_commit' if commit else 'add_sheet_discard'
    with Case('qs_dashboard_add',name) as case,running(binary,'empty',case) as (sandbox,harness,client,row):
        torrent,files=fixture(sandbox);original={p:p.read_bytes() for p in files}
        saved=sandbox.downloads/'existing.bin';saved.write_bytes(b'keep existing download')
        fixtures.path_shim('wl-paste','printf %s '+shlex.quote(str(torrent.torrent)),sandbox.root)
        q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
            q.wait(lambda:(q.state().get('view') or {}).get('window',{}).get('visible'))
            keys=['a'] if commit else ['-M','ctrl','v','-m','ctrl']
            q.keys(keys,lambda:state(q).get('visible'));preview=wait_sheet(q,'preview')
            record(case,q,'review existing-daemon add before any write')
            case.check(client.call('torrents.list',{})['result']['torrents']==[] and not preview['trace'],'a/Ctrl+V reads the clipboard and opens review without starting a download',json.dumps(preview))
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
            if commit:
                q.keys(['-k','Return'],lambda:not state(q).get('visible'))
                result=client.call('torrents.get',dict(hash=torrent.info_hash))
                trace=state(q)['trace'];requests=[t for t in trace if t['kind']=='request']
                case.check(result['ok'] and len(requests)==1 and requests[0]['op']=='torrents.add' and 'stage' not in requests[0]['data'],'Download sends one real existing add request with no unsupported stage field',json.dumps(dict(reply=result,trace=trace)))
                case.check(result['result']['torrent']['hash']==torrent.info_hash,'the confirmed hash belongs to the actual local metainfo',json.dumps(result))
            else:
                q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
                case.check(client.call('torrents.list',{})['result']['torrents']==[] and not state(q)['trace'],'Discard from compatibility review sends no add or discard mutation',json.dumps(state(q)))
            case.check(all(p.read_bytes()==body for p,body in original.items()) and saved.read_bytes()==b'keep existing download','source payloads and existing downloads remain byte-identical',str(saved))
            record(case,q,'final sheet state and actual request/reply trace')
        finally:q.close()

def drop_routing(binary):
    with Case('qs_dashboard_add','drop_routing') as case,running(binary,'empty',case) as (sandbox,harness,client,row):
        torrent,files=fixture(sandbox);q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{}')
            q.wait(lambda:(q.state().get('view') or {}).get('window',{}).get('visible'))
            accepted=q.ipc('routeDrop',torrent.torrent.as_uri());preview=wait_sheet(q,'preview')
            case.check(accepted=='true' and preview['source']==dict(path=str(torrent.torrent)),'production drop-routing function decodes a local .torrent URL into the real sheet',json.dumps(preview))
            case.check(not preview['trace'],'drop routing has not started a download',json.dumps(preview))
            q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
            case.check(q.ipc('routeDrop','file://remote/path.torrent')=='false','non-local file URLs are rejected','false')
            fixtures.path_shim('wl-paste','exit 0',sandbox.root)
            q.keys(['a'],lambda:state(q).get('phase')=='input')
            magnet='magnet:?xt=urn:btih:'+'a'*40
            q.keys(['magnet:?xt=urn:btih:','-s','1200','a'*40,'-k','Return'],lambda:state(q).get('phase')=='preview')
            case.check(state(q)['source']==dict(magnet=magnet),'typing across the one-second service tick preserves the complete source',json.dumps(state(q)))
            q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
            case.check(True,'record manual drag qualification boundary','NOT RUN: this is routing under private Sway, not the requested manual Hyprland check')
        finally:q.close()

def handler(binary,suite='qs_dashboard_add',name='handler_confirm_mode'):
    with Case(suite,name) as case,running(binary,'empty',case) as (sandbox,harness,client,row):
        torrent,files=fixture(sandbox);q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            command=[str(binary),'add','--origin','handler',str(torrent.torrent),'--json']
            unavailable=subprocess.run(command,env=q.env,capture_output=True,text=True,timeout=12)
            case.check(unavailable.returncode==5 and client.call('torrents.list',{})['result']['torrents']==[], 'missing dashboard refuses confirmation without adding',json.dumps(dict(exit=unavailable.returncode,stdout=unavailable.stdout,stderr=unavailable.stderr)))
            link=Path(sandbox.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents';link.parent.mkdir(parents=True);link.symlink_to(q.config/'Plugin',target_is_directory=True)
            fixtures.path_shim('omarchy-shell','exec /usr/share/omarchy/bin/omarchy-shell "$@"',sandbox.root)
            command=[str(binary),'add','--origin','handler',str(torrent.torrent),'--json']
            result=subprocess.run(command,env=q.env,capture_output=True,text=True,timeout=12)
            case.check(result.returncode==0,'handler confirm routes through actual private shell CLI',json.dumps(dict(argv=command,exit=result.returncode,stdout=result.stdout,stderr=result.stderr)))
            value=json.loads(result.stdout);preview=wait_sheet(q,'preview')
            log=(sandbox.root/'shims/omarchy-shell.log').read_text()
            case.check(value['confirmation_required'] and value['count']==0 and not value['staged'] and client.call('torrents.list',{})['result']['torrents']==[],'without staging cap the handler opens review and creates no torrent record',json.dumps(dict(result=value,argv=log,state=preview)))
            case.check(preview['origin']=='handler' and preview['source']==dict(path=str(torrent.torrent)),'handler source and origin reach the actual sheet as structured data',json.dumps(preview))
            q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
            cfg=Path(sandbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml';cfg.write_text(cfg.read_text()+'\nhandler = "add"\n')
            before=(sandbox.root/'shims/omarchy-shell.log').read_bytes()
            result=subprocess.run(command,env=q.env,capture_output=True,text=True,timeout=12)
            case.check(result.returncode==0 and json.loads(result.stdout).get('hash')==torrent.info_hash,'ordinary handler add mode still creates the actual torrent immediately',result.stdout+result.stderr)
            case.check((sandbox.root/'shims/omarchy-shell.log').read_bytes()==before,'add mode does not summon confirmation',str(command))
            case.check(True,'record live staging qualification boundary','UNMET until staging capability is merged; current compatibility path is real and verified')
        finally:q.close()

if __name__=='__main__':
    import private_net
    private_net.enter()
    archive()
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--handler-only',action='store_true');p.add_argument('--ui-only',action='store_true');p.add_argument('--case',choices=['add_sheet_commit','add_sheet_discard','drop_routing','handler_confirm_mode']);a=p.parse_args();binary=a.binary.resolve()
    if a.case=='add_sheet_commit':sheet(binary,True)
    elif a.case=='add_sheet_discard':sheet(binary,False)
    elif a.case=='drop_routing':drop_routing(binary)
    elif a.case=='handler_confirm_mode':handler(binary)
    else:
        if not a.handler_only:sheet(binary,True);sheet(binary,False);drop_routing(binary)
        if not a.ui_only:handler(binary)
    sys.exit(suite_status('qs_dashboard_add'))
