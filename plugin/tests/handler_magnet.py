#!/usr/bin/env python3
"""Handler magnet -> one review -> Enter -> real loopback payload, private state."""
import argparse,base64,hashlib,json,os,socket,subprocess,time,tomllib,traceback,urllib.parse
from contextlib import ExitStack
from pathlib import Path
from dashboard import Session
import daemon,fixtures
from fbtlog import Case,suite_status

def run(binary):
    with Case('qs_handler_magnet','review') as case,ExitStack() as stack:
        nodes=[]
        for _ in range(2):
            box=stack.enter_context(fixtures.Sandbox());h=stack.enter_context(daemon.Harness(case,binary,box,False))
            p=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
            c=stack.enter_context(daemon.ApiConn(h));hello=c.hello()['result'];nodes.append((box,h,c))
        seedbox,seedh,seed=nodes[0];box,h,c=nodes[1]
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        compact=socket.inet_aton('127.0.0.1')+port.to_bytes(2,'big')
        routes={'/announce':dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=compact))).decode()),'/seed':dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=b''))).decode())}
        http=stack.enter_context(fixtures.HttpFixture(box.root,routes))
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
            started=time.monotonic();child=subprocess.Popen([str(binary),'add','--origin','handler',magnet,'--json'],env=q.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            def sheet():return (q.state().get('view') or {}).get('add') or {}
            opened=q.wait(lambda:s if (s:=sheet()).get('visible') else None,timeout=12)
            click_ms=round((time.monotonic()-started)*1000)
            case.check(opened['name']==payload.name,'magnet dn name appears in the real review sheet',json.dumps(dict(build=hello['server']['build'],click_to_sheet_ms=click_ms,sheet=opened)))
            q.wait(lambda:sheet().get('phase')=='ready',timeout=12)
            q.compositor.ipc('[title="fbittorrent"] focus')
            pressed=time.monotonic();subprocess.run(['wtype','-k','Return'],env=q.env,check=True,timeout=3)
            samples=[]
            def downloading():
                current=sheet();row=c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent']
                samples.append(dict(ms=round((time.monotonic()-pressed)*1000),phase=current.get('phase'),visible=current.get('visible'),state=row['state'],staged=row['staged'],completed_bytes=row.get('completed_bytes',0),error=current.get('error')))
                if current.get('phase')=='error':raise AssertionError(json.dumps(dict(sheet=current,samples=samples)))
                return row if row['state']=='downloading' and not row['staged'] and not current.get('visible') else None
            row=q.wait(downloading,timeout=12);download_ms=round((time.monotonic()-pressed)*1000)
            q.wait(lambda:c.call('torrents.get',dict(hash=source.info_hash))['result']['torrent'].get('completed_bytes',0)>0,timeout=12)
            first_bytes_ms=round((time.monotonic()-pressed)*1000)
            until=time.monotonic()+1
            while time.monotonic()<until:
                assert not sheet().get('visible'),sheet();time.sleep(.05)
            out,err=child.communicate(timeout=12);assert child.returncode==0,(out,err)
            assert len(c.call('torrents.list',{})['result']['torrents'])==1 and row['seed'] is False
            case.check(True,'one handler review and one Enter commit; sheet stays closed while real bytes download',json.dumps(dict(click_to_sheet_ms=click_ms,enter_to_downloading_ms=download_ms,enter_to_bytes_ms=first_bytes_ms,samples=samples,handler=json.loads(out))))
            print(json.dumps(dict(click_to_sheet_ms=click_ms,enter_to_downloading_ms=download_ms,enter_to_bytes_ms=first_bytes_ms)),flush=True)
        except Exception:
            print(json.dumps(q.state()),flush=True);traceback.print_exc();raise
        finally:
            if 'child' in locals() and child.poll() is None:child.terminate();child.communicate(timeout=3)
            keyboard.terminate();keyboard.wait(timeout=3);q.close()
    return suite_status('qs_handler_magnet')
if __name__=='__main__':
    import private_net
    private_net.enter();p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',required=True,type=Path);a=p.parse_args()
    raise SystemExit(run(a.binary.resolve()))
