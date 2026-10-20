#!/usr/bin/env python3
"""Search review/discard/review/Download must update the real rendered table."""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import tomllib
import traceback
from journey_support import JourneySession
import daemon
import fixtures
from handler_magnet import Tracker
from fbtlog import Case,suite_status


def run(binary):
    with Case('qs_search_commit','discard_then_download') as case,ExitStack() as stack:
        owners=[]
        for index in range(2):
            box=stack.enter_context(fixtures.Sandbox());h=stack.enter_context(daemon.Harness(case,binary,box,False))
            # The local Nova script needs no configured binding; private_net
            # still confines the entire journey to its loopback-only namespace.
            if index == 1:
                daemon.select_replay_interface(Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml',None)
            h.start_serve(idle_exit_secs=0)
            if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private engine startup failed')
            client=stack.enter_context(daemon.ApiConn(h));client.hello();owners.append((box,h,client))
        seedbox,seedh,seed=owners[0];box,h,client=owners[1]
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        tracker=Tracker(port,False);stack.callback(tracker.close)
        payload=seedbox.downloads/'Search lifecycle.mkv';payload.write_bytes(os.urandom(8*fixtures.MIB))
        torrent=fixtures.torrent_fixture(box.root,[payload],tracker=tracker.base+'/announce')
        seed.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(seedbox.downloads),seed=True))
        if not seedh.wait_for(lambda:((row:=seed.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'real seed is ready'):raise RuntimeError('seed unavailable')
        seed.call('torrents.set_upload_limit',dict(hashes=[torrent.info_hash],bps=32768))
        http=stack.enter_context(fixtures.HttpFixture(box.root))
        nova=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/nova3';engines=nova/'engines';engines.mkdir(parents=True,exist_ok=True)
        (nova/'row.json').write_text(json.dumps(dict(link=http.base+'/'+torrent.torrent.name,name=payload.name,size=payload.stat().st_size,seeds=1,leech=0,engine_url=http.base,desc_link=http.base+'/source')))
        provider='''class rows:
 name="Row fixture"
 url="http://127.0.0.1"
 def search(self,query):
  import json,pathlib
  from novaprinter import prettyPrinter
  prettyPrinter(json.loads(pathlib.Path("row.json").read_text()))
  pathlib.Path("finished-"+type(self).__name__).write_text("done")
'''
        for name in ('rows','other'):
            (engines/(name+'.py')).write_text(provider.replace('class rows:', 'class '+name+':'))
        q=JourneySession(case,box)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
            q.wait(lambda:(q.state().get('view') or {}).get('rows')==0)
            def view():return q.state().get('view') or {}
            def add():return view().get('add') or {}
            q.keys(['2'],lambda:view().get('window',{}).get('view')=='search' and view().get('focus')=='query')
            q.keys(['Row fixture','-k','Return'],lambda:len(view().get('rows',[]))==1)
            q.wait(lambda:view().get('search',{}).get('phase')=='done',timeout=12)
            case.check(all((nova/('finished-'+name)).exists() for name in ('rows','other')),
                       'both real provider tasks return before normal pool retirement',str(list(nova.glob('finished-*'))))
            def clean_provider_status(step):
                state=json.loads(q.ipc('gridState'))
                case.check(state.get('phase')=='done' and state.get('message')=='',
                           'two finished providers keep a successful Search status '+step,json.dumps(state))
            clean_provider_status('before review')
            q.keys(['-k','Down'],lambda:view().get('focus')=='results')
            q.keys(['-k','Return'],lambda:add().get('phase')=='ready')
            clean_provider_status('while opening review')
            q.keys(['-k','Escape'],lambda:not add().get('visible'))
            q.wait(lambda:not client.call('torrents.list',{})['result']['torrents'])
            clean_provider_status('after Discard')
            q.keys(['-k','Return'],lambda:add().get('phase')=='ready')
            clean_provider_status('while reopening review')
            q.keys(['-k','Return'],lambda:not add().get('visible'))
            # Download from Search keeps the results open for the next pick and
            # marks this result in place; Torrents shows it when asked.
            try:q.wait(lambda:view().get('window',{}).get('view')=='search' and len(view().get('rows',[]))==1 and len(view().get('added',[]))==1,timeout=6);stayed=True
            except TimeoutError:stayed=False
            case.check(stayed,'Download from Search stays on the results and marks the row + In your list',json.dumps(view()))
            q.keys(['1'],lambda:view().get('window',{}).get('view')=='torrents')
            q.wait(lambda:(row:=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state']=='downloading' and not row['staged'] and row.get('completed_bytes',0)>0,timeout=15)
            def displayed():
                rendered=json.loads(q.ipc('renderedRows'));state=view()
                return len(rendered)==1 and rendered[0].get('hash')==torrent.info_hash and payload.name in rendered[0]['texts'] and 'Downloading' in rendered[0]['texts'] and 'Reviewing' not in rendered[0]['texts'] and isinstance(rendered[0].get('progress'),(int,float)) and 0<rendered[0]['progress']<1
            try:q.wait(displayed,timeout=6);ok=True
            except TimeoutError:ok=False
            row=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
            case.check(ok,'Search discard/review/Download retires Reviewing and renders live progress without reopening or sorting',json.dumps(dict(api=row,view=view(),rendered=json.loads(q.ipc('renderedRows')))))
            case.check(row['upload_enabled'] is False and row.get('ul_bps',0)==0,'downloading client keeps upload disabled',json.dumps(row))
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
        except Exception:
            print("Search commit state at failure: "+json.dumps(q.state()),flush=True)
            traceback.print_exc()
            raise
        finally:q.close()
    return suite_status('qs_search_commit')


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
