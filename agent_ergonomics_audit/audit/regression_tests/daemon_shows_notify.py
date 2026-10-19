"""P6-10: real EZTV checks and recording notifications, never a desktop window."""
import json
import time
import daemon as d
import daemon_notify as notify
import fixtures

IMDB='tt1234001'


def run(name,binary,systemd):
    with d.Case('daemon',name) as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        box.env['FBT_EZTV_BASE_URL']=http.base;box.keys+=['FBT_EZTV_BASE_URL']
        notify.config(box,notify=True);notify.shims(box)
        cache=d.Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        (cache/(IMDB+'.json')).write_text(json.dumps(dict(id=IMDB,name='Local<& Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')))
        def releases(count):
            rows=[dict(id=i,imdb_id=IMDB[2:],title=f'Local.Fixture.S01E{i:02}.1080p',season='1',episode=str(i),seeds=2,size_bytes='262144',date_released_unix=1792274400+i,magnet_url='magnet:?xt=urn:btih:'+f'{i:040x}') for i in range(count,0,-1)]
            (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=count,limit=100,page=1,torrents=rows)))
            case.step('prepare local EZTV releases','only private fixture releases are checked').ok(json.dumps(rows))
        releases(5)
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as c:
                c.hello('bar');c.call('subscribe',dict(topics=['events']))
                c.call('shows.follow',dict(imdb=IMDB))
                def check(expected):
                    pending=c.call('shows.check',dict(imdb=IMDB))['result']
                    result=d.mutation_outcome(c,pending['ref'])
                    case.check(result.get('result',{}).get('new')==expected,'check reports the real number of new fixture releases',json.dumps(result))
                if name=='shows_notify_suppressed_interactive':
                    with d.ApiConn(h) as ui,d.ApiConn(h) as dashboard:
                        ui.call('hello',dict(proto=1,client=dict(kind='tui',name='shows-notify-tui',version='0'),interactive=True))
                        h.wait_for(lambda:((rows:=c.call('shows.list')['result']['shows'])[0]['counts']['episodes']==5 and not rows[0]['checking'],json.dumps(rows)),'stale interactive connect checks the actual fixture')
                        time.sleep(.1)
                        case.check(not notify.calls(box),'interactive stale check emits no desktop notification',json.dumps(notify.calls(box)))
                        releases(6);check(1)
                        time.sleep(.1)
                        case.check(not notify.calls(box),'explicit check is also suppressed while a TUI is interactive',json.dumps(notify.calls(box)))
                        ui.call('app.client',dict(interactive=False))
                        dashboard.call('hello',dict(proto=1,client=dict(kind='bar',name='shows-notify-dashboard',version='0'),interactive=True))
                        dashboard.call('app.client',dict(interactive=True,dashboard_visible=True))
                        releases(7);check(1)
                        time.sleep(.1)
                        case.check(not notify.calls(box),'visible dashboard on the shared bar connection suppresses desktop notifications',json.dumps(notify.calls(box)))
                        dashboard.call('app.client',dict(interactive=False,dashboard_visible=False))
                        releases(8);check(1)
                    notify.await_calls(h,box,1)
                    calls=notify.observe(case,box)
                    case.check(len(calls)==1 and calls[0][-2:]==['1 new episode','Local&lt;&amp; Fixture S01E08'],'hiding the last interactive UI lets the next check notify once',json.dumps(calls))
                else:
                    check(5);notify.await_calls(h,box,1)
                    calls=notify.observe(case,box)
                    case.check(len(calls)==1 and calls[0][-2:]==['5 new episodes','Local&lt;&amp; Fixture S01E05, Local&lt;&amp; Fixture S01E04, Local&lt;&amp; Fixture S01E03, + 2 more'] and 'window=Open Shows' in calls[0],
                               'one notification names at most three new unread releases, escapes markup and offers the Shows action',json.dumps(calls))
                    if name=='shows_notify_none_when_unchanged':
                        check(0);time.sleep(.15)
                        case.check(len(notify.calls(box))==1,'a second unchanged EZTV check sends no notification',json.dumps(notify.calls(box)))
                rows=c.call('torrents.list',{})['result']['torrents']
                case.check(rows==[],'Shows checks and notifications never start a transfer',json.dumps(rows))
            h.stop_serve(p)


def shows_notify(binary,systemd):run('shows_notify',binary,systemd)
def shows_notify_none_when_unchanged(binary,systemd):run('shows_notify_none_when_unchanged',binary,systemd)
def shows_notify_suppressed_interactive(binary,systemd):run('shows_notify_suppressed_interactive',binary,systemd)
