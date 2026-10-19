#!/usr/bin/env python3
"""Real destination selection, permission refusal and CLI exit 2 in private state."""
import argparse
import base64
from contextlib import contextmanager, ExitStack
import json
from pathlib import Path
import shlex
import shutil
import socket
import struct
import subprocess
import sys
import time
import tomllib
from dashboard import Session, ROOT
from add_sheet import state, wait_sheet
from visual_fixture import running
import daemon
import fixtures
from fbtlog import Case, suite_status

@contextmanager
def readonly(sandbox):
    import private_net
    private_net.require()
    directory=sandbox.downloads/'z readonly';directory.mkdir()
    (directory/'keep.txt').write_bytes(b'keep readonly fixture')
    subprocess.run(['mount','--bind',str(directory),str(directory)],check=True)
    try:
        subprocess.run(['mount','-o','remount,bind,ro',str(directory)],check=True)
        yield directory
    finally:subprocess.run(['umount',str(directory)],check=True)

def picker(q):return state(q).get('picker')
def press(case,q,keys,predicate,expect):
    q.keys(keys,predicate)
def open_picker(case,q):
    press(case,q,['-k','Tab','-k','Tab','-k','Tab'],lambda:state(q).get('field')=='choose','Tab cycles only through the modal form to Choose')
    press(case,q,['-k','Return'],lambda:picker(q) is not None,'Enter on Choose opens the native folder browser')

def cli_unwritable(binary,systemd=False):
    with Case('agent_contract','add_save_unwritable') as case,fixtures.Sandbox() as sandbox,readonly(sandbox) as path:
        source=sandbox.root/'source';source.mkdir();payload=source/'payload.bin';payload.write_bytes(b'local fixture')
        torrent=fixtures.torrent_fixture(source,[payload])
        argv=[str(binary),'add',str(torrent.torrent),'--save',str(path),'--json']
        result=subprocess.run(argv,env=sandbox.env,capture_output=True,text=True,timeout=8)
        value=json.loads(result.stdout)
        case.check(result.returncode==2 and value.get('code')==2 and value.get('code_name')=='usage' and 'not writable' in value.get('error',''),'add --save refuses a real read-only folder with typed exit 2',json.dumps(dict(argv=argv,exit=result.returncode,result=value,stderr=result.stderr)))
        case.check(not (Path(sandbox.env['XDG_RUNTIME_DIR'])/'fbittorrent/api.sock').exists(),'invalid destination is refused before starting a daemon','api.sock absent')
        case.check((path/'keep.txt').read_bytes()==b'keep readonly fixture','destination validation writes no probe or payload',str(path))

