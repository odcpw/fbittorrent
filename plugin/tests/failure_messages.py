#!/usr/bin/env python3
"""Real failure/recovery journeys on a private daemon and all four clients."""
import argparse
import base64
import socket
import tomllib
import hashlib
from contextlib import contextmanager, ExitStack
import json
import os
import shutil
import sys
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
def surfaces(binary, name, incompatible=False):
    with Case('failure_messages', name) as case, ExitStack() as stack:
        box=stack.enter_context(daemon.Sandbox())
        h=stack.enter_context(daemon.Harness(case,binary,box,False))
        process=h.start_serve(idle_exit_secs=0);h.failure_process=process
        assert h.receive_ready() and h.wait_engine()
        client=stack.enter_context(daemon.ApiConn(h));client.hello()
        source=None
        if incompatible:
            source=box.root/'incompatible-plugin';shutil.copytree(Path(__file__).parents[1]/'oliver.torrents',source)
            client_source=source/'ApiClient.js';text=client_source.read_text();assert 'args: { proto: 1, client:' in text
            client_source.write_text(text.replace('args: { proto: 1, client:','args: { proto: 2, client:'))
        q=Session(case,box,recipe='bar-host',source_link=source);stack.callback(q.close)
        keyboard=subprocess.Popen(['wtype','-s','600000'],env=q.env)
        stack.callback(lambda:(keyboard.terminate(),keyboard.wait(timeout=3)))
        ui=PaneUi(case,str(binary),box,h,session='fbt-failures-'+name);stack.callback(ui.close)
        ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','120','-y','40')
        stack.enter_context(evidence(case,q,ui))
        q.wait(lambda:q.state()['widgets']==1 if incompatible else q.ipc('ready')=='true')
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


def destinations(binary):
    with surfaces(binary,'destinations') as (case,box,h,c,q,ui):
        folder=box.root/'read-only';folder.mkdir();folder.chmod(0o500)
        removed=box.root/'removed';removed.mkdir();removed.rmdir()
        payload=box.root/'source.bin';payload.write_bytes(b'private source'*2048)
        item=fixtures.torrent_fixture(box.root,[payload])
        try:
            (folder/'probe').write_bytes(b'owned fixture probe')
        except PermissionError:pass
        else:raise RuntimeError('fixture must provoke real EACCES before any product assertions')
        for label,path,words in [('permission',folder,('permissions','choose')),('removed',removed,('missing','restore')),('long',box.root/('x'*300),('too long','shorter'))]:
            inspected=c.call('app.free_space',dict(path=str(path)))['result']
            code,refused=cli(case,binary,box,'add',str(item.torrent),'--save',str(path))
            q.shell('summon','{"view":"settings"}')
            q.wait(lambda:(q.state().get('view') or {}).get('settings',{}).get('loaded'))
            q.compositor.ipc('[title="^fbittorrent$"] focus')
            assert q.ipc('focusSetting','save_path')=='true'
            q.keys(['-M','ctrl','a','-m','ctrl',str(path),'-k','Return'],lambda:not q.state()['settingFields']['save_path']['pending'])
            state=q.state();error=state['view']['settings']['errors'].get('save_path','')
            q.ipc('popup');state=q.state()
            case.check(not inspected['writable'] and code!=0 and all(w in (refused.get('error','')+' '+refused.get('hint','')).lower() for w in words)
                       and all(w in error.lower() for w in words) and clean(error)
                       and any(error==text for text in state['popup']['texts']),
                       label+' destination is refused with an actionable CLI/dashboard/popup explanation',json.dumps(dict(inspect=inspected,cli=refused,state=state)))
            q.native_hook('close')
            assert ui.contains('F6','Simultaneous downloads',literal=False)
            for _ in range(9):ui.press('Down','select the destination setting',literal=False)
            ui.press('Right','edit the destination',literal=False);ui.press('C-a','replace the destination',literal=False)
            ui.press(str(path),'type the failing destination');ui.press('Enter','keep the destination draft',literal=False)
            ui.press('Enter','attempt to save destination',literal=False)
            assert ui.observe('destination refusal in the terminal','a failed save remains editable and explains how to recover',lambda p:all(w in p.lower() for w in words) and 'Download settings' in p)
            ui.press('Escape','cancel the refused destination draft',literal=False)
            # Correct the same field through the real dashboard control.
            q.compositor.ipc('[title="^fbittorrent$"] focus')
            assert q.ipc('focusSetting','save_path')=='true'
            q.keys(['-M','ctrl','a','-m','ctrl',str(box.downloads),'-k','Return'],lambda:not q.state()['settingFields']['save_path']['pending'] and not q.state()['view']['settings']['errors'].get('save_path'))
            case.check(c.call('app.settings.get')['result']['save_path']==str(box.downloads),'choosing a writable folder clears the failed setting without losing other settings',str(path))
        code,added=cli(case,binary,box,'add',str(item.torrent),'--stopped')
        case.check(code==0 and added.get('hash')==item.info_hash,'a corrected destination permits the real add',json.dumps(added))


