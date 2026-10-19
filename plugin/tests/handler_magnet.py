#!/usr/bin/env python3
"""Handler magnet -> one review -> Enter -> real loopback payload, private state."""
import argparse,base64,hashlib,http.server,json,os,shutil,socket,subprocess,threading,time,tomllib,traceback,urllib.parse
from contextlib import ExitStack
from pathlib import Path
from dashboard import Session,ROOT
import daemon,fixtures,daemon_notify as notify
from fbtlog import Case,suite_status

class Tracker:
    """Withhold peer discovery until Enter in the metadata-unknown control."""
    def __init__(self,port,hold):
        self.release=threading.Event()
        if not hold:self.release.set()
        owner=self;peers=socket.inet_aton('127.0.0.1')+port.to_bytes(2,'big')
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self,*_):pass
            def do_GET(self):
                if self.path.startswith('/announce'):owner.release.wait(15);value=peers
                else:value=b''
                body=fixtures.bencode(dict(interval=1,peers=value))
                self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers()
                try:self.wfile.write(body)
                except OSError:pass
        self.server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.base='http://127.0.0.1:'+str(self.server.server_port)
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True);self.thread.start()
    def close(self):self.release.set();self.server.shutdown();self.server.server_close();self.thread.join(timeout=2)

def cpu_sample():
    values=list(map(int,Path("/proc/stat").read_text().splitlines()[0].split()[1:9]))
    return sum(values),values[3]+values[4]

