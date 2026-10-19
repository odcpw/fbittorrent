"""W2: follow, preferred quality, unread inbox, deliberate add and persistence."""
from contextlib import ExitStack
import json
from pathlib import Path
import time
from dashboard import ROOT
from journey_support import JourneySession
import daemon
import daemon_notify as notify
import fixtures
from fbtlog import Case,suite_status

IMDB='tt1234001'

def run(binary):
    with Case('qs_journey_w2','w2_follow_shows') as case,daemon.Sandbox() as box,ExitStack() as stack:
        source=box.root/'episodes';source.mkdir();routes={'/api/get-torrents':dict(file='episodes.json')};payloads=[]
        for episode in (1,2,3):
            for quality in ('1080p','2160p'):
                path=source/f'Owned.Show.S01E{episode:02d}.{quality}.mkv';path.write_bytes((path.name.encode()+b'\0')*8192)
                torrent=fixtures.torrent_fixture(source,[path],name=f'{episode}-{quality}');payloads.append((episode,quality,torrent,path))
                routes[f'/{episode}-{quality}.torrent']=dict(file=str(torrent.torrent.relative_to(box.root)))
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        (cache/(IMDB+'.json')).write_text(json.dumps(dict(id=IMDB,name='Owned Shows Fixture',year=2026,kind='tvSeries',stars='',poster_url='',blurb='',wiki_url='')))
        http=stack.enter_context(fixtures.HttpFixture(box.root,routes))
        box.env.update(FBT_EZTV_BASE_URL=http.base,FBT_IMDB_BASE_URL=http.base);box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
        episodes=[dict(id=e*10+(q=='2160p'),imdb_id=IMDB[2:],title=p.name,season='1',episode=str(e),seeds=3,size_bytes=str(p.stat().st_size),date_released_unix=1700000000+e,magnet_url=http.base+f'/{e}-{q}.torrent') for e,q,t,p in payloads]
        def catalogue(rows):
            (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=len(rows),limit=100,page=1,torrents=rows)))
        catalogue([row for row in episodes if int(row['episode'])<=2])
        notify.shims(box);notify.config(box,notify=True)
        h=stack.enter_context(daemon.Harness(case,binary,box));process=h.start_serve(idle_exit_secs=0);q=None
        try:
            h.receive_ready();h.wait_engine();c=daemon.ApiConn(h);c.hello();q=JourneySession(case,box)
            def view():return q.state().get('view') or {}
            def shows():return view().get('shows') or {}
            def add():return view().get('add') or {}
            def records():return c.call('torrents.list',{})['result']['torrents']
            def releases():return c.call('shows.releases',dict(imdb=IMDB,unread_only=False))['result']['releases']
            def followed():return c.call('shows.list')['result']['shows']
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"shows"}')
            q.wait(lambda:'shows' in view())
            q.keys(['n'],lambda:shows().get('panel')=='follow')
            q.keys([IMDB,'-k','Return'],lambda:len(shows().get('shows',[]))==1 and not shows().get('panel'))
            case.check(not records(),'Follow creates a durable show without starting a download',json.dumps(followed()))
            q.shell('hide');q.wait(lambda:q.state().get('view') is None)
            q.shell('summon','{"view":"shows"}');q.wait(lambda:len(shows().get('rows',[]))==2,timeout=12)
            inbox=shows()['rows'];case.check(all(not r['read'] and r['transfer']=='none' for r in inbox) and inbox[0]['published_at']>inbox[1]['published_at'] and followed()[0]['unread']==2 and not records(),'opening an overdue Shows inbox checks locally, sorts newest first, shows unread counts and never downloads',json.dumps(shows()))
            case.check(not notify.calls(box),'the visible inbox suppresses a redundant desktop notification',notify.calls(box))
            q.shell('hide');q.wait(lambda:q.state().get('view') is None and json.loads(q.ipc('visibility'))['client'] is False)
            catalogue(episodes)
            def background_check():
                reply=c.call('shows.check',dict(imdb=IMDB))['result']
                if reply.get('state')=='pending':daemon.mutation_outcome(c,reply['ref'])
            background_check();q.wait(lambda:len(notify.calls(box))==1,timeout=5)
            case.check(len(releases())==3 and not records(),'the background daemon discovers a new release and notifies once without downloading',notify.calls(box))
            background_check()
            case.check(len(notify.calls(box))==1,'a completed repeat background check sends no duplicate notification',notify.calls(box))
            q.shell('summon','{"view":"shows"}');q.wait(lambda:len(shows().get('rows',[]))==3)
            q.keys(['l'],lambda:shows().get('focus')=='releases')
            newest=shows()['rows'][0]['key'];q.keys(['m'],lambda:len(shows().get('rows',[]))==2)
            case.check(not records() and next(r for r in releases() if r['key']==newest)['read'],'marking a release read changes only inbox intent','no torrent created')
            q.keys(['u'],lambda:len(shows().get('rows',[]))==3)
            case.check(not next(r for r in releases() if r['key']==newest)['read'],'Undo read restores that same release to the inbox',newest)
            q.keys(['='],lambda:(view().get('controls') or {}).get('preferenceOpen'))
            q.keys(['j','-k','Return'],lambda:not (view().get('controls') or {}).get('preferenceOpen') and followed()[0].get('quality')=='2160p')
            case.check(not records(),'the visible per-show quality dropdown durably selects 2160p without downloading',json.dumps(followed()))
            q.keys(['g','-k','Return'],lambda:add().get('visible'));q.wait(lambda:add().get('phase')=='ready',timeout=15)
            selected=shows()['rows'][shows()['cursor']]['key'];hash_=add()['hash']
            stage=c.call('torrents.get',dict(hash=hash_))['result']['torrent']
            case.check(stage['staged'] and not stage['checked'] and not next(r for r in releases() if r['key']==selected)['read'] and not list(box.downloads.iterdir()),'review stages release metadata while leaving the inbox unread and payload absent',json.dumps(stage))
            q.keys(['-k','Escape'],lambda:not add().get('visible'))
            case.check(not records() and not next(r for r in releases() if r['key']==selected)['read'],'Discard leaves no transfer and does not mark the release read','same unread release')
            q.keys(['-k','Return'],lambda:add().get('visible'));q.wait(lambda:add().get('phase')=='ready',timeout=15)
            q.keys(['-k','Return'],lambda:not add().get('visible') and len(records())==1)
            q.keys(['3'],lambda:'shows' in view())
            q.keys(['h'],lambda:shows().get('focus')=='shows')
            q.keys(['j'],lambda:shows().get('selected')==IMDB)
            q.keys(['l'],lambda:shows().get('focus')=='releases')
            q.wait(lambda:any(r['key']==selected and r['read'] and r['transfer']!='none' for r in shows().get('rows',[])))
            case.check(records()[0]['origin']=='shows' and not records()[0]['staged'],'only deliberate Download creates a transfer and marks the release read; read and transfer remain independent',json.dumps(releases()))
            q.shell('hide');q.wait(lambda:q.state().get('view') is None);c.close()
            h.stop_serve(process);process=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine();c=daemon.ApiConn(h);c.hello()
            saved=followed()[0];saved_releases=releases()
            case.check(saved['quality']=='2160p' and next(r for r in saved_releases if r['key']==selected)['read'] and len(records())==1,'a real daemon restart retains follow, preference, inbox read state and transfer association',json.dumps(dict(show=saved,releases=saved_releases)))
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"shows"}');q.wait(lambda:'shows' in view())
            case.check(shows()['shows'][0]['quality']=='2160p','the reopened dashboard renders the saved preference and inbox',json.dumps(shows()))
            q.keys(['q'],lambda:q.state().get('view') is None)
            case.check(process.child.poll() is None,'closing Shows detaches from the same running owner','private daemon survives')
        except Exception:
            if q:print('W2 state at failure: '+json.dumps(q.state()),flush=True)
            raise
        finally:
            try:
                if q:q.close()
            finally:
                if 'c' in locals():c.close()
                if process.child.poll() is None:h.stop_serve(process)
    return suite_status('qs_journey_w2')