def configuration(binary):
    with surfaces(binary,'config') as (case,box,h,c,q,ui):
        path=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml';original=path.read_text()
        q.ipc('popup')
        try:
            for bad in ['upload = "PRIVATE_VALUE_MUST_NOT_BE_ECHOED\n','max_active_downlods = 2\n']:
                path.write_text(original+'\n'+bad)
                code,value=cli(case,binary,box,'status')
                case.check(code!=0 and 'config.toml' in value.get('error','') and actionable(value['error'])
                           and 'PRIVATE_VALUE_MUST_NOT_BE_ECHOED' not in json.dumps(value) and clean(value['error']),
                           'a real config typo names the file and correction without echoing its contents',json.dumps(value))
                state=q.wait(lambda:s if 'config.toml' in ' '.join((s:=q.state())['popup']['texts']) else None,timeout=8)
                case.check(any('config.toml' in text and ('Fix' in text or 'retry' in text) for text in state['renderedWindow']),
                           'dashboard renders the same live config warning as the popup',json.dumps(state))
                assert ui.observe('live config warning','the terminal explains a broken config while retaining last valid settings',lambda p:'config.toml' in p and ('Fix' in p or 'retry' in p),timeout=8)
                path.write_text(original)
                q.wait(lambda:not any('config.toml' in text for text in q.state()['popup']['texts']),timeout=8)
                assert ui.observe('config recovery','fixing the file clears the warning without restart',lambda p:'config.toml' not in p,timeout=8)
                code,value=cli(case,binary,box,'status');case.check(code==0 and value['running'],'correcting config restores ordinary CLI status',json.dumps(value))
        finally:path.write_text(original)


def protocol(binary):
    with surfaces(binary,'protocol',incompatible=True) as (case,box,h,c,q,ui):
        q.ipc('popup')
        state=q.wait(lambda:s if (s:=q.state())['feedback']['error'] else None)
        message=state['feedback']['error']
        case.check(not state['online'] and 'API' in message and 'setup --plugin=copy' in message and actionable(message)
                   and any(message==text for text in state['popup']['texts']) and any(message==text for text in state['renderedWindow']),
                   'real daemon rejects an incompatible plugin hello and both GUI surfaces retain the update action',json.dumps(state))
        time.sleep(.2)
        case.check(q.state()['feedback']['error']==message,'disconnect does not erase the protocol refusal',json.dumps(q.state()))
        code,value=cli(case,binary,box,'status')
        case.check(code==0 and value['running'] and 'Torrents' in ui.capture(ui.target),
                   'matching CLI and TUI remain healthy; plugin refusal never claims the daemon stopped',json.dumps(value))


def row_message(case,binary,box,h,c,q,ui,hash_,words):
    q.shell('summon',json.dumps(dict(view='torrents',hash=hash_)))
    q.compositor.ipc('[title="^fbittorrent$"] focus')
    q.keys(['-k','Return'],lambda:(q.state()['view'].get('details') or {}).get('visible'))
    ui.press('Home','select the failing record',literal=False);assert ui.contains('Enter','[Overview]',literal=False)
    q.ipc('popup')
    state=q.wait(lambda:s if all(w in ' '.join((s:=q.state())['popup']['texts']).lower() for w in words) else None)
    assert ui.observe('actionable torrent problem','the terminal explains the same failure and recovery',lambda p:all(w in p.lower() for w in words),timeout=5)
    code,value=cli(case,binary,box,'show',hash_)
    text=value.get('hint','')+' '+value['torrent'].get('error','')
    case.check(code==0 and all(w in text.lower() for w in words) and clean(text)
               and all(w in ' '.join(state['renderedWindow']).lower() for w in words),
               'CLI and both rendered GUI surfaces explain the same problem',json.dumps(dict(cli=value,state=state,pane=ui.capture(ui.target))))