def run(binary,early=False):
    with Case('qs_handler_magnet','early' if early else 'review') as case,ExitStack() as stack:
        nodes=[]
        for index in range(2):
            box=stack.enter_context(fixtures.Sandbox())
            if index==1:
                notify.config(box,notify=True);notify.shims(box)
                config=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
                config.write_text(config.read_text()+'\nseed_by_default = true\n')
            h=stack.enter_context(daemon.Harness(case,binary,box,False))
            p=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
            c=stack.enter_context(daemon.ApiConn(h));hello=c.hello()['result'];nodes.append((box,h,c))
        seedbox,seedh,seed=nodes[0];box,h,c=nodes[1]
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        http=Tracker(port,early);stack.callback(http.close)
        payload=seedbox.downloads/'Owner magnet review.mkv';payload.write_bytes(os.urandom(8*1024*1024))
        source=fixtures.torrent_fixture(box.root,[payload],tracker=http.base+'/announce')
        # BEP 27 intentionally forbids private seeds from sharing BEP 9 metadata.
        source.info.pop('private');source.info_hash=hashlib.sha1(fixtures.bencode(source.info)).hexdigest()
        source.torrent.write_bytes(fixtures.bencode(dict(info=source.info,announce=http.base+'/announce')))
        source.magnet='magnet:?'+urllib.parse.urlencode(dict(xt='urn:btih:'+source.info_hash,dn=payload.name,tr=http.base+'/announce'))
        seedfile=seedbox.root/'seed.torrent';seedfile.write_bytes(fixtures.bencode(dict(info=source.info,announce=http.base+'/seed')))
        seed.call('torrents.add',dict(source=dict(path=str(seedfile)),save_path=str(seedbox.downloads),seed=True))
        assert seedh.wait_for(lambda:((r:=seed.call('torrents.get',dict(hash=source.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(r)),'owned seed is ready')
        seed.call('torrents.set_upload_limit',dict(hashes=[source.info_hash],bps=32768))
        magnet=source.magnet
        q=Session(case,box);keyboard=subprocess.Popen(['wtype','-s','600000'],env=q.env)
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            link=Path(box.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents';link.parent.mkdir(parents=True);link.symlink_to(q.config/'Plugin',target_is_directory=True)
            fixtures.path_shim('omarchy-shell','exec /usr/share/omarchy/bin/omarchy-shell "$@"',box.root)
            cpu_start=cpu_sample();started=time.monotonic();child=subprocess.Popen([str(binary),'add','--origin','handler',magnet,'--json'],env=q.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            def sheet():return (q.state().get('view') or {}).get('add') or {}
            opened=q.wait(lambda:s if (s:=sheet()).get('visible') and s.get('field')=='sheet' else None,timeout=12)
            click_ms=round((time.monotonic()-started)*1000)
            case.check(opened['name']==payload.name,'magnet dn name appears in the real review sheet',json.dumps(dict(build=hello['server']['build'],click_to_sheet_ms=click_ms,sheet=opened)))
            if early:
                q.wait(lambda:sheet().get('hash')==source.info_hash and sheet().get('destinationReady'))
                row=c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']
                assert 'size' not in row and not sheet().get('files') and not list(box.downloads.iterdir()),row
            else:q.wait(lambda:sheet().get('phase')=='ready',timeout=12)
            assert not notify.calls(box),'a staged review must not claim Added'
            assert sheet()['seed'] is False
            q.compositor.ipc('[title="fbittorrent"] focus')
            pressed=time.monotonic();subprocess.run(['wtype','-k','Return'],env=q.env,check=True,timeout=3)
            if early:
                q.wait(lambda:not sheet().get('visible') and not c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']['staged'])
                case.check(True,'Download commits and closes while metadata is still withheld',json.dumps(sheet()))
                http.release.set()
            samples=[]
            def downloading():
                current=sheet();row=c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']
                samples.append(dict(ms=round((time.monotonic()-pressed)*1000),phase=current.get('phase'),visible=current.get('visible'),state=row['state'],staged=row['staged'],completed_bytes=row.get('completed_bytes',0),error=current.get('error')))
                if current.get('phase')=='error':raise AssertionError(json.dumps(dict(sheet=current,samples=samples)))
                return row if row['state']=='downloading' and not row['staged'] and not current.get('visible') else None
            row=q.wait(downloading,timeout=12);download_ms=round((time.monotonic()-pressed)*1000)
            q.wait(lambda:c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent'].get('completed_bytes',0)>0,timeout=12)
            first_bytes_ms=round((time.monotonic()-pressed)*1000)
            cpu_end=cpu_sample();total=cpu_end[0]-cpu_start[0];cpu_busy=round(100*(1-(cpu_end[1]-cpu_start[1])/max(1,total)),1)
            until=time.monotonic()+1
            while time.monotonic()<until:
                assert not sheet().get('visible'),sheet();time.sleep(.05)
            q.wait(lambda:len(notify.calls(box))==1)
            assert notify.calls(box)[0][-2]=='Added',notify.calls(box)
            files=c.call('torrents.files',dict(hash=source.info_hash))['result']['files']
            assert files and all(row['included'] for row in files),files
            out,err=child.communicate(timeout=12);assert child.returncode==0,(out,err)
            assert len(c.call('torrents.list',{})['result']['torrents'])==1 and row['seed'] is False
            extra='e'*40
            c.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+extra),stage=True))
            case.check(not c.call('torrents.get',dict(hash=extra))['result']['torrent']['seed'],'omitting seed on the daemon API stays off despite the saved old default',extra)
            c.call('torrents.discard',dict(hash=extra))
            q.keys(['u'],lambda:((q.state().get('view') or {}).get('details') or {}).get('visible'))
            assert not c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']['seed']
            q.keys(['u'],lambda:c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']['seed'])
            q.keys(['u'],lambda:not c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']['seed'])
            case.check(True,'only the Details override changes Upload off/on/off',json.dumps(c.call('torrents.get',dict(hash=source.info_hash))))
            case.check(True,'one handler review and one Enter commit; sheet stays closed while real bytes download',json.dumps(dict(click_to_sheet_ms=click_ms,enter_to_downloading_ms=download_ms,enter_to_bytes_ms=first_bytes_ms,samples=samples,handler=json.loads(out))))
            print(json.dumps(dict(click_to_sheet_ms=click_ms,enter_to_downloading_ms=download_ms,enter_to_bytes_ms=first_bytes_ms,host_cpu_busy_pct=cpu_busy)),flush=True)
        except Exception:
            print(json.dumps(q.state()),flush=True)
            if 'child' in locals() and child.poll() is not None:print('handler:',child.communicate(),flush=True)
            log=box.root/'shims/omarchy-shell.log'
            if log.exists():print('handoff:',log.read_text(),flush=True)
            traceback.print_exc();raise
        finally:
            if 'child' in locals() and child.poll() is None:child.terminate();child.communicate(timeout=3)
            keyboard.terminate();keyboard.wait(timeout=3);q.close()
    return suite_status('qs_handler_magnet')
if __name__=='__main__':
    import private_net
    private_net.enter()
    old=ROOT/'target/test-logs/qs_handler_magnet'
    if old.exists():shutil.copytree(old,ROOT/'target/test-logs/qs_handler_magnet_runs'/str(time.time_ns()))
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',required=True,type=Path);p.add_argument('--early',action='store_true');a=p.parse_args()
    raise SystemExit(run(a.binary.resolve(),a.early))
