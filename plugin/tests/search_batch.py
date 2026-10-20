#!/usr/bin/env python3
"""Search multi-select: Space marks results, Enter confirms one batch, results stay open."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import traceback
import urllib.parse
from journey_support import JourneySession
import daemon
import fixtures
from fbtlog import Case,suite_status


def run(binary):
    with Case('qs_search_batch','select_and_download') as case,ExitStack() as stack:
        box=stack.enter_context(fixtures.Sandbox());h=stack.enter_context(daemon.Harness(case,binary,box,False))
        # Local Nova scripts cannot honour a binding; private_net confines the run.
        daemon.select_replay_interface(Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml',None)
        h.start_serve(idle_exit_secs=0)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private engine startup failed')
        client=stack.enter_context(daemon.ApiConn(h));client.hello()
        source=box.root/'batch-source';source.mkdir()
        torrents=[]
        for title in ('Batch One','Batch Two'):
            payload=source/(title+'.bin');payload.write_bytes(title.encode()*64)
            torrents.append(fixtures.torrent_fixture(source,[payload],name=title))
        http=stack.enter_context(fixtures.HttpFixture(source))
        nova=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/nova3';engines=nova/'engines';engines.mkdir(parents=True,exist_ok=True)
        rows=[dict(link=http.base+'/'+urllib.parse.quote(t.torrent.name),name=t.torrent.stem,size=128,seeds=5-i,leech=0,engine_url=http.base,desc_link=http.base+'/source') for i,t in enumerate(torrents)]
        (nova/'rows.json').write_text(json.dumps(rows))
        (engines/'batch.py').write_text('''class batch:
 name="Batch fixture"
 url="http://127.0.0.1"
 def search(self,query):
  import json,pathlib
  from novaprinter import prettyPrinter
  for row in json.loads(pathlib.Path("rows.json").read_text()):prettyPrinter(row)
''')
        q=JourneySession(case,box)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
            def view():return q.state().get('view') or {}
            q.keys(['2'],lambda:view().get('window',{}).get('view')=='search' and view().get('focus')=='query')
            q.keys(['Batch','-k','Return'],lambda:len(view().get('rows',[]))==2)
            q.wait(lambda:view().get('search',{}).get('phase')=='done',timeout=12)
            q.keys(['-k','Down'],lambda:view().get('focus')=='results')
            keys=[r['key'] for r in view()['rows']]
            # Streaming results keep the cursor on the first row that arrived.
            q.keys(['g'],lambda:view().get('cursor')==keys[0])
            q.keys(['-k','space'],lambda:view().get('selection')==[keys[0]] and view().get('cursor')==keys[1])
            q.keys(['-k','space'],lambda:sorted(view().get('selection',[]))==sorted(keys))
            q.keys(['-k','Escape'],lambda:view().get('selection')==[])
            case.check(view().get('focus')=='results','Esc clears the selection before leaving the results',json.dumps(view()))
            # Shift+Down marks from the cursor through the row moved to.
            q.keys(['g','-M','shift','-k','Down','-m','shift'],lambda:sorted(view().get('selection',[]))==sorted(keys) and view().get('cursor')==keys[1])
            case.check(not client.call('torrents.list',{})['result']['torrents'],'marking results adds nothing by itself',json.dumps(view()))
            q.keys(['-k','Return'],lambda:view().get('panel')=='batch' and view().get('panelCount')==2)
            panel=case.path.with_name(case.path.stem+'-panel.png');q.ipc('grab',panel);q.wait(lambda:panel.exists() and panel.stat().st_size>0)
            q.keys(['-k','Return'],lambda:not view().get('panel'))
            def added():
                listed=client.call('torrents.list',{})['result']['torrents']
                return len(listed)==2 and all(not r.get('staged') for r in listed)
            q.wait(added,timeout=15)
            listed=client.call('torrents.list',{})['result']['torrents']
            details=[client.call('torrents.get',dict(hash=r['hash']))['result']['torrent'] for r in listed]
            case.check(sorted(d['hash'] for d in details)==sorted(t.info_hash for t in torrents),'one confirmation adds exactly the two selected results',json.dumps(details))
            case.check(all(d['origin']=='search' and d['upload_enabled'] is False for d in details),'batch adds keep search origin and upload disabled',json.dumps(details))
            try:q.wait(lambda:view().get('window',{}).get('view')=='search' and sorted(view().get('added',[]))==sorted(keys) and view().get('selection')==[],timeout=8);stayed=True
            except TimeoutError:stayed=False
            case.check(stayed,'results stay open with both rows marked + In your list and the selection cleared',json.dumps(view()))
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
        except Exception:
            print("Search batch state at failure: "+json.dumps(q.state()),flush=True)
            traceback.print_exc()
            raise
        finally:q.close()
    return suite_status('qs_search_batch')


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