def metadata(binary):
    with surfaces(binary,'metadata') as (case,box,h,c,q,ui):
        hash_='c'*40;source='magnet:?xt=urn:btih:'+hash_+'&dn=Peerless%20fixture&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce&x.pe=127.0.0.1:1'
        q.shell('summon',json.dumps(dict(view='torrents',add=dict(source=dict(magnet=source)))))
        q.wait(lambda:((q.state().get('view') or {}).get('add') or {}).get('owned'))
        ui.observe('peerless stage is visible','the row is visibly awaiting metadata',lambda p:'Peerless fixture' in p)
        started=time.monotonic()
        def timed_out():
            state=q.state();sheet=(state.get('view') or {}).get('add') or {}
            return state if sheet.get('phase')=='metadata_timeout' else None
        state=q.wait(timed_out,timeout=96)
        assert h.wait_for(lambda:((row:=c.call('torrents.get',dict(hash=hash_))['result']['torrent']).get('reason')=='metadata_timeout',json.dumps(row)),'engine reports the real no-peer deadline',timeout=5)
        row=c.call('torrents.get',dict(hash=hash_))['result']['torrent']
        q.ipc('popup')
        def rendered_timeout():
            state=q.state()
            return state if any('retry later' in t.lower() for t in state['renderedWindow']) else None
        state=q.wait(rendered_timeout,timeout=3)
        code,value=cli(case,binary,box,'show',hash_)
        assert ui.contains('Enter','retry',literal=False)
        case.check(code==0 and 'retry' in value.get('hint','').lower() and any('retry later' in t.lower() for t in state['renderedWindow']) and any('retry' in t.lower() for t in state['popup']['texts']),'real timeout explains recovery on all surfaces',json.dumps(dict(cli=value,state=state)))
        ui.press('Escape','close details',literal=False)
        case.check(row.get('reason')=='metadata_timeout' and row['staged'] and not list(box.downloads.iterdir()),
                   'the real metadata deadline retains an uncommitted stage and creates no payload',json.dumps(dict(elapsed=time.monotonic()-started,row=row,state=state)))
        q.native_hook('close')
        q.keys(['-k','Escape'],lambda:not ((q.state().get('view') or {}).get('add') or {}).get('visible'))
        case.check(not c.call('torrents.list',{})['result']['torrents'],'Discard remains available after the real no-peer timeout','stage removed')


def trackers(binary):
    with surfaces(binary,'trackers') as (case,box,h,c,q,ui):
        with fixtures.HttpFixture(box.root,{'/announce':dict(status=503,body='Unavailable')}) as http:
            payload=box.root/'Tracker fixture.bin';payload.write_bytes(os.urandom(65536))
            item=fixtures.torrent_fixture(box.root,[payload],tracker=http.base+'/announce')
            c.call('torrents.add',dict(source=dict(path=str(item.torrent))))
            assert h.wait_for(lambda:(bool(rows:=c.call('torrents.trackers',dict(hash=item.info_hash))['result']['trackers']) and all(r['status']=='error' for r in rows),json.dumps(rows)),
                              'every real local tracker announce returns HTTP 503',timeout=8)
            row_message(case,binary,box,h,c,q,ui,item.info_hash,('all trackers failed','retry'))
            http.routes['/announce']=dict(base64=__import__('base64').b64encode(fixtures.bencode(dict(interval=1,peers=b''))).decode())
            c.call('torrents.reannounce',dict(hashes=[item.info_hash]))
            assert h.wait_for(lambda:(bool(rows:=c.call('torrents.trackers',dict(hash=item.info_hash))['result']['trackers']) and all(r['status']=='ok' for r in rows),json.dumps(rows)),
                              'retry reaches the repaired real tracker',timeout=8)
            q.wait(lambda:not any('All trackers failed' in text for text in q.state()['popup']['texts']),timeout=5)
            assert ui.observe('tracker recovery clears terminal warning','a successful announce clears the old error without reopening Details',lambda p:'All trackers failed' not in p,timeout=5)
            case.check(not any('All trackers failed' in text for text in q.state()['renderedWindow']),'dashboard warning clears while its Details remain open','successful tracker announce')
            case.check(not c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'].get('error'),
                       'tracker recovery clears the problem without fabricating a download','no peers remain honestly stalled')


