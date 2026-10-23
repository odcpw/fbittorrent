#!/usr/bin/env python3
"""W-001..016: actual popup keys against an owned daemon/private compositor."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
from visual import Capture,ROOT
from visual_fixture import running
import fixtures
from fbtlog import Case,suite_status
from open_support import allow_open,add_text_torrent,calls,launcher_calls,blocked_payload_error

SUITE='qs_widget'
# Preserve failures before fbtlog initializes this run's suite.
old=ROOT/'target/test-logs'/SUITE
if old.exists():shutil.copytree(old,ROOT/'target/test-logs/qs_widget_runs'/str(time.time_ns()))
def cursor(q):return json.loads(q.ipc('cursor'))
def keys(q,args,predicate):
    before=cursor(q).get('keySerial',0)
    child=subprocess.Popen(['wtype','-s','80',*args,'-s','700'],env=q.env)
    try:return q.wait(lambda:(args[0] not in ['j','k','h','l'] or cursor(q).get('keySerial',0)>before) and predicate(),timeout=6)
    except TimeoutError as e:raise TimeoutError(json.dumps(dict(args=args,cursor=cursor(q),state=json.loads(q.ipc("state"))))) from e
    finally:child.wait(timeout=3)
def peek(client,op,args):return client.response(client.send(op,args))
def run(binary):
    with Case(SUITE,'fixture') as setup,running(binary,'paused',setup) as (sandbox,harness,client,row):
        fixtures.path_shim('xdg-open','exit 0',sandbox.root)
        fixtures.path_shim('mpv','exit 0',sandbox.root)
        fixtures.path_shim('wl-paste','printf %s '+"'magnet:?xt=urn:btih:"+'b'*40+"&dn=Popup%20offer'",sandbox.root)
        q=Capture(setup,sandbox,'owner')
        allow_open(sandbox,binary)
        try:
            q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open');q.wait(lambda:cursor(q)['opened']);q.ipc('expand','paused')
            q.wait(lambda:cursor(q)['rows']==1)
            with Case(SUITE,'w_001') as c:
                keys(q,['j'],lambda:cursor(q)['active']);c.check(cursor(q)['hints'],'first movement reveals key hints',q.ipc('cursor'))
            with Case(SUITE,'w_002') as c:
                keys(q,['k'],lambda:cursor(q)['section']=='header');keys(q,['j'],lambda:cursor(q)['section']=='rows');c.check(cursor(q)['row']==0,'j/k cross header and first row',q.ipc('cursor'))
            with Case(SUITE,'w_003') as c:
                keys(q,['k'],lambda:cursor(q)['section']=='header');keys(q,['l'],lambda:cursor(q)['header']==1);keys(q,['h'],lambda:cursor(q)['header']==0);keys(q,['j'],lambda:cursor(q)['section']=='rows')
            with Case(SUITE,'w_004') as c:
                shutil.copyfile(sandbox.root/'pristine/UI fixture.bin',Path(row['content_path']))
                before=launcher_calls(sandbox)
                keys(q,['o'],lambda:blocked_payload_error(' '.join(json.loads(q.ipc('activity'))['texts'])))
                c.check(launcher_calls(sandbox)==before,'o refuses the existing .bin payload visibly without invoking an opener or player',q.ipc('activity'))
                before=len(calls(sandbox,'xdg-open'))
                keys(q,['O'],lambda:len(calls(sandbox,'xdg-open'))>before)
                folder=calls(sandbox,'xdg-open')[-1]
                c.check(folder==[str(Path(row['content_path']).parent)],'O reveals the download folder using the desktop file manager',folder)
                document,payload=add_text_torrent(sandbox,client)
                q.wait(lambda:(json.loads(q.ipc('confirmed',document.info_hash)) or {}).get('state')=='done')
                q.ipc('expand','done');q.wait(lambda:document.info_hash in cursor(q)['hashes'])
                keys(q,['j'],lambda:cursor(q)['hashes'][cursor(q)['row']]==document.info_hash)
                before=len(calls(sandbox,'xdg-open'));players=launcher_calls(sandbox)
                keys(q,['-k','Return'],lambda:len(calls(sandbox,'xdg-open'))>before)
                c.check(calls(sandbox,'xdg-open')[-1]==[str(payload)]
                        and all(launcher_calls(sandbox)[name]==players[name] for name in ('mpv','ffplay','vlc')),
                        'Enter opens a genuine safe .txt document literally without invoking a media player',calls(sandbox,'xdg-open'))
                before=len(calls(sandbox,'xdg-open'))
                keys(q,['o'],lambda:len(calls(sandbox,'xdg-open'))>before)
                c.check(calls(sandbox,'xdg-open')[-1]==[str(payload)],'lowercase o also opens the selected safe document',calls(sandbox,'xdg-open'))
                client.call('torrents.remove',dict(hashes=[document.info_hash],delete_files=False))
                q.wait(lambda:cursor(q)['hashes']==[row['hash']])
            with Case(SUITE,'w_005') as c:
                keys(q,['p'],lambda:client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']['intent']=='run')
                # Keep this row paused for the remaining confirmation tests.
                client.call('torrents.stop',dict(hashes=[row['hash']]));q.wait(lambda:json.loads(q.ipc('state'))['rows']==1)
            with Case(SUITE,'w_006') as c:
                keys(q,['u'],lambda:not cursor(q)['opened']);text=q.log.read_text()
                c.check('FBTVISUAL-SUMMON oliver.torrents '+json.dumps(dict(view='torrents',hash=row['hash']),separators=(',',':')) in text and not client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']['seed'],'u opens this torrent in Details and leaves seeding off',text);q.ipc('open');q.wait(lambda:cursor(q)['opened'])
            with Case(SUITE,'w_007') as c:
                keys(q,['d'],lambda:cursor(q)['confirming'] and cursor(q)['confirmFocused']=='cancel');keys(q,['-k','Return'],lambda:not cursor(q)['confirming']);c.check(client.call('torrents.get',dict(hash=row['hash']))['ok'],'remove strip starts Cancel-focused; Enter cancels',q.ipc('cursor'))
            with Case(SUITE,'w_008') as c:
                keys(q,['x'],lambda:cursor(q)['confirming']);keys(q,['-k','Escape'],lambda:not cursor(q)['confirming']);c.check(cursor(q)['opened'],'Esc cancels only the inline strip',q.ipc('cursor'))
            with Case(SUITE,'w_009') as c:
                keys(q,['-k','Delete'],lambda:cursor(q)['confirming']);keys(q,['y'],lambda:peek(client,'torrents.get',dict(hash=row['hash']))['ok'] is False);c.check(Path(row['content_path']).exists(),'y confirms removal and keeps payload',row['content_path'])
            with Case(SUITE,'w_010') as c:
                before=client.call('app.settings.get')['result']['alt_limits_on']
                serial=cursor(q)['keySerial']
                keys(q,['z'],lambda:cursor(q)['keySerial']>serial)
                state=json.loads(q.ipc('state'))
                c.check(client.call('app.settings.get')['result']['alt_limits_on'] is before
                        and q.ipc('clickTurtle')=='false' and 'turtle' not in state['bar'],
                        'retired z leaves settings unchanged and the popup has no mode button or bar indicator',json.dumps(state))
            with Case(SUITE,'w_011') as c:
                keys(q,['w'],lambda:not cursor(q)['opened']);q.ipc('open')
            with Case(SUITE,'w_012') as c:
                keys(q,['t'],lambda:not cursor(q)['opened']);c.check('FBTVISUAL-BLOCKED' in q.log.read_text(),'t reaches only the explicit TUI launcher; recording host blocks execution',str(q.log));q.ipc('open')
            with Case(SUITE,'w_013') as c:
                before=client.call('torrents.list',{})['result']['torrents'];keys(q,['a'],lambda:'Popup offer' in q.ipc('offer'))
                c.check(client.call('torrents.list',{})['result']['torrents']==before,'a shows dn name before any add',q.ipc('offer'))
                keys(q,['-k','Return'],lambda:len(client.call('torrents.list',{})['result']['torrents'])>len(before))
                client.call('torrents.remove',dict(hashes=['b'*40],delete_files=False))
            with Case(SUITE,'w_014') as c:
                # A fresh empty payload is incomplete, so Space has a meaningful start/stop control.
                source=sandbox.root/'space';source.mkdir();payload=source/'Space.bin';payload.write_bytes(b'x'*65536);torrent=fixtures.torrent_fixture(source,[payload]);client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),stopped=True))
                q.wait(lambda:json.loads(q.ipc('confirmed',torrent.info_hash)));q.ipc('expand','paused');q.wait(lambda:torrent.info_hash in cursor(q)['hashes'])
                before=(sandbox.root/'shims/xdg-open.log').read_text()
                if not cursor(q)['active']:keys(q,['j'],lambda:cursor(q)['active'])
                if cursor(q)['section']!='rows':keys(q,['j'],lambda:cursor(q)['section']=='rows')
                keys(q,['-k','space'],lambda:client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']['intent']=='run')
                c.check((sandbox.root/'shims/xdg-open.log').read_text()==before,'Space sends a mutation and never opens a file',q.ipc('cursor'))
            with Case(SUITE,'w_015') as c:
                keys(q,['q'],lambda:not cursor(q)['opened']);c.check(client.call('app.info')['ok'],'q closes only the popup; daemon stays alive',q.ipc('cursor'))
        finally:q.close()
    with Case(SUITE,'w_016') as c,running(binary,'downloading',c) as (sandbox,harness,client,row):
        q=Capture(c,sandbox,'owner')
        try:
            q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open');q.wait(lambda:cursor(q)['rows']>0)
            keys(q,['Q'],lambda:cursor(q)['stopping'] and cursor(q)['confirmFocused']=='cancel');keys(q,['-k','Return'],lambda:not cursor(q)['stopping'])
            c.check(client.call('app.info')['ok'],'Q offers Cancel-first shutdown; Enter cancels',q.ipc('cursor'))
            q.ipc('clickPower');q.wait(lambda:cursor(q)['stopping'] and cursor(q)['confirmFocused']=='cancel')
            keys(q,['y'],lambda:not (Path(sandbox.env['XDG_RUNTIME_DIR'])/'fbittorrent/api.sock').exists());q.wait(lambda:not json.loads(q.ipc('state'))['online']);q.ipc('close');q.wait(lambda:not cursor(q)['visible'],timeout=12)
            status=subprocess.run([str(binary),'status','--json'],env=sandbox.env,capture_output=True,text=True,timeout=5)
            c.check(json.loads(status.stdout).get('running') is False,'Stop drains daemon; status running false and icon absent after stopped-recently',status.stdout)
        finally:q.close()
    return suite_status(SUITE)
if __name__=='__main__':
    import private_net
    private_net.enter()
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);a=p.parse_args();sys.exit(run(a.binary.resolve()))
