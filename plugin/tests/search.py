#!/usr/bin/env python3
"""Native Search acceptance using real Nova providers and a loopback torrent source."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import shutil
import subprocess
import sys
from dashboard import ROOT
from journey_support import JourneySession
import daemon
import fixtures
from fbtlog import Case,suite_status


def view(q):return q.state().get('view') or {}
def press(case,q,keys,predicate,expect):
    q.keys(keys,predicate)

def run(binary):
    from datetime import datetime
    archive=ROOT/'target/test-logs/fbt-p3-12-wkf/runs'/datetime.now().strftime('%H%M%S')
    for suite in ['qs_search','daemon']:
        previous=ROOT/'target/test-logs'/suite
        if previous.exists():shutil.copytree(previous,archive/suite)
    with daemon.imdb_owned('search_setup',binary,False) as (setup,sandbox,harness,http,directory,cache),ExitStack() as stack:
        runtime=Path(sandbox.env['XDG_DATA_HOME'])/'fbittorrent/nova3'
        engines=runtime/'engines';engines.mkdir(parents=True,exist_ok=True)
        revision=subprocess.check_output(['git','rev-parse','main'],cwd=ROOT,text=True).strip()
        for name in ['alpha','beta']:
            path=Path('agent_ergonomics_audit/audit/regression_tests/fixtures/providers')/(name+'.py')
            content=(ROOT/path).read_bytes() if (ROOT/path).exists() else subprocess.check_output(['git','show',revision+':'+str(path)],cwd=ROOT)
            (engines/(name+'.py')).write_bytes(content)
        source=sandbox.root/'search-source';source.mkdir()
        sources=[]
        for title,size,seeds,uploaded in [('Ten',10,100,9),('Two',2,2,10)]:
            payload=source/(title+'.bin');payload.write_bytes((title.encode()*size)[:size])
            fixture=fixtures.torrent_fixture(source,[payload],name=title)
            sources.append(dict(title=title,size=size,seeds=seeds,uploaded=uploaded,fixture=fixture))
        torrent_http=stack.enter_context(fixtures.HttpFixture(source))
        items=[dict(link=torrent_http.base+'/'+r['fixture'].torrent.name,name='Local Fixture' if r['title']=='Ten' else r['title'],size=r['size'],seeds=r['seeds'],leech=-1,engine_url=torrent_http.base,desc_link=torrent_http.base+'/source',pub_date=r['uploaded']) for r in sources]
        (runtime/'fixture-items.json').write_text(json.dumps(items))
        (engines/'gamma.py').write_text('''class gamma:
 name="Local Gamma"
 url="https://gamma.invalid"
 def search(self,query):
  import json,pathlib
  from novaprinter import prettyPrinter
  for row in json.loads(pathlib.Path("fixture-items.json").read_text()):prettyPrinter(row)
''')
        fixtures.path_shim('xdg-open','exit 0',sandbox.root);fixtures.path_shim('wl-copy','exit 0',sandbox.root)
        client=stack.enter_context(daemon.ApiConn(harness));hello=client.hello()['result']
        if not {'search','subscribe'}.issubset(hello['caps']):raise RuntimeError('real Search acceptance requires the daemon search service')
        hash='a'*40
        client.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+hash),stopped=True))
        # Keep the private virtual seat attached across Search/Torrents switches.
        q=JourneySession(setup,sandbox)
        try:
            q.compositor.ipc('output * mode 1920x1080')
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"search","query":"Sintel"}')
            with Case('qs_search','search_focus') as case:
                try:q.wait(lambda:view(q).get('focus')=='query' and view(q).get('query')=='Sintel')
                except TimeoutError as error:raise RuntimeError(q.ipc('searchFocus')) from error
                # Headless Sway has no physical keyboard. Keep the virtual device
                # alive while inspecting activeFocus; wtype exit removes it.
                keyboard=subprocess.Popen(['wtype','-s','80','-k','End',' typed','-s','1000'],env=q.env)
                try:
                    q.wait(lambda:view(q).get('query')=='Sintel typed' and view(q).get('focus')=='query')
                    focus=json.loads(q.ipc('searchFocus'))
                    case.check(focus['query']['active'] and focus['query']['focus'],'query payload receives actual native typing while the virtual keyboard is alive',json.dumps(focus))
                finally:keyboard.wait(timeout=3)
                press(case,q,['-k','BackSpace']*6,lambda:view(q).get('query')=='Sintel','BackSpace edits the same native query')
            with Case('qs_search','search_stream') as case:
                q.wait(lambda:len(view(q).get('rows',[]))==3,timeout=12)
                rows=view(q)['rows'];merged=next(r for r in rows if r['key']==hash)
                case.check(merged['providers']==['alpha','beta'] and merged['name']=='辛特尔' and view(q)['search']['frames']>=3,'real provider streams deduplicate repaired names and retain both provider names',json.dumps(dict(row=merged,search=view(q)['search']),ensure_ascii=False))
                q.wait(lambda:view(q).get('posterLoaded') is True and view(q).get('rating')==8.2,timeout=12)
                case.check(view(q)['poster']=='tt1234001','rested result without an ID resolves through the real IMDb fixture and renders its local poster/rating',json.dumps(view(q)))
                image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
                press(case,q,['-k','Down'],lambda:view(q).get('focus')=='results','Down leaves the query and routes keys to results')
            with Case('qs_search','search_sort_typed') as case:
                case.check(view(q)['rows'][0]['size']==10,'initial seeds sort puts100 before9 and2',json.dumps(view(q)['rows']))
                for key in ['size','uploaded','name','rating','seeds']:
                    press(case,q,['s'],lambda:view(q).get('sort',{}).get('key')==key,'s selects '+key+' typed sort')
                    rows=view(q)['rows']
                    known=[r for r in rows if ({'size':'size','seeds':'seeds','uploaded':'uploaded_at','rating':'rating','name':'name'}[key]) in r]
                    if key=='uploaded':case.check([r['uploaded_at'] for r in known]==[10,9],'uploaded uses numeric timestamps10 before9',json.dumps(known))
                press(case,q,['S'],lambda:view(q).get('sort',{}).get('reverse') is True,'S reverses the selected typed sort')
                case.check(view(q)['rows'][0]['seeds']==2,'reversed numeric seeds puts2 before9 and100',json.dumps(view(q)['rows']))
                press(case,q,['S'],lambda:view(q).get('sort',{}).get('reverse') is False,'S restores descending seeds')
            with Case('qs_search','search_added_marker') as case:
                press(case,q,['j'],lambda:view(q).get('cursor')==hash,'j selects the merged existing hash')
                case.check(hash in view(q)['added'],'result shows the server-provided + In your list marker',json.dumps(view(q)))
                press(case,q,['-k','Return'],lambda:view(q).get('window',{}).get('view')=='torrents','Enter on added selects the existing ledger torrent')
                case.check(view(q)['cursor']==hash and len(client.call('torrents.list',{})['result']['torrents'])==1,'existing selection sends no second add/grab',json.dumps(view(q)))
            with Case('qs_search','search_browse') as case:
                press(case,q,['2'],lambda:view(q).get('window',{}).get('view')=='search' and len(view(q).get('rows',[]))==3,'2 returns from the selected torrent to real streamed Search')
                press(case,q,['-k','Down'],lambda:view(q).get('focus')=='results','leave query for Browse shortcut')
                press(case,q,['b'],lambda:view(q).get('panel')=='browse','b opens the real daemon browse catalogue')
                q.wait(lambda:view(q).get('panelCount',0)>0)
                catalogue=client.call('search.browse_sources')['result']['sources']
                case.check(len(catalogue)>=3 and any(r['in_app'] for r in catalogue),'browse lists include in-app lists and browser sources',json.dumps(catalogue))
                press(case,q,['-k','Return'],lambda:(sandbox.root/'shims/xdg-open.log').exists(),'Enter on a site-only source opens its explicit URL without scraping it')
                press(case,q,['-k','Escape'],lambda:not view(q).get('panel'),'Esc closes browse')
                press(case,q,['P'],lambda:view(q).get('panel')=='providers','P opens the actual provider sheet')
                q.wait(lambda:view(q).get('panelCount')==3)
                press(case,q,['-k','space'],lambda:not client.call('search.providers')['result']['providers'][0]['enabled'],'Space changes a provider through the real durable API')
                press(case,q,['-k','Escape'],lambda:not view(q).get('panel'),'Esc closes providers')
            with Case('qs_search','search_enter_stages') as case:
                # Review stages a real provider result before any payload download.
                press(case,q,['g'],lambda:view(q).get('cursor')==sources[0]['fixture'].info_hash,'g selects the new real metainfo result')
                before=client.call('torrents.list',{})['result']['torrents']
                press(case,q,['-k','Return'],lambda:view(q).get('add',{}).get('visible'),'Enter opens the shared add sheet')
                add=view(q)['add']
                q.wait(lambda:view(q).get('add',{}).get('phase')=='ready',timeout=12)
                row=client.call('torrents.get',dict(hash=sources[0]['fixture'].info_hash))['result']['torrent']
                case.check(row['staged'] and len(client.call('torrents.list',{})['result']['torrents'])==len(before)+1,'Enter stages exactly one torrent before Download',json.dumps(row))
                press(case,q,['-k','Return'],lambda:not view(q).get('add',{}).get('visible'),'Download confirmation commits through search.grab')
                row=client.call('torrents.get',dict(hash=sources[0]['fixture'].info_hash))['result']['torrent']
                case.check(row['origin']=='search' and view(q).get('window',{}).get('view')=='torrents','confirmed real loopback metainfo retains search origin and selects its torrent',json.dumps(row))
                def rendered():return json.loads(q.ipc('renderedRows'))
                q.wait(lambda:len(rendered())==len(before)+1 and all(r.get('hash') for r in rendered()) and any(r['hash']==row['hash'] and row['name'] in r['texts'] and 'Reviewing' not in r['texts'] for r in rendered()))
                case.check(not row['staged'],'Search commit updates the real rendered row name/state without leaving a review delegate',json.dumps(rendered()))
        finally:q.close()
    return suite_status('qs_search') or suite_status('daemon')

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,required=True)
    args=parser.parse_args();sys.exit(run(args.binary.resolve()))
