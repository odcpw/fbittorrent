#!/usr/bin/env python3
"""Display-name journey through the real CLI, attached TUI and dashboard."""
import argparse, hashlib, json, os, subprocess, sys
import http.server, socket, threading, tomllib, urllib.parse
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
import daemon as d
import fixtures, native, fbtlog
from qs_env import prepare
from headless import Headless


def metadata_journey(case,pair,binary):
    # BEP 9 correctly refuses metadata for a private torrent. Create public
    # metainfo over our same random fixture bytes; all networking is bound to lo.
    item=fixtures.torrent_fixture(pair.box.root,[pair.box.downloads/'fixture-state.bin'],name='metadata')
    info=dict(item.info);info.pop('private');hash_value=hashlib.sha1(fixtures.bencode(info)).hexdigest()
    item.torrent.write_bytes(fixtures.bencode(dict(info=info)))
    with d.ApiConn(pair.h) as seed:
        seed.hello();seed.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(pair.box.downloads),seed=True))
        assert pair.h.wait_for(lambda:((r:=seed.call('torrents.get',dict(hash=hash_value))['result']['torrent'])['state']=='seeding',json.dumps(r)),'owned seed is live for metadata exchange')
    gate=threading.Event();announced=threading.Event()
    port=tomllib.loads((Path(pair.box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
    body=fixtures.bencode(dict(interval=1,peers=socket.inet_aton('127.0.0.1')+int(port).to_bytes(2,'big')))
    class Tracker(http.server.BaseHTTPRequestHandler):
        def log_message(self,*_):pass
        def do_GET(self):
            announced.set();gate.wait(10)
            self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    tracker=http.server.ThreadingHTTPServer(('127.0.0.1',0),Tracker)
    worker=threading.Thread(target=tracker.serve_forever,kwargs=dict(poll_interval=.05));worker.start()
    try:
        with d.Sandbox() as box,d.Harness(case,binary,box,False) as h:
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as client:
                client.hello();magnet='magnet:?xt=urn:btih:'+hash_value+'&dn=Provisional&tr='+urllib.parse.quote('http://127.0.0.1:'+str(tracker.server_port)+'/announce',safe='')
                client.call('torrents.add',dict(source=dict(magnet=magnet),stage=True))
                client.call('torrents.rename',dict(hash=hash_value,name='Before metadata'))
                case.check(announced.wait(5),'local metadata lookup is held until after rename','owned tracker gate')
                gate.set()
                def resolved():
                    row=client.call('torrents.get',dict(hash=hash_value))['result']['torrent']
                    return row.get('original_name')=='fixture-state.bin' and row['name']=='Before metadata',json.dumps(row)
                assert h.wait_for(resolved,'real metadata refresh preserves a pre-existing display override',timeout=15)
                client.call('torrents.rename',dict(hash=hash_value,name=''))
                row=client.call('torrents.get',dict(hash=hash_value))['result']['torrent']
                case.check(row['name']=='fixture-state.bin' and not list(box.downloads.iterdir()),'clear after metadata uses verified original without creating payloads',json.dumps(row))
            h.stop_serve(p)
    finally:
        gate.set();tracker.shutdown();tracker.server_close();worker.join(timeout=3)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('binary',type=Path);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();binary=args.binary.resolve()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    with fbtlog.Case('rename_ui','display_name_live') as case, fixtures.daemon_pair(case,binary,False,state='seeding') as pair:
        def row(client=None):return (client or pair.client).call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        original=row()['name'];payload=pair.box.downloads/'fixture-state.bin'
        def snapshot():return dict(files=[str(payload.relative_to(pair.box.downloads))],inode=payload.stat().st_ino,bytes=payload.stat().st_size,digest=hashlib.sha256(payload.read_bytes()).hexdigest())
        before=snapshot()
        case.check(pair.h.cli('rename',pair.hash,'  CLI title  ').get('ok') is True,'CLI accepts and trims a display name',json.dumps(row()))
        case.check(row()['name']=='CLI title' and row()['original_name']==original,'API projects override and preserves original metadata name',json.dumps(row()))
        pair.h.cli('rename',pair.hash,'bad\nname',code=2)
        pair.client.call('torrents.rename',dict(hash=pair.hash,name='x'*513),error='invalid_args')
        pair.client.call('torrents.rename',dict(hash='a'*40,name='Missing'),error='not_found')
        ui=native.AttachedTui(case,str(binary),pair.box,pair.h,'fbt-fbt-p7-03-6uk')
        with d.Sandbox() as box:
            d.private_environment(box);compositor=Headless(box,case)
            config,env=prepare(box,Path(__file__).with_name('rename-live'));env.update(compositor.env,FBT_TEST_BINARY=str(binary),XDG_RUNTIME_DIR=pair.box.env['XDG_RUNTIME_DIR'])
            log=case.path.with_suffix('.qs.log');stream=log.open('w');child=subprocess.Popen(['qs','--no-color','-p',str(config)],env=env,stdout=stream,stderr=subprocess.STDOUT)
            def ipc(method,*values):
                r=subprocess.run(['qs','ipc','-p',str(config),'call','rename',method,*map(str,values)],env=env,capture_output=True,text=True,timeout=3)
                if r.returncode:raise RuntimeError(f'qs ipc exit={r.returncode}; stdout={r.stdout!r}; stderr={r.stderr!r}; log={log.read_text()[-2000:]}')
                return r.stdout.strip()
            def state():return json.loads(ipc('state'))
            def named(name):
                s=state();return s.get('rows',{}).get(pair.hash,{}).get('name')==name,json.dumps(s)
            try:
                assert pair.h.wait_for(lambda:('Configuration Loaded' in log.read_text(),log.read_text()[-2000:]),'private dashboard registers IPC after QML load')
                case.check(ipc('open',pair.hash)=='true','dashboard subscribes to rows when visible',ipc('state'))
                assert pair.h.wait_for(lambda:named('CLI title'),'dashboard gets current display name')
                target,done=ui.start()
                assert ui.assert_pane(target,'CLI title')
                ui.keys(target,'N');assert ui.assert_pane(target,'Rename to:')
                ui.keys(target,'C-a',literal=False);ui.keys(target,'TUI title');ui.keys(target,'Enter',literal=False)
                assert pair.h.wait_for(lambda:(row()['name']=='TUI title',json.dumps(row())),'TUI N commits through daemon')
                assert pair.h.wait_for(lambda:named('TUI title'),'dashboard receives TUI rename delta')
                # Two records exercise real selector ambiguity and dashboard name-sort invalidation.
                other=pair.box.root/'other.bin';other.write_bytes(b'other fixture')
                item=fixtures.torrent_fixture(pair.box.root,[other]);pair.client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(pair.box.downloads),stopped=True))
                pair.h.cli('rename',item.info_hash,'TUI title');pair.h.cli('rename','TUI title','ambiguous',code=7)
                pair.h.cli('rename',item.info_hash,'Zulu title')
                case.check(ipc('open',pair.hash)=='true','dashboard opens the same torrent',ipc('state'))
                case.check(ipc('rename')=='true','dashboard N opens the real inline editor',ipc('state'))
                assert pair.h.wait_for(lambda:(state().get('editing') is True,ipc('state')),'inline editor owns typing focus')
                case.check(ipc('submit','Dashboard title')=='true','inline editor submits display name',ipc('state'))
                assert pair.h.wait_for(lambda:named('Dashboard title'),'dashboard and daemon confirm the new title')
                s=state();case.check(s['filtered']==[pair.hash] and s['keys']==[pair.hash,item.info_hash],'filter and sort follow the pushed display name',json.dumps(s))
                assert ui.assert_pane(target,'Dashboard title')
                ui.keys(target,'q');assert pair.h.wait_for(lambda:(done.exists(),ui.capture(target)),'q closes TUI only')
                pair.h.stop_serve(pair.p);pair.h.notify.close();pair.h.start_serve(idle_exit_secs=0);pair.h.receive_ready();pair.h.wait_engine()
                with d.ApiConn(pair.h) as current:
                    current.hello();value=row(current)
                    case.check(value['name']=='Dashboard title' and value['original_name']==original,'restart preserves display override and original name',json.dumps(value))
                    assert pair.h.wait_for(lambda:named('Dashboard title'),'dashboard reconnect preserves title')
                    pair.h.cli('rename',pair.hash,'')
                    case.check(row(current)['name']==original,'empty CLI name restores metadata title',json.dumps(row(current)))
                case.check(snapshot()==before,'all three surfaces and restart preserve payload names, inode and content',json.dumps(dict(before=before,after=snapshot())))
                ipc('hide');case.check(pair.h.daemon_pid()>0,'closing the dashboard leaves daemon alive',str(pair.h.daemon_pid()))
            finally:
                ui.close();subprocess.run(['qs','kill','--pid',str(child.pid)],env=env,capture_output=True,timeout=3)
                try:child.wait(timeout=3)
                except subprocess.TimeoutExpired:child.kill();child.wait()
                stream.close();compositor.close()
                errors=[line for line in log.read_text().splitlines() if any(x in line for x in ['TypeError:','ReferenceError:','SyntaxError:','Cannot assign','Binding loop detected','Failed to load configuration'])]
                case.check(child.returncode==0 and not errors,'private QML journey exits cleanly',json.dumps(dict(exit=child.returncode,errors=errors)))
        metadata_journey(case,pair,binary)
    return fbtlog.suite_status('rename_ui')
if __name__=='__main__':sys.exit(main())