@contextmanager
def seed_fixture(case,binary,box,size=2*1024*1024):
    with daemon.Sandbox() as seedbox,daemon.Harness(case,binary,seedbox,False) as seed,ExitStack() as stack:
        payload=seedbox.downloads/'Failure fixture.bin';payload.write_bytes(os.urandom(size))
        response=lambda peers:dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=peers))).decode())
        http=stack.enter_context(fixtures.HttpFixture(box.root,{'/announce':response(b'')}))
        item=fixtures.torrent_fixture(box.root,[payload],tracker=http.base+'/announce')
        process=seed.start_serve(idle_exit_secs=0);assert seed.receive_ready() and seed.wait_engine()
        client=stack.enter_context(daemon.ApiConn(seed));client.hello()
        client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(seedbox.downloads),seed=True))
        assert seed.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'private seed verifies its actual payload')
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        http.routes['/announce']=response(socket.inet_aton('127.0.0.1')+port.to_bytes(2,'big'))
        client.call('transfer.set_limits',dict(upload_bps=1024*1024))
        yield item,payload,client


def disk(binary,dashboard=False):
    with surfaces(binary,'disk_dashboard' if dashboard else 'disk') as (case,box,h,c,q,ui):
        subprocess.run(['mount','-t','tmpfs','-o','size=4M,mode=700','tmpfs',str(box.downloads)],check=True,env=box.env,cwd=box.root,timeout=5)
        try:
            filler=box.downloads/'owned-filler';filler.write_bytes(b'F'*(3*1024*1024))
            with seed_fixture(case,binary,box) as (item,payload,peer):
                c.call('torrents.add',dict(source=dict(path=str(item.torrent))))
                assert h.wait_for(lambda:((row:=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']).get('reason')=='disk_full',json.dumps(row)),'real tmpfs ENOSPC is reported',timeout=20)
                row_message(case,binary,box,h,c,q,ui,item.info_hash,('disk full','free space','retry'))
                filler.unlink()
                # This rise is below the automatic retry threshold: use the actual terminal Retry action.
                if dashboard:
                    q.native_hook('close')
                    assert q.ipc('retryTorrent')=='true'
                else:ui.press('p','retry after freeing space',literal=False)
                assert h.wait_for(lambda:((row:=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='done',json.dumps(row)),'Retry completes the real payload after space is freed',timeout=20)
                result=box.downloads/payload.name
                case.check(result.read_bytes()==payload.read_bytes() and row_uploaded(c,item.info_hash)==0,'retry preserves payload integrity and zero upload',str(result))
        finally:
            # Stop the registered owner before dismantling its filesystem.
            h.stop_serve(h.failure_process)
            subprocess.run(['umount',str(box.downloads)],check=True,env=box.env,cwd=box.root,timeout=5)


def row_uploaded(client,hash_):
    return client.call('torrents.get',dict(hash=hash_))['result']['torrent'].get('uploaded_bytes',0)


def duplicates(binary):
    with surfaces(binary,'duplicates') as (case,box,h,c,q,ui),seed_fixture(case,binary,box,16*1024*1024) as (item,payload,peer):
        c.call('torrents.add',dict(source=dict(path=str(item.torrent))))
        assert h.wait_for(lambda:((row:=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='downloading',json.dumps(row)),'fixture is actually running')
        for completed in (False,True):
            if completed:
                peer.call('transfer.set_limits',dict(upload_bps=0))
                assert h.wait_for(lambda:((row:=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='done',json.dumps(row)),'fixture actually completes',timeout=30)
            code,value=cli(case,binary,box,'add',str(item.torrent))
            human=subprocess.run([str(binary),'--no-start','add',str(item.torrent)],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=12)
            case.check(human.returncode==0 and 'already in your list' in human.stdout.lower(),'plain CLI duplicate says already present rather than added',human.stdout)
            q.shell('summon',json.dumps(dict(view='torrents',add=dict(source=dict(path=str(item.torrent))))))
            state=q.wait(lambda:s if (((s:=q.state()).get('view') or {}).get('add') or {}).get('duplicate') else None)
            q.compositor.ipc('[title="^fbittorrent$"] focus')
            q.keys(['-k','Escape'],lambda:not q.state()['view']['add']['visible'])
            ui.contains('a','streaming automatic');ui.press(str(item.torrent),'add the same source');ui.press('Enter','confirm duplicate add',literal=False)
            assert ui.observe('duplicate add stays truthful','terminal says already in list rather than added',lambda p:'Already in your list' in p)
            fixtures.path_shim('wl-paste',"printf '%s' "+__import__('shlex').quote(item.magnet),box.root)
            q.ipc('popup');q.ipc('popupAdd')
            state=q.wait(lambda:s if 'already' in (s:=q.state())['feedback']['action'].lower() else None)
            case.check(code==0 and value.get('duplicate') and 'existing' in value.get('hint','') and len(c.call('torrents.list',{})['result']['torrents'])==1 and any('already' in t.lower() for t in state['popup']['texts']),
                       'duplicate while '+('completed' if completed else 'running')+' has one record and an honest result on every surface',json.dumps(dict(cli=value,state=state)))
            q.native_hook('close')


def vpn(binary):
    with surfaces(binary,'vpn') as (case,box,h,c,q,ui):
        c.call('app.settings.set',dict(network_interface='fbmissing0'))
        payload=box.root/'VPN fixture.bin';payload.write_bytes(os.urandom(65536))
        item=fixtures.torrent_fixture(box.root,[payload])
        c.call('torrents.add',dict(source=dict(path=str(item.torrent))))
        q.ipc('popup')
        state=q.wait(lambda:s if all(w in ' '.join((s:=q.state())['popup']['texts']).lower() for w in ('fbmissing0','settings')) else None)
        assert ui.observe('VPN wait explains its remedy','missing interface is named with a settings action',lambda p:'fbmissing0' in p and 'Settings' in p,timeout=8)
        code,value=cli(case,binary,box,'status')
        case.check(code==0 and value['network']['state']=='waiting' and 'Settings' in value['hint'] and all(w in ' '.join(state['renderedWindow']) for w in ('fbmissing0','Settings')) and not list(box.downloads.iterdir()),'missing VPN is consistent and never falls back to another interface',json.dumps(dict(cli=value,state=state)))
        c.call('app.settings.set',dict(network_interface='lo'))
        assert h.wait_engine()
        q.wait(lambda:not any('fbmissing0' in t for t in q.state()['popup']['texts']),timeout=8)
        case.check(cli(case,binary,box,'status')[1]['network']['state']=='ready','choosing the actual private interface clears the wait','loopback only')


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--case',choices=['inputs','destinations','config','protocol','metadata','trackers','disk','disk_dashboard','duplicates','vpn'],default='inputs')
    parser.add_argument('--log-root',type=Path)
    args=parser.parse_args()
    if args.case=='destinations' and os.environ.get('FBT_FAILURE_DAC_DROPPED')!='1':
        caps='-dac_override,-dac_read_search'
        result=subprocess.run(['setpriv','--bounding-set='+caps,'--inh-caps='+caps,'--ambient-caps='+caps,sys.executable,'-B',*sys.argv],env=dict(os.environ,FBT_FAILURE_DAC_DROPPED='1'))
        raise SystemExit(result.returncode)
    if args.log_root:
        import fbtlog
        fbtlog.LOG_ROOT=args.log_root.resolve()
    {'inputs':source_inputs,'destinations':destinations,'config':configuration,'protocol':protocol,'metadata':metadata,'trackers':trackers,'disk':disk,'disk_dashboard':lambda binary:disk(binary,True),'duplicates':duplicates,'vpn':vpn}[args.case](args.binary.resolve())
    raise SystemExit(suite_status('failure_messages'))