def destination_picker(binary):
    with Case('qs_destination','destination_picker') as case,running(binary,'empty',case) as (sandbox,harness,client,row),readonly(sandbox) as ro,ExitStack() as stack:
        target=sandbox.downloads/'a choice #1';target.mkdir();(target/'inner').mkdir()
        (sandbox.downloads/'.hidden').mkdir();(sandbox.downloads/'file.txt').write_text('not a folder')
        source=sandbox.root/'source';source.mkdir();payload=source/'payload.bin';payload.write_bytes(b'actual loopback payload'*7000)
        torrent=fixtures.torrent_fixture(source,[payload])
        seedbox=stack.enter_context(fixtures.Sandbox());seed=stack.enter_context(daemon.Harness(case,binary,seedbox,False))
        process=seed.start_serve();stack.callback(seed.stop_serve,process)
        if not (seed.receive_ready() and seed.wait_engine()):raise RuntimeError('private seed is not ready')
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        body=fixtures.bencode(dict(interval=1,complete=1,incomplete=0,peers=socket.inet_aton('127.0.0.1')+struct.pack('!H',port)))
        tracker=fixtures.HttpFixture(source,{'/announce':dict(base64=base64.b64encode(body).decode())});stack.callback(tracker.close)
        torrent.torrent.write_bytes(fixtures.bencode(dict(info=torrent.info,announce=tracker.base+'/announce')))
        harness.seed_harness,harness.replay_seedbox=seed,seedbox
        daemon.start_replay_seed(client,harness,dict(hash=torrent.info_hash,fixture=torrent,pristine=source))
        writable=client.call('app.free_space',dict(path=str(target)))['result']
        denied=client.call('app.free_space',dict(path=str(ro)))['result']
        missing=client.call('app.free_space',dict(path=str(sandbox.root/'missing')))['result']
        case.check(writable['writable'] and isinstance(writable.get('free_bytes'),int) and writable['free_bytes']>=0,'daemon reports real non-negative available space',json.dumps(writable))
        case.check(not denied['writable'] and missing==dict(writable=False),'read-only is refused and missing space is omitted',json.dumps(dict(readonly=denied,missing=missing)))
        invalid=client.call('app.free_space',dict(path='relative'),error='invalid_args')
        case.check(invalid['error'].get('details',{}).get('key')=='path','invalid destination names the exact argument',json.dumps(invalid))
        fixtures.path_shim('wl-paste','printf %s '+shlex.quote(str(torrent.torrent)),sandbox.root)
        q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{}')
            q.wait(lambda:(q.state().get('view') or {}).get('window',{}).get('visible'))
            press(case,q,['a'],lambda:state(q).get('phase')=='ready','a opens the actual add review')
            q.wait(lambda:state(q).get('destinationReady'))
            open_picker(case,q);q.wait(lambda:picker(q) and not picker(q).get('loading') and picker(q).get('count')==2)
            case.check(picker(q)['path']==str(sandbox.downloads) and picker(q)['names']==['a choice #1','z readonly'],'picker starts at save_path and excludes files and hidden folders',json.dumps(picker(q)))
            for field in ['address','cancel','use','address']:
                press(case,q,['-k','Tab'],lambda:picker(q) and picker(q).get('field')==field,'Tab stays inside the folder picker: '+field)
            press(case,q,['-M','shift','-k','Tab','-m','shift'],lambda:picker(q) and picker(q).get('field')=='use','Shift+Tab wraps inside the picker')
            image=case.path.with_suffix('.folder.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size)
            press(case,q,['jj'],lambda:picker(q) and picker(q).get('cursor')==1,'j selects the real read-only folder')
            press(case,q,['-k','Return'],lambda:not picker(q) and state(q).get('savePath')==str(ro),'Enter selects the highlighted path')
            q.wait(lambda:not state(q)['space'].get('pending'))
            case.check(state(q)['space']['writable'] is False and not state(q)['destinationReady'],'unwritable destination disables Download',json.dumps(state(q)))
            q.keys(['-k','Return'],lambda:state(q).get('visible'))
            case.check(not any(t['kind']=='request' and t['op']=='torrents.commit' for t in state(q)['trace']) and all(t['staged'] for t in client.call('torrents.list',{})['result']['torrents']),'Enter cannot commit an unwritable destination',json.dumps(state(q)))
            open_picker(case,q)
            press(case,q,['-k','Left'],lambda:picker(q) and picker(q).get('path')==str(sandbox.downloads),'Left navigates to the parent folder')
            q.wait(lambda:picker(q) and not picker(q).get('loading') and picker(q).get('count')==2)
            press(case,q,['j'],lambda:picker(q) and picker(q).get('cursor')==0,'j highlights the intended writable folder')
            press(case,q,['-k','Return'],lambda:not picker(q) and state(q).get('savePath')==str(target),'Enter selects a path containing spaces and #')
            q.wait(lambda:state(q).get('destinationReady'))
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size)
            press(case,q,['-k','Return'],lambda:not state(q).get('visible'),'Enter commits to the selected destination')
            def complete():
                result=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
                return result.get('state')=='done',json.dumps(result)
            if not harness.wait_for(complete,'actual local payload finishes in selected folder',timeout=15):raise RuntimeError('local transfer did not complete')
            result=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
            case.check(result['save_path']==str(target) and (target/payload.name).read_bytes()==payload.read_bytes(),'selected save_path contains the exact real downloaded bytes',json.dumps(result))
        finally:q.close()

if __name__=='__main__':
    import private_net
    private_net.enter()
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);a=p.parse_args()
    old=ROOT/'target/test-logs/qs_destination'
    if old.exists():shutil.copytree(old,ROOT/'target/test-logs/qs_destination_runs'/str(time.time_ns()))
    destination_picker(a.binary.resolve());sys.exit(suite_status('qs_destination'))
