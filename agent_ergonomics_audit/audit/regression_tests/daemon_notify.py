"""P1-13 real daemon events with isolated recording desktop commands."""
import base64
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shlex
import socket
import struct
import sys
import time
import tomllib
import urllib.parse

import daemon as d
import fixtures


def config(box, **values):
    path = Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
    text = path.read_text()
    for key, value in values.items():
        import re
        line = key+' = '+json.dumps(value)
        text, count = re.subn(r'^'+key+r' = .*$', line, text, flags=re.M)
        if not count: text += line+'\n'
    path.write_text(text)


def shims(box, action='', delay=0):
    # The shim's logger records exact argv before this private child starts.
    program = box.root/'record-notification.py'
    state = box.root/'notification-concurrency.json'
    state.write_text('{"active":0,"peak":0,"done":0}')
    program.write_text('''import fcntl,json,sys,time,os
p=sys.argv[1]
def change(delta):
 with open(p,'r+') as f:
  fcntl.flock(f,fcntl.LOCK_EX); s=json.load(f); s['active']+=delta
  s['peak']=max(s['peak'],s['active']); s['done']+=int(delta<0);s['pid']=os.getpid()
  f.seek(0);json.dump(s,f);f.truncate()
change(1)
time.sleep(float(sys.argv[2]))
change(-1)
print(sys.argv[3],flush=True)
''')
    fixtures.path_shim('notify-send', 'exec '+shlex.join([sys.executable,str(program),str(state),str(delay),action]), box.root)
    fixtures.path_shim('xdg-open', 'exit 0', box.root)


def calls(box, name='notify-send'):
    path=box.root/'shims'/(name+'.log')
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def observe(case, box, name='notify-send'):
    rows=calls(box,name)
    artifact=case.path.with_suffix('.'+name+'.argv.json')
    artifact.write_text(json.dumps(rows,indent=2))
    case.step('record exact '+name+' argv', 'exact argv artifact retained').ok(json.dumps(rows), [artifact])
    return rows


@contextmanager
def owned(name,binary,systemd,action='',enabled=True,no_notify=False,delay=0):
    with d.Case('daemon',name) as case, d.Sandbox() as box:
        config(box,notify=enabled);shims(box,action,delay)
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(no_notify=no_notify)
            if not(h.receive_ready() and h.wait_engine()): raise RuntimeError('private notifier daemon did not start')
            try:
                with d.ApiConn(h) as client:
                    client.hello()
                    yield case,box,h,p,client
            finally:
                if p.child.poll() is None: h.stop_serve(p)
                h.lock_free()


def complete(client,box,index=0):
    root=box.root/f'complete-{index}';root.mkdir()
    payload=root/f'A<&B {index}.bin';payload.write_bytes(os.urandom(16384))
    item=fixtures.torrent_fixture(root,[payload])
    client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(root)))
    return item,payload


def await_calls(h,box,count):
    return h.wait_for(lambda:(len(calls(box))>=count,json.dumps(calls(box))),f'{count} recorded notification calls',timeout=20)


