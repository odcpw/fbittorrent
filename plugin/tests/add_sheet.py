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
            q.keys(keys,lambda:state(q).get('visible'));preview=wait_sheet(q,'ready')
            staged=client.call('torrents.list',{})['result']['torrents']
            case.check(len(staged)==1 and staged[0]['staged'] and not staged[0]['checked'] and len(preview['files'])==2 and not staged[0]['upload_enabled'],'a/Ctrl+V stages real metadata and file choices with no payload transfer or upload permission',json.dumps(preview))
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
            if commit:
                q.keys(['-k','Return'],lambda:not state(q).get('visible'))
                result=client.call('torrents.get',dict(hash=torrent.info_hash))
                trace=state(q)['trace'];requests=[t for t in trace if t['kind']=='request']
                case.check(result['ok'] and not result['result']['torrent']['staged'] and [r['op'] for r in requests]==['torrents.add','torrents.commit'] and requests[0]['data']['stage'] is True and not requests[1]['data']['seed'],'Download commits the reviewed stage exactly once with upload off',json.dumps(dict(reply=result,trace=trace)))
                case.check(result['result']['torrent']['hash']==torrent.info_hash,'the confirmed hash belongs to the actual local metainfo',json.dumps(result))
            else:
                q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
                requests=[t for t in state(q)['trace'] if t['kind']=='request']
                case.check(client.call('torrents.list',{})['result']['torrents']==[] and [r['op'] for r in requests]==['torrents.add','torrents.discard'],'Discard removes only the reviewed stage without committing it',json.dumps(state(q)))
            case.check(all(p.read_bytes()==body for p,body in original.items()) and saved.read_bytes()==b'keep existing download','source payloads and existing downloads remain byte-identical',str(saved))
        finally:q.close()

def drop_routing(binary):
    with Case('qs_dashboard_add','drop_routing') as case,running(binary,'empty',case) as (sandbox,harness,client,row):
        torrent,files=fixture(sandbox);q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{}')
            q.wait(lambda:(q.state().get('view') or {}).get('window',{}).get('visible'))
            accepted=q.ipc('routeDrop',torrent.torrent.as_uri());preview=wait_sheet(q,'ready')
            case.check(accepted=='true' and preview['source']==dict(path=str(torrent.torrent)),'production drop-routing function decodes a local .torrent URL into the real sheet',json.dumps(preview))
            rows=client.call('torrents.list',{})['result']['torrents']
            case.check(len(rows)==1 and rows[0]['staged'] and not list(sandbox.downloads.iterdir()),'drop review resolves metadata without starting a payload download',json.dumps(preview))
            q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
            case.check(q.ipc('routeDrop','file://remote/path.torrent')=='false','non-local file URLs are rejected','false')
            fixtures.path_shim('wl-paste','exit 0',sandbox.root)
            q.keys(['a'],lambda:state(q).get('phase')=='input')
            magnet='magnet:?xt=urn:btih:'+'a'*40
            q.keys(['magnet:?xt=urn:btih:','-s','1200','a'*40,'-k','Return'],lambda:state(q).get('hash')=='a'*40 and state(q).get('owned'))
            case.check(state(q)['source']==dict(magnet=magnet),'typing across the one-second service tick preserves the complete source',json.dumps(state(q)))
            q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
        finally:q.close()

