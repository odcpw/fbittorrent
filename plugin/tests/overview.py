#!/usr/bin/env python3
"""Move/open and cap controls in the real dashboard/TUI, using private payloads."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys
from dashboard import Session
import daemon
import fixtures
from fbtlog import Case,suite_status
from open_support import allow_open

def run(binary):
    with Case('qs_overview','move_and_cap') as case,daemon.Sandbox() as box,daemon.Harness(case,binary,box) as h:
        source=box.root/'source';source.mkdir();payload=source/'daemon-fixture.txt';payload.write_bytes(b'owned move fixture'*32768)
        digest=hashlib.sha256(payload.read_bytes()).hexdigest();fixture=fixtures.torrent_fixture(box.root,[payload])
        destination=box.root/'new folder # literal';destination.mkdir();collision=box.root/'collision';collision.mkdir()
        unrelated=collision/payload.name;unrelated.write_bytes(b'keep unrelated bytes')
        final=box.root/'TUI destination';final.mkdir();fixtures.path_shim('xdg-open','exit 0',box.root)
        fixtures.path_shim('mpv','exit 0',box.root)
        process=h.start_serve();h.receive_ready();h.wait_engine();q=None;tui_pid=None;socket=box.root/'ui.sock'
        def tmux(*args,check=True):return subprocess.run(['tmux','-S',str(socket),*args],env=box.env,check=check,capture_output=True,text=True,timeout=4).stdout
        try:
            with daemon.ApiConn(h) as c:
                c.hello();c.call('torrents.add',dict(source=dict(path=str(fixture.torrent)),save_path=str(source)))
                def row():return c.call('torrents.get',dict(hash=fixture.info_hash))['result']['torrent']
                assert h.wait_for(lambda:(row()['state']=='done',json.dumps(row())),'private fixture finishes verification')
                initial=row();q=Session(case,box)
                allow_open(box,binary)
                def view():return q.state().get('view') or {}
                def detail():return view().get('details') or {}
                def action():return detail().get('action') or {}
                def press(args,predicate):
                    q.compositor.ipc('[title="^fbittorrent$"] focus')
                    try:return q.keys(args,predicate)
                    except TimeoutError as error:raise TimeoutError('Overview keys '+repr(args)+': '+str(error)) from error
                q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=fixture.info_hash)))
                q.wait(lambda:view().get('cursor')==fixture.info_hash);press(['-k','Return'],lambda:view().get('focus')=='details');q.wait(lambda:detail().get('hash')==fixture.info_hash)
                press(['m'],lambda:action().get('modal')=='move' and action().get('picker'))
                press(['-k','Tab','-k','Tab'],lambda:action()['picker']['field']=='cancel')
                press(['-k','Return'],lambda:not action())
                case.check(row()['save_path']==str(source) and payload.exists(),'folder-picker Cancel activates from the keyboard without moving data','source location retained')
                def move_to(path):
                    press(['m'],lambda:action().get('modal')=='move' and action().get('picker'))
                    press(['-k','Tab'],lambda:action()['picker']['field']=='address')
                    press(['-M','ctrl','a','-m','ctrl',str(path),'-k','Return'],lambda:action()['picker']['path']==str(path))
                    press(['-k','Tab','-k','Tab','-k','Return'],lambda:not action())
                move_to(collision);q.wait(lambda:'exists' in detail().get('actionStatus','').lower())
                case.check(row()['save_path']==str(source) and hashlib.sha256(payload.read_bytes()).hexdigest()==digest and unrelated.read_bytes()==b'keep unrelated bytes','failed dashboard move preserves source, location and unrelated destination bytes',detail().get('actionStatus'))
                move_to(destination);q.wait(lambda:row()['save_path']==str(destination) and detail().get('actionStatus')=='Move complete')
                moved=destination/payload.name
                case.check(row()['state']=='done' and row()['checked'] and row()['completed_bytes']==initial['completed_bytes'] and hashlib.sha256(moved.read_bytes()).hexdigest()==digest and not payload.exists(),'dashboard move confirms the new location with complete data intact',json.dumps(row()))
                press(['o'],lambda:(box.root/'shims/xdg-open.log').exists())
                opened=json.loads((box.root/'shims/xdg-open.log').read_text().splitlines()[-1])
                case.check(opened==[str(moved)],'Open content uses the new path literally',opened)
                before=len((box.root/'shims/xdg-open.log').read_text().splitlines())
                press(['O'],lambda:len((box.root/'shims/xdg-open.log').read_text().splitlines())>before)
                folder=json.loads((box.root/'shims/xdg-open.log').read_text().splitlines()[-1])
                case.check(folder==[str(destination)],'O reveals the moved download folder literally',folder)
                c.call('app.settings.set',dict(download_limit_bps=32768,alt_download_limit_bps=16384))
                press(['c'],lambda:action().get('modal')=='limit');press(['64K','-k','Return'],lambda:not action() and row().get('download_limit_bps')==65536)
                q.wait(lambda:detail().get('overview',{}).get('effective')=='32.8 KB/s')
                case.check(row()['checked'] and row()['state']=='done','download cap changes without a recheck and displays the lower global cap',json.dumps(detail().get('overview')))
                c.call('transfer.set_alt',dict(on=True));q.wait(lambda:detail().get('overview',{}).get('effective')=='16.4 KB/s')
                press(['c'],lambda:action().get('modal')=='limit');press(['-M','ctrl','a','-m','ctrl','none','-k','Return'],lambda:not action() and 'download_limit_bps' not in row())
                q.wait(lambda:detail().get('overview',{}).get('cap')=='Unlimited')
                case.check(row()['state']=='done' and row()['checked'],'clearing the torrent cap retains the active legacy API global cap',json.dumps(detail().get('overview')))
                q.shell('hide');q.wait(lambda:q.state().get('view') is None)
                tmux('new-session','-d','-s','fbt-fbt-p7-02-ss9','-x','120','-y','40',shlex.join([str(binary),'tui']))
                tui_pid=int(tmux('display-message','-p','-t','fbt-fbt-p7-02-ss9','#{pane_pid}').strip())
                def pane():return tmux('capture-pane','-p','-t','fbt-fbt-p7-02-ss9')
                h.wait_for(lambda:('daemon-fixture.txt' in pane(),pane()),'private TUI attaches to the same owner')
                tmux('send-keys','-t','fbt-fbt-p7-02-ss9','Enter');h.wait_for(lambda:('[Overview]' in pane(),pane()),'TUI opens Overview')
                tmux('send-keys','-t','fbt-fbt-p7-02-ss9','-l','c');h.wait_for(lambda:('Download cap' in pane(),pane()),'TUI cap editor opens')
                tmux('send-keys','-t','fbt-fbt-p7-02-ss9','-l','128K');tmux('send-keys','-t','fbt-fbt-p7-02-ss9','Enter')
                assert h.wait_for(lambda:(row().get('download_limit_bps')==131072,json.dumps(row())),'TUI applies the per-torrent cap')
                assert h.wait_for(lambda:('Download cap applied' in pane(),pane()),'TUI shows the confirmed cap before the next action')
                tmux('send-keys','-t','fbt-fbt-p7-02-ss9','-l','m');h.wait_for(lambda:('Move to folder' in pane(),pane()),'TUI Move editor opens')
                tmux('send-keys','-t','fbt-fbt-p7-02-ss9','C-a');tmux('send-keys','-t','fbt-fbt-p7-02-ss9','-l',str(final));tmux('send-keys','-t','fbt-fbt-p7-02-ss9','Enter')
                assert h.wait_for(lambda:(row()['save_path']==str(final),json.dumps(row())),'TUI confirms the new destination')
                case.check(hashlib.sha256((final/payload.name).read_bytes()).hexdigest()==digest and row()['state']=='done' and row()['checked'],'TUI Move and cap use the same durable daemon operations','private payload bytes and completion retained')
                tmux('send-keys','-t','fbt-fbt-p7-02-ss9','Escape');tmux('send-keys','-t','fbt-fbt-p7-02-ss9','-l','q')
                case.check(process.child.poll() is None,'closing both interfaces leaves the owner running','same private daemon')
        finally:
            try:
                try:
                    tmux('kill-server',check=False)
                    if tui_pid:h.wait_for(lambda:(not daemon.process_alive(tui_pid),'owned TUI PID '+str(tui_pid)),'owned TUI exits after its private tmux server closes',timeout=3)
                finally:
                    if q:q.close()
            finally:
                if process.child.poll() is None:h.stop_serve(process)
    return suite_status('qs_overview')
if __name__=='__main__':
    import private_net
    private_net.enter()
    p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,required=True);a=p.parse_args();sys.exit(run(a.binary.resolve()))
