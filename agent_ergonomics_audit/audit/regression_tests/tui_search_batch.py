#!/usr/bin/env python3
"""TUI Search multi-select: Shift+Down marks a run, Enter twice adds exactly that run.

Run: tui_search_batch.py target/debug/fbittorrent. Private XDG, loopback only.
"""
import argparse
import json
from pathlib import Path
import sys
import urllib.parse
from fbtlog import Case,REPO,suite_status
import fbtlog
import daemon
import fixtures
from tui_smoke import PaneUi
sys.path.insert(0,str(REPO/'plugin/tests'))
import private_net

TITLES=('Batch Alpha','Batch Beta','Batch Gamma')


def run(binary):
    private_net.require()
    with Case('tui_search_batch','shift_down_then_enter') as case,fixtures.Sandbox() as box,daemon.Harness(case,binary,box,False) as h:
        # Local Nova scripts cannot honour a binding; private_net confines the run.
        daemon.select_replay_interface(Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml',None)
        h.start_serve(idle_exit_secs=0)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private engine startup failed')
        source=box.root/'batch-source';source.mkdir()
        torrents=[]
        for title in TITLES:
            payload=source/(title+'.bin');payload.write_bytes(title.encode()*64)
            torrents.append(fixtures.torrent_fixture(source,[payload],name=title))
        with fixtures.HttpFixture(source) as http,daemon.ApiConn(h) as client:
            client.hello()
            nova=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/nova3';engines=nova/'engines';engines.mkdir(parents=True,exist_ok=True)
            rows=[dict(link=http.base+'/'+urllib.parse.quote(t.torrent.name),name=title,size=128,seeds=30-10*i,leech=0,engine_url=http.base,desc_link=http.base+'/source')
                  for i,(title,t) in enumerate(zip(TITLES,torrents))]
            (nova/'rows.json').write_text(json.dumps(rows))
            (engines/'batch.py').write_text('''class batch:
 name="Batch fixture"
 url="http://127.0.0.1"
 def search(self,query):
  import json,pathlib
  from novaprinter import prettyPrinter
  for row in json.loads(pathlib.Path("rows.json").read_text()):prettyPrinter(row)
''')
            ui=PaneUi(case,str(binary),box,h,session='fbt-search-batch')
            try:
                ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','120','-y','30')
                def need(ok,what):
                    if not ok:raise RuntimeError('TUI batch step failed: '+what)
                need(ui.contains('2','Search'),'open Search')
                ui.press('batch','type the search query')
                need(ui.press('Enter','all three results arrive',lambda p:all(t in p for t in TITLES),literal=False),'search results')
                ui.press('Escape','leave query editing',lambda p:'[Query]' not in p,literal=False)
                ui.press('Home','first result selected',literal=False)
                need(ui.press('S-Down','Shift+Down marks the first two results',lambda p:p.count('✓ Batch')==2,literal=False),'Shift+Down marks')
                case.check(not client.call('torrents.list',{})['result']['torrents'],'marking adds nothing by itself','torrents.list is empty')
                need(ui.press('Enter','one confirmation names the batch',lambda p:'Download 2 results?' in p and 'no upload' in p,literal=False),'batch prompt')
                ui.press('Enter','Enter confirms the batch download',literal=False)
                def two_added():
                    listed=client.call('torrents.list',{})['result']['torrents']
                    return len(listed)==2 and all(not r.get('staged') for r in listed)
                ok=h.wait_for(lambda:(two_added(),ui.capture(ui.target)),'TUI batch adds exactly two results',timeout=15)
                details=[client.call('torrents.get',dict(hash=r['hash']))['result']['torrent'] for r in client.call('torrents.list',{})['result']['torrents']]
                case.check(ok and sorted(d['hash'] for d in details)==sorted(t.info_hash for t in torrents[:2]),'exactly the two marked results were added',json.dumps(details))
                case.check(all(d['origin']=='search' and d['upload_enabled'] is False for d in details),'batch adds keep search origin and upload off',json.dumps(details))
                ui.observe('rows show their added state','both added results show + In list and the third stays plain',
                           lambda p:p.count('+ In list')==2 and 'Batch Gamma' in p and '✓ Batch' not in p and 'added 2 search results' in p,timeout=8)
            finally:ui.close()
    return suite_status('tui_search_batch')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('binary',type=Path);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();private_net.enter()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    return run(args.binary.resolve())


if __name__=='__main__':sys.exit(main())
