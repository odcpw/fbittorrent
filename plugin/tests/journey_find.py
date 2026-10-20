"""W1: stream/deduplicate, inspect, stage, download and watch through the dashboard."""
from contextlib import ExitStack
import base64
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import time
import tomllib
import urllib.parse
from dashboard import Session,ROOT
import daemon
import daemon_notify as notify
import daemon_watch as watch
import fixtures
from journey_support import JourneySession
import grid
from fbtlog import Case,suite_status

MIB=1024*1024

def run(binary):
    with Case('qs_journey_w1','w1_find_watch') as case,daemon.Sandbox() as box,daemon.Sandbox() as seedbox,ExitStack() as stack:
        # All provider/IMDb/browse/tracker traffic remains within the network namespace.
        notify.shims(box,action="open");notify.config(box,notify=True)
        directory=box.root/'imdb-http';shutil.copytree(fixtures.HERE/'fixtures/imdb',directory)
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent'
        (cache/'title.ratings.tsv').write_bytes((directory/'ratings.tsv').read_bytes())
        imdb=stack.enter_context(fixtures.imdb_server(directory))
        browse=stack.enter_context(fixtures.HttpFixture(box.root,grid.content(box)))
        (cache/'title.ratings.tsv').write_bytes((directory/'ratings.tsv').read_bytes())
        for key,value in [('FBT_IMDB_BASE_URL',imdb.base),('FBT_TEST_BROWSE_ROOT',browse.base)]:box.env[key]=value;box.keys.append(key)
        runtime=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/nova3';engines=runtime/'engines';engines.mkdir(parents=True)
        for provider in ('local_one','local_two'):
            (engines/(provider+'.py')).write_text('class '+provider+':\n name="'+provider+'"\n url="https://fixture.invalid"\n def search(self,query):\n  import json,pathlib\n  from novaprinter import prettyPrinter\n  for row in json.loads(pathlib.Path("fixture-items.json").read_text()):\n   row["engine_url"]="https://'+provider+'.invalid";prettyPrinter(row)\n')
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        peers=socket.inet_aton('127.0.0.1')+port.to_bytes(2,'big')
        tracker=stack.enter_context(fixtures.HttpFixture(seedbox.root,{'/announce':dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=peers))).decode())}))
        source=seedbox.downloads;video=source/'Sintel.mkv';notes=source/'credits.txt'
        with video.open('wb') as output:
            output.write(watch.header()[:-8]+struct.pack(">d",256000.))
            while output.tell()<128*MIB:output.write(os.urandom(min(MIB,128*MIB-output.tell())))
        notes.write_bytes(b'Private credits\n'*65536)
        torrent=fixtures.torrent_fixture(seedbox.root,[video,notes],name='Local Fixture',tracker=tracker.base+'/announce',piece_length=MIB)
        metainfo=stack.enter_context(fixtures.HttpFixture(seedbox.root))
        magnet=torrent.magnet+'&xs='+urllib.parse.quote(metainfo.base+'/'+torrent.torrent.name)
        (runtime/'fixture-items.json').write_text(json.dumps([dict(link=metainfo.base+'/'+urllib.parse.quote(torrent.torrent.name),name='Local Fixture',size=video.stat().st_size+notes.stat().st_size,seeds=7,leech=0,engine_url=metainfo.base,desc_link=metainfo.base+'/details',pub_date=1700000000)]))
        h=stack.enter_context(daemon.Harness(case,binary,box));seed=stack.enter_context(daemon.Harness(case,binary,seedbox))
        process=h.start_serve(idle_exit_secs=0);sp=seed.start_serve(idle_exit_secs=0);q=None
        try:
            h.receive_ready();h.wait_engine();seed.receive_ready();seed.wait_engine()
            c=stack.enter_context(daemon.ApiConn(h));peer=stack.enter_context(daemon.ApiConn(seed));c.hello();peer.hello()
            peer.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(seedbox.downloads),seed=True))
            assert seed.wait_for(lambda:((r:=watch.row(peer,torrent.info_hash))['state']=='seeding',json.dumps(r)),'private video seed completes verification')
            peer.call('transfer.set_limits',dict(upload_bps=2*MIB))
            q=JourneySession(case,box)
            def view():return q.state().get('view') or {}
            def add():return view().get('add') or {}
            def row():return watch.row(c,torrent.info_hash)
            def files():return c.call('torrents.files',dict(hash=torrent.info_hash))['result']['files']
            def press(keys,predicate):return q.keys(keys,predicate)
            q.compositor.ipc('output * mode 1920x1080')
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}');q.wait(lambda:view().get('window',{}).get('visible'))
            press(['2'],lambda:view().get('window',{}).get('view')=='search' and view().get('focus')=='query')
            press(['Sintel','-k','Return'],lambda:len(view().get('rows',[]))==1)
            q.wait(lambda:view().get('posterLoaded') and view().get('rating')==8.2,timeout=12)
            result=view()['rows'][0]
            case.check(result['key']==torrent.info_hash and len(result['providers'])==2 and result['size']==video.stat().st_size+notes.stat().st_size and result['seeds']==7 and result.get('uploaded_at'),'typed query streams one deduplicated result with size, seeds, providers, date, IMDb rating and a decoded poster',json.dumps(view()))
            press(['-k','Return'],lambda:add().get('visible'));q.wait(lambda:add().get('phase')=='ready',timeout=15)
            case.check(row()['staged'] and not row()['checked'] and not list(box.downloads.iterdir()) and len(add()['files'])==2 and add()['savePath']==str(box.downloads) and add()['seed'] is False,'review stages metadata with files, upload off and destination, before any transfer',json.dumps(add()))
            press(['-k','Escape'],lambda:not add().get('visible'))
            case.check(not c.call('torrents.list',{})['result']['torrents'],'Discard leaves no torrent or download','no transfer was started')
            press(['-k','Return'],lambda:add().get('visible'));q.wait(lambda:add().get('phase')=='ready',timeout=15)
            began=time.monotonic()
            press(['-k','Return'],lambda:not add().get('visible') and row().get('completed_bytes',0)>0)
            elapsed=time.monotonic()-began
            q.wait(lambda:torrent.info_hash in view().get('added',[]))
            case.check(elapsed<=2 and view()['window']['view']=='search','Download starts the real loopback transfer within two seconds and keeps the results open with the row marked',dict(elapsed_s=elapsed,row=row(),window=view().get('window')))
            press(['1'],lambda:view().get('window',{}).get('view')=='torrents' and view().get('cursor')==torrent.info_hash and view().get('rowProgress',0)>0)
            press(['-k','Return'],lambda:view().get('focus')=='details')
            press([']'],lambda:(view().get('details') or {}).get('tab')=='files')
            press(['/credits','-k','Escape'],lambda:(view().get('details') or {}).get('files')==[1])
            press(['-k','space'],lambda:files()[1]['included'] is False)
            case.check(files()[0]['included'],'Files can exclude credits without dropping the video selection',str(files()))
            press(['['],lambda:(view().get('details') or {}).get('tab')=='overview')
            q.wait(lambda:row().get('playable',{}).get('ready') is True,timeout=35)
            q.wait(lambda:view().get('stateLabel')=='Ready to watch',timeout=8)
            ready=row();case.check(ready['state'] in ['downloading','stalled'] and ready['completed_bytes']<ready['size'] and ready['playable']['ready'],'Ready to watch appears while real video bytes are still downloading',json.dumps(ready))
            target=str(box.downloads/'Sintel.mkv')
            q.wait(lambda:any(target in call for call in notify.calls(box,'xdg-open')),timeout=5)
            opened=len(notify.calls(box,'xdg-open'))
            case.check(notify.calls(box,'xdg-open')[-1]==[target],'the ready notification Open action requests the literal video path',notify.calls(box,'xdg-open'))
            press(['o'],lambda:len(notify.calls(box,'xdg-open'))>opened)
            case.check(notify.calls(box,'xdg-open')[-1]==[target] and row()['completed_bytes']<row()['size'],'dashboard o requests the same ready video while its transfer continues',notify.calls(box,'xdg-open'))
            case.check(any(('-A' in call or '--action' in call) and 'Open' in ' '.join(call) for call in notify.calls(box)),'ready notification offers Open',notify.calls(box))
            # M2 demo: live Settings, responsive details, empty-query poster grid, detach.
            press([','],lambda:view().get('window',{}).get('view')=='settings')
            q.wait(lambda:(view().get('settings') or {}).get('loaded'))
            assert q.ipc('focusSetting','max_active_downloads')=='true'
            press(['-M','ctrl','a','-m','ctrl','3','-k','Return'],lambda:c.call('app.settings.get')['result']['max_active_downloads']==3)
            case.check(process.child.poll() is None,'Active downloads applies through the visible Settings control without an engine restart','max_active_downloads=3')
            press(['-k','Escape'],lambda:view().get('window',{}).get('view')=='torrents')
            q.compositor.ipc('[title="^fbittorrent$"] floating enable, resize set 1100 px 720 px')
            q.wait(lambda:(view().get('details') or {}).get('mode')=='sheet')
            press(['-k','Return'],lambda:(view().get('details') or {}).get('visible'))
            press(['-k','Escape'],lambda:not (view().get('details') or {}).get('visible'))
            press(['2'],lambda:view().get('window',{}).get('view')=='search')
            press(['/','-M','ctrl','a','-m','ctrl','-k','BackSpace','-k','Return'],lambda:(view().get('grid') or {}).get('ready',0)>0)
            case.check(view()['grid']['cells']>0,'an empty query renders the daemon browse list as a poster grid',json.dumps(view()['grid']))
            press(['1'],lambda:view().get('window',{}).get('view')=='torrents')
            before=row()['completed_bytes'];press(['q'],lambda:q.state().get('view') is None)
            q.wait(lambda:row()['completed_bytes']>before,timeout=4)
            case.check(process.child.poll() is None,'closing the dashboard detaches while the same transfer continues',json.dumps(row()))
        except Exception:
            if q:print("W1 state at failure: "+json.dumps(q.state())+"; confirmed="+q.ipc("confirmed",torrent.info_hash),flush=True)
            import traceback;traceback.print_exc();raise
        finally:
            try:
                if q:q.close()
            finally:
                for owner,child in [(h,process),(seed,sp)]:
                    if child.child.poll() is None:owner.stop_serve(child)
    return suite_status('qs_journey_w1')