def notify_completed(binary,systemd):
    # A real local magnet resolves BEP 9 metadata, transfers bytes and completes.
    with owned('notify_completed',binary,systemd) as (case,box,h,p,client), d.Sandbox() as seedbox, d.Harness(case,binary,seedbox) as seed:
        sp=seed.start_serve()
        if not(seed.receive_ready() and seed.wait_engine()): raise RuntimeError('private seed did not start')
        try:
            root=seedbox.downloads; payload=root/'Download<&.bin';payload.write_bytes(os.urandom(65536))
            item=fixtures.torrent_fixture(root,[payload]);info=dict(item.info);info.pop('private',None)
            import hashlib
            hash=hashlib.sha1(fixtures.bencode(info)).hexdigest()
            port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
            tracker=fixtures.bencode(dict(interval=1,peers=socket.inet_aton('127.0.0.1')+struct.pack('!H',port)))
            with fixtures.HttpFixture(root,{'/announce':dict(base64=base64.b64encode(tracker).decode())}) as http:
                item.torrent.write_bytes(fixtures.bencode(dict(info=info,announce=http.base+'/announce')))
                with d.ApiConn(seed) as source:
                    source.hello();source.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(root),seed=True))
                    seed.wait_for(lambda:((row:=source.call('torrents.get',dict(hash=hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'private seed verifies local bytes')
                    magnet='magnet:?'+urllib.parse.urlencode(dict(xt='urn:btih:'+hash,tr=http.base+'/announce'))
                    client.call('torrents.add',dict(source=dict(magnet=magnet),save_path=str(box.downloads)))
                    await_calls(h,box,1)
                    actual=box.downloads/payload.name
                    case.check(actual.is_file() and actual.read_bytes()==payload.read_bytes(),'real magnet completes with exact source bytes',str(actual))
                    rows=observe(case,box)
                    case.check(len(rows)==1 and rows[0][-2:]==['Download complete','Download&lt;&amp;.bin'] and
                               all(value in rows[0] for value in ['default=Open','open=Open File','folder=Open Folder']),
                               'one completion has default, open and folder actions and escaped markup',json.dumps(rows))
                    epoch=client.call('app.info')['result']['engine_epoch']
                    client.call('app.restart',dict(scope='engine'))
                    h.wait_for(lambda:((info:=client.call('app.info')['result'])['engine_epoch']>epoch and info['state']=='running',json.dumps(info)),'completed ledger restores under a new engine epoch')
                    time.sleep(1)
                    case.check(len(calls(box))==1,'restoring a completed record never duplicates its completion notification',str(calls(box)))
        finally:
            if sp.child.poll() is None: seed.stop_serve(sp)
            seed.lock_free()


def action_case(binary,systemd,action):
    with owned('notify_action_'+action,binary,systemd,action) as (case,box,h,p,client):
        item,payload=complete(client,box)
        h.wait_for(lambda:(bool(calls(box,'xdg-open')),json.dumps(calls(box,'xdg-open'))),'chosen action opens the recorded path')
        rows=observe(case,box);opened=observe(case,box,'xdg-open')
        target=payload.parent if action=='folder' else payload
        case.check(opened==[[str(target)]] and len(rows)==1,'notification action opens exactly its file or containing directory',json.dumps(opened))


def notify_action_open(binary,systemd): action_case(binary,systemd,'open')
def notify_action_default(binary,systemd): action_case(binary,systemd,'default')
def notify_action_folder(binary,systemd): action_case(binary,systemd,'folder')


def notify_added_undo(binary,systemd):
    with owned('notify_added_undo',binary,systemd,'undo') as (case,box,h,p,client):
        root=box.root/'handler';root.mkdir();payload=root/'Keep<&.bin';payload.write_bytes(os.urandom(16384))
        item=fixtures.torrent_fixture(root,[payload]);before=payload.read_bytes()
        client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(root),stopped=True,origin='handler'))
        h.wait_for(lambda:(not client.call('torrents.list',{})['result']['torrents'],json.dumps(calls(box))),'Undo removes only the handler torrent')
        rows=observe(case,box)
        case.check(len(rows)==1 and rows[0][-2:]==['Added','Keep&lt;&amp;.bin'] and all(v in rows[0] for v in ['undo=Undo','window=Open window']) and payload.read_bytes()==before,
                   'daemon-owned Undo uses the API and leaves every payload byte intact',json.dumps(rows))


def notify_off(binary,systemd):
    for suffix,enabled,no_notify in [('config',False,False),('flag',True,True)]:
        with owned('notify_off_'+suffix,binary,systemd,enabled=enabled,no_notify=no_notify) as (case,box,h,p,client):
            complete(client,box)
            client.call('torrents.add',dict(source=dict(path=str(box.root/'absent.torrent')),origin='handler'),error='invalid_args')
            time.sleep(1.3)
            case.check(not observe(case,box),'config false or --no-notify suppresses every notification',json.dumps(calls(box)))


def notify_failed(binary,systemd):
    with owned('notify_failed',binary,systemd) as (case,box,h,p,client):
        client.call('torrents.add',dict(source=dict(path=str(box.root/'absent.torrent')),origin='handler'),error='invalid_args')
        await_calls(h,box,1);rows=observe(case,box)
        case.check(len(rows)==1 and rows[0][-2]=='Could not add' and 'log=Open log' in rows[0],'failed handler add offers its log action without leaking the source',json.dumps(rows))


def notify_queue_bounded(binary,systemd):
    with owned('notify_queue_bounded',binary,systemd,delay=.6) as (case,box,h,p,client):
        for index in range(40): complete(client,box,index)
        await_calls(h,box,40)
        state=box.root/'notification-concurrency.json'
        h.wait_for(lambda:((s:=json.loads(state.read_text()))['done']==40,json.dumps(s)),'all forty child notifications finish',timeout=20)
        rows=observe(case,box);s=json.loads(state.read_text())
        case.check(len(rows)==40 and s['peak']==4 and s['active']==0 and s['done']==40,
                   'all forty real completions drain through at most four children',json.dumps(s))


def notify_vpn_once_per_transition(binary,systemd):
    with owned('notify_vpn_once_per_transition',binary,systemd) as (case,box,h,p,client):
        began=time.monotonic()
        for interface,ready in [('fbmissing0',False),('lo',True),('fbmissing0',False)]:
            config(box,network_interface=interface)
            client.call('app.restart',dict(scope='engine'))
            h.wait_for(lambda:((value:=client.call('app.network')['result']['state'])['ready']==ready and value.get('selection')==interface,json.dumps(value)), 'new private network selection is visible')
        elapsed=time.monotonic()-began
        await_calls(h,box,2);rows=observe(case,box)
        case.check(elapsed<=2 and len(rows)==2 and rows[0][-2]=='Waiting for fbmissing0' and rows[1][-2]=='VPN back; 0 downloads resumed',
                   'three real network transitions within two seconds produce at most two named notifications',json.dumps(dict(seconds=elapsed,calls=rows)))


def notify_cli_policy(binary,systemd):
    with owned('notify_cli_policy',binary,systemd) as (case,box,h,p,client):
        root=box.root/'cli';root.mkdir();payload=root/'manual.bin';payload.write_bytes(os.urandom(16384));item=fixtures.torrent_fixture(root,[payload]);fixtures.corrupt_byte(payload,0)
        def invoke(extra):
            step=case.step('run isolated add '+shlex.join(extra),'CLI exits 0')
            child=case.popen([str(binary),'--no-start','add',str(item.torrent),'--stopped','--save',str(root),*extra],env=box.env,cwd=box.root,stdout=d.subprocess.PIPE,text=True)
            out,_=child.communicate(timeout=10);step.close(child.returncode==0,f'exit={child.returncode}; stdout={out}')
        for args in [[],['--json'],['--quiet','--notify'],['--json','--notify'],['--origin','agent','--notify']]: invoke(args)
        time.sleep(.3);case.check(not calls(box),'non-TTY, agents, JSON and quiet adds have no plain toast',json.dumps(calls(box)))
        invoke(['--notify']);await_calls(h,box,1)
        rows=observe(case,box);case.check(len(rows)==1 and '-A' not in rows[0] and '-i' in rows[0] and rows[0][rows[0].index('-i')+1]=='fbittorrent','explicit manual toast uses app icon and has no wait/actions',json.dumps(rows))


def notify_killed_handler(binary,systemd):
    with owned('notify_killed_handler',binary,systemd,'open',delay=30) as (case,box,h,p,client):
        complete(client,box)
        state=box.root/'notification-concurrency.json'
        h.wait_for(lambda:((value:=json.loads(state.read_text()))['active']==1,json.dumps(value)),'action handler is alive')
        pid=json.loads(state.read_text())['pid']
        step=case.step('kill owned fake notification handler','killed handler never starts its open action')
        os.kill(pid,9);time.sleep(.4)
        step.close(not calls(box,'xdg-open') and not Path(f'/proc/{pid}').exists(),json.dumps(dict(pid=pid,opened=calls(box,'xdg-open'))))
        observe(case,box)


def notify_handler_cli_once(binary,systemd):
    with owned('notify_handler_cli_once',binary,systemd) as (case,box,h,p,client):
        # This case tests immediate-add notifications, not handler confirmation.
        config(box,handler='add')
        for index,extra in enumerate([[],['--json'],['--quiet']]):
            root=box.root/f'click-{index}';root.mkdir();payload=root/'clicked.bin';payload.write_bytes(os.urandom(16384))
            item=fixtures.torrent_fixture(root,[payload]);fixtures.corrupt_byte(payload,0)
            step=case.step('run private desktop handler add '+shlex.join(extra),'handler CLI exits 0')
            child=case.popen([str(binary),'--no-start','add','--origin','handler',item.torrent.as_uri(),'--stopped','--save',str(root),*extra],env=box.env,cwd=box.root,stdout=d.subprocess.PIPE,text=True)
            out,_=child.communicate(timeout=10);step.close(child.returncode==0,f'exit={child.returncode}; stdout={out}')
            if not extra: await_calls(h,box,1)
            time.sleep(.2)
            case.check(len(calls(box))==1,'one browser click has one daemon toast; JSON and quiet handlers add none',json.dumps(calls(box)))
        rows=observe(case,box)
        case.check(rows[0][-2]=='Added' and all(v in rows[0] for v in ['undo=Undo','window=Open window']),
                   'the short-lived desktop handler delegates its actioned notice to the daemon',json.dumps(rows))
