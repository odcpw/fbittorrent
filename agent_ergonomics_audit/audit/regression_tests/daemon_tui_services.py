"""P4-03: real service screens over one API socket, with unusable client store files."""
import json
from pathlib import Path
from types import SimpleNamespace
import time
import daemon as d
import fixtures
import native

IMDB='tt1234001'


def tui_service_screens(binary,systemd):
    with d.Case('daemon','tui_service_screens') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        box.env.update(FBT_EZTV_BASE_URL=http.base,FBT_IMDB_BASE_URL=http.base)
        box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        (cache/(IMDB+'.json')).write_text(json.dumps(dict(id=IMDB,name='Local Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')))
        payload=box.root/'service.bin';payload.write_bytes(b'service screen fixture'*2048)
        torrent=fixtures.torrent_fixture(box.root,[payload])
        magnet='magnet:?xt=urn:btih:'+torrent.info_hash
        (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=2,limit=100,page=1,torrents=[dict(id=i,imdb_id=IMDB[2:],title=f'Local.Fixture.S01E0{i}.1080p',season='1',episode=str(i),seeds=2,size_bytes='45056',date_released_unix=1792274400+i,magnet_url='magnet:?xt=urn:btih:'+f'{i:040x}')for i in (2,1)])))
        (box.root/'feed.xml').write_text('<rss><channel><title>Fixture Feed</title><item><guid>release</guid><title>RSS Release Alpha</title><enclosure url="'+http.base+'/'+torrent.torrent.name+'" type="application/x-bittorrent"/></item></channel></rss>')
        client_data=box.root/'client-data';(client_data/'fbittorrent').mkdir(parents=True)
        for name in ('shows.json','rss.json'):(client_data/'fbittorrent'/name).write_text('intentionally invalid client-local JSON')
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(idle_exit_secs=0)
            if not(h.receive_ready()and h.wait_engine()):raise RuntimeError('private service daemon did not start')
            ui=None
            try:
                with d.ApiConn(h) as c:
                    c.hello('bar')
                    before=c.call('app.stats')['result']['active_clients']['tui']
                    hidden=h.engine_path.with_name('unused-engine.sock');h.engine_path.rename(hidden)
                    ui=native.AttachedTui(case,str(binary),SimpleNamespace(root=box.root,env=dict(box.env,XDG_DATA_HOME=str(client_data))),h,session='fbt-fbt-p4-03-r5p-services')
                    target,done=ui.start()
                    ui.keys(target,'4');ui.keys(target,'n');ui.keys(target,http.base+'/feed.xml');ui.keys(target,'Enter',literal=False)
                    ui.assert_pane(target,'127.0.0.1')
                    def feed_ready():
                        feeds=c.call('rss.feeds')['result']['feeds'];return len(feeds)==1 and not feeds[0]['loading'],json.dumps(feeds)
                    h.wait_for(feed_ready,'the TUI add-feed intent is stored/fetched by the daemon')
                    feed=c.call('rss.feeds')['result']['feeds'][0]
                    ui.assert_pane(target,'RSS Release Alpha');ui.keys(target,'Enter',literal=False);ui.assert_pane(target,'[Items]')
                    ui.keys(target,'m')
                    def read(expected):
                        rows=c.call('rss.items',dict(feed=feed['id']))['result']['items'];return rows[0]['read']==expected,json.dumps(rows)
                    h.wait_for(lambda:read(True),'TUI marks the actual API item read')
                    ui.assert_pane(target,'✓ Read')
                    c.call('rss.set_read',dict(feed=feed['id'],item='release',read=False))
                    ui.assert_pane(target,'● Unread')
                    case.check(c.call('torrents.list',{})['result']['torrents']==[],'reading and refreshing services never add torrents','empty engine ledger')
                    ui.keys(target,'Enter',literal=False)
                    h.wait_for(lambda:((rows:=c.call('torrents.list',{})['result']['torrents'])and rows[0].get('origin')=='rss',json.dumps(rows)),'RSS Enter grabs with daemon-owned rss origin')
                    ui.keys(target,'3');ui.keys(target,'n');ui.keys(target,IMDB);ui.keys(target,'Enter',literal=False)
                    ui.assert_pane(target,'Local Fixture')
                    ui.keys(target,'R')
                    def checked():
                        rows=c.call('shows.list')['result']['shows'];return rows and rows[0]['counts']['episodes']==2 and not rows[0]['checking'],json.dumps(rows)
                    h.wait_for(checked,'explicit TUI check discovers the real local EZTV releases')
                    ui.keys(target,'Home',literal=False);ui.keys(target,'Enter',literal=False)
                    ui.assert_pane(target,'S01E02')
                    ui.keys(target,'m')
                    def inbox_count(n):
                        rows=c.call('shows.releases',dict(inbox=True))['result']['releases'];return len(rows)==n,json.dumps(rows)
                    h.wait_for(lambda:inbox_count(1),'TUI read removes exactly the selected show/release from the daemon inbox')
                    ui.assert_pane(target,'(u) Undo read');ui.keys(target,'u');h.wait_for(lambda:inbox_count(2),'TUI Undo restores the daemon-owned read target')
                    ui.keys(target,'Left',literal=False);ui.keys(target,'Down',literal=False);ui.keys(target,'Enter',literal=False)
                    ui.assert_pane(target,'S01E02')
                    c.call('shows.set_read',dict(imdb=IMDB,key='S01E02',read=True))
                    ui.assert_pane(target,'✓ Read')
                    ui.keys(target,'M')
                    h.wait_for(lambda:inbox_count(0),'TUI mark-show-read updates the shared history')
                    stats=c.call('app.stats')['result'];case.check(before==0 and stats['active_clients']['tui']==1,'all service reads/mutations/subscriptions use one TUI connection',json.dumps(stats))
                    case.check(all((client_data/'fbittorrent'/name).read_text()=='intentionally invalid client-local JSON'for name in ('shows.json','rss.json')),
                               'native TUI neither loads nor overwrites its unusable local service files',str(client_data))
                    case.step('retain panes, service frames and loopback request counts','service integration observations remain reviewable').ok(json.dumps(dict(pane=ui.capture(target),frames=c.pushed,requests=http.requests)))
                    ui.keys(target,'q');h.wait_for(lambda:(done.exists()and done.read_text()=='0'and p.child.poll()is None,str(done)),'q detaches while service owner survives')
            finally:
                if ui:ui.close()
                if hidden.exists()and not h.engine_path.exists():hidden.rename(h.engine_path)
                if p.child.poll()is None:h.stop_serve(p)


CASES=(tui_service_screens,)