def handler(binary,suite='qs_dashboard_add',name='handler_confirm_mode'):
    with Case(suite,name) as case,running(binary,'empty',case) as (sandbox,harness,client,row):
        with fixtures.Sandbox() as closed:
            manifest=Path(closed.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents/manifest.json'
            manifest.parent.mkdir(parents=True);manifest.write_text('{"kinds":["panel"]}')
            fixtures.path_shim('omarchy-shell','exit 0',closed.root)
            cfg=Path(closed.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
            cfg.write_text(cfg.read_text().replace('autostart = false','autostart = true'))
            command=[str(binary),'add','--origin','handler','magnet:?xt=urn:btih:'+'a'*40,'--json']
            for flags,env in [(['--no-start'],closed.env),([],dict(closed.env,FBITTORRENT_NO_START='1'))]:
                result=subprocess.run([*command,*flags],env=env,capture_output=True,text=True,timeout=4)
                case.check(result.returncode==4 and not (closed.root/'shims/omarchy-shell.log').exists() and not (closed.root/'shims/systemctl.log').exists(),'no-start refuses a cold handler before any launch/handoff',result.stdout+result.stderr)
            result=subprocess.run(command,env=closed.env,capture_output=True,text=True,timeout=4)
            case.check(result.returncode==0 and json.loads(result.stdout)['count']==0 and not (Path(closed.env['XDG_RUNTIME_DIR'])/'fbittorrent/api.sock').exists(),'ordinary confirmation hands off immediately without waiting for a daemon',result.stdout+result.stderr)
        torrent,files=fixture(sandbox);original={p:p.read_bytes() for p in files};q=Session(case,sandbox)
        target=sandbox.root/'handler downloads';target.mkdir()
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            command=[str(binary),'add','--origin','handler','--seed','--save',str(target),str(torrent.torrent),'--json']
            def invoke():return subprocess.run(command,env=q.env,capture_output=True,text=True,timeout=12)
            def rows():return client.call('torrents.list',{})['result']['torrents']
            def outcome(result):return json.dumps(dict(exit=result.returncode,stdout=result.stdout,stderr=result.stderr))
            unavailable=invoke()
            case.check(unavailable.returncode==5 and rows()==[], 'missing dashboard refuses confirmation without adding',outcome(unavailable))
            link=Path(sandbox.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents';link.parent.mkdir(parents=True);link.symlink_to(q.config/'Plugin',target_is_directory=True)
            fixtures.path_shim('omarchy-shell','exit 7',sandbox.root)
            unavailable=invoke()
            case.check(unavailable.returncode==5 and rows()==[],'an installed but unavailable dashboard refuses before staging',outcome(unavailable))
            counter=sandbox.root/'summon-count'
            recorder='printf x >> '+shlex.quote(str(counter))+'; exec /usr/share/omarchy/bin/omarchy-shell "$@"'
            fixtures.path_shim('omarchy-shell',recorder,sandbox.root)
            result=invoke()
            case.check(result.returncode==0 and counter.read_text()=='x','handler sends one immediate review handoff',outcome(result))
            value=json.loads(result.stdout);preview=wait_sheet(q,'ready');staged=rows()
            case.check(value['confirmation_required'] and value['count']==0 and len(staged)==1 and staged[0]['staged'] and staged[0]['state']=='paused' and not staged[0]['checked'],'the shell creates one metadata-only stage; CLI does not claim a download was added',json.dumps(dict(result=value,rows=staged,sheet=preview)))
            case.check(preview['origin']=='handler' and preview['hash']==torrent.info_hash and preview['owned'] and not preview['seed'] and preview['savePath']==str(target) and len(preview['files'])==2 and preview['neededBytes']==sum(p.stat().st_size for p in files) and preview['destinationReady'],'review keeps the destination and real file choices, with seeding off',json.dumps(preview))
            fixtures.path_shim('omarchy-shell','exit 7',sandbox.root)
            duplicate=invoke();retained=rows()
            case.check(duplicate.returncode==5 and len(retained)==1 and retained[0]['staged'] and retained[0]['hash']==torrent.info_hash,'a failed handoff never discards an existing stage',outcome(duplicate))
            fixtures.path_shim('omarchy-shell',recorder,sandbox.root)
            q.keys(['-k','Escape'],lambda:not state(q).get('visible'))
            case.check(rows()==[] and not list(target.iterdir()) and all(p.read_bytes()==body for p,body in original.items()),'Discard removes the stage and preserves every source and destination byte',str(target))
            result=invoke();wait_sheet(q,'ready')
            case.check(result.returncode==0,'a fresh review after Discard opens a new stage',outcome(result))
            q.keys(['-k','Return'],lambda:not state(q).get('visible'))
            def committed():
                current=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
                return current if not current['staged'] else None
            committed_row=q.wait(committed)
            case.check(committed_row['origin']=='handler' and not committed_row['seed'] and committed_row['save_path']==str(target) and committed_row['intent']=='run','Enter commits once with seeding off and the reviewed destination',json.dumps(committed_row))
            cfg=Path(sandbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml';cfg.write_text(cfg.read_text()+'\nhandler = "add"\n')
            source=sandbox.root/'ordinary source';source.mkdir();file=source/'ordinary.bin';file.write_bytes(b'ordinary handler mode')
            immediate=fixtures.torrent_fixture(source,[file]);command[-2]=str(immediate.torrent)
            before=(sandbox.root/'shims/omarchy-shell.log').read_bytes();result=invoke()
            case.check(result.returncode==0 and json.loads(result.stdout).get('hash')==immediate.info_hash and not client.call('torrents.get',dict(hash=immediate.info_hash))['result']['torrent']['staged'],'ordinary handler add mode still creates the actual torrent immediately',outcome(result))
            case.check((sandbox.root/'shims/omarchy-shell.log').read_bytes()==before,'add mode does not summon confirmation',str(command))
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
