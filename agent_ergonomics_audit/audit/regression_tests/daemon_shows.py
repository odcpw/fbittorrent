"""P3-03: actual EZTV responses, inbox intents, CLI reload and idle silence."""
import json
import time
import threading
import socket
import daemon as d
import fixtures

IMDB='tt1234001'
KEYS=['S02','S02E01','S01E02','S01E01']


def run(name,binary,systemd):
    with d.Case('daemon',name) as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        box.env['FBT_EZTV_BASE_URL']=http.base;box.env['FBT_IMDB_BASE_URL']=http.base
        box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
        cache=d.Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        title=dict(id=IMDB,name='Local Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')
        (cache/(IMDB+'.json')).write_text(json.dumps(title))
        named=cache.parent/'by-name-v2';named.mkdir(exist_ok=True);(named/'local_fixture.json').write_text(json.dumps(dict(id=IMDB)))
        releases=[]
        for index,(season,episode) in enumerate([(1,1),(1,2),(2,1),(2,0)]):
            key=f'S{season:02}'+(f'E{episode:02}' if episode else '')
            releases.append(dict(id=index+1,imdb_id=IMDB[2:],title='Local.Fixture.'+key+'.1080p',season=str(season),episode=str(episode),seeds=2,size_bytes='262144',date_released_unix=1792274400+index,magnet_url='magnet:?xt=urn:btih:'+f'{index+1:040x}'))
        releases.reverse()
        (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=4,limit=100,page=1,torrents=releases)))
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            try:
                with d.ApiConn(h) as c:
                    c.hello('bar');c.call('subscribe',dict(topics=['shows','shows.releases:inbox','events']))
                    def hits():return sum(path.startswith('/api/get-torrents?') for path in http.requests)
                    if name=='shows_cli_edit_visible':
                        # Timestamp actual socket receipt independently of
                        # JSONL fsync/poll delays under shared build load.
                        with d.ApiConn(h) as subscriber:
                            subscriber.hello('bar');subscriber.call('subscribe',dict(topics=['shows']))
                            subscriber.socket.settimeout(.1)
                            frames=[];arrivals=[];errors=[];exit_at=[];stop=threading.Event();ready=threading.Event()
                            def receive():
                                ready.set()
                                while not stop.is_set():
                                    try:frame=subscriber.receive()
                                    except socket.timeout:continue
                                    except (OSError,ValueError) as e:
                                        if not stop.is_set():errors.append(str(e))
                                        return
                                    if frame is None:return
                                    at=time.monotonic()
                                    if len(frames)>=64:errors.append('bounded frame capture exceeded');return
                                    frames.append(dict(at=at,frame=frame))
                                    if frame.get('topic')=='shows' and any(row.get('imdb')==IMDB for row in frame.get('added',frame.get('items',[]))):arrivals.append(at)
                            reader=threading.Thread(target=receive);reader.start();ready.wait(1)
                            try:
                                result=h.cli('shows','follow',IMDB,on_exit=exit_at.append)
                                h.wait_for(lambda:(bool(arrivals),json.dumps(dict(arrivals=arrivals,cli_exit=exit_at,errors=errors))),'observe the independently timestamped Shows frame',timeout=6)
                                elapsed=min(arrivals)-exit_at[0] if arrivals and exit_at else float('inf')
                                case.check(not errors and elapsed<=2,'in-process CLI follow arrives in pushed Shows within the unchanged two-second budget',json.dumps(dict(elapsed=elapsed,arrivals=arrivals,cli_exit=exit_at,frames=frames,cli=result,hits=hits())))
                            finally:
                                stop.set();reader.join(timeout=2)
                                case.check(not reader.is_alive(),'bounded topic receiver stops before sandbox cleanup',str(reader.is_alive()))
                    else:
                        c.call('shows.follow',dict(query='Local Fixture',quality='1080p'))
                        case.check(hits()==0,'following stores intent without an implicit EZTV check',str(http.requests))
                    def all_events():
                        c.call('app.info');out=[]
                        for frame in c.pushed:
                            if frame.get('ev') in ['shows.checked','mutation.result']:out.append(frame)
                            if frame.get('topic')=='events':out.extend(frame.get('added',frame.get('items',[])))
                        return out
                    def check():
                        pending=c.call('shows.check',dict(imdb=IMDB))['result'];found=[]
                        def finished():
                            found[:]=[ev for ev in all_events() if ev.get('ev')=='mutation.result' and ev.get('ref')==pending['ref']]
                            return bool(found),json.dumps(found)
                        h.wait_for(finished,'pending Shows check completes with a real mutation outcome',timeout=5)
                        case.check(found[-1].get('outcome')=='done' and 'new' in found[-1].get('result',{}),'check completion returns the discovered count',json.dumps(found[-1]))
                        checked=[ev for ev in all_events() if ev.get('ev')=='shows.checked']
                        case.check(bool(checked) and checked[-1].get('imdb')==IMDB,'shows.checked includes identity, new count and warnings',json.dumps(checked))
                    def inbox(expected):
                        value=c.call('shows.releases',dict(inbox=True))['result']['releases']
                        show=c.call('shows.list')['result']['shows'][0]
                        case.check(len(value)==expected and show['unread']==expected,'inbox and show unread count agree after the operation',json.dumps(dict(show=show,releases=value)))
                        return value
                    if name=='shows_check_only_when_asked':
                        c.call('app.client',dict(interactive=True)) # A bar cannot become interactive.
                        began=time.monotonic()
                        for second in range(60):
                            time.sleep(max(0,began+second+1-time.monotonic()));c.call('app.info')
                            case.step('idle EZTV counter second '+str(second+1),'zero requests while no interactive UI exists').close(hits()==0,json.dumps(dict(seconds=time.monotonic()-began,hits=hits(),requests=http.requests)))
                        with d.ApiConn(h) as opened:
                            opened.call('hello',dict(proto=1,client=dict(kind='tui',name='shows-open',version='0'),interactive=True))
                            h.wait_for(lambda:(hits()==1,json.dumps(dict(hits=hits(),frames=c.pushed))),'stale interactive connection initiates one check',timeout=5)
                            h.wait_for(lambda:((rows:=c.call('shows.list')['result']['shows'])[0]['counts']['episodes']==4 and not rows[0]['checking'],json.dumps(rows)),'interactive check populates the stored catalogue')
                        with d.ApiConn(h) as fresh:
                            fresh.call('hello',dict(proto=1,client=dict(kind='tui',name='shows-fresh',version='0'),interactive=True));time.sleep(.3)
                        case.check(hits()==1,'fresh reconnect schedules no duplicate request',str(http.requests))
                        check();case.check(hits()==2,'explicit check works while nobody is interactive',str(http.requests))
                    else:
                        before=c.call('torrents.list',{})['result']['torrents'];check()
                        after=c.call('torrents.list',{})['result']['torrents']
                        case.check(before==after==[],'check changes neither ledger size nor transfer intent',json.dumps(dict(before=before,after=after,hits=hits())))
                        values=inbox(4)
                        case.check([row['key'] for row in values]==KEYS and all(row['transfer']=='none' and not row['read'] for row in values),'four unread releases are newest first and have independent empty transfer state',json.dumps(values))
                        if name=='shows_inbox_semantics':
                            c.call('shows.set_read',dict(imdb=IMDB,key='S01E01',read=True));inbox(3)
                            c.call('shows.mark_show_read',dict(imdb=IMDB));inbox(0)
                            restored=c.call('shows.undo_read')['result'];values=inbox(1)
                            case.check(restored==dict(restored=dict(imdb=IMDB,key='S01E01')) and values[0]['key']=='S01E01','undo restores the exact individually marked release after bulk read',json.dumps(restored))
                            case.check(c.call('shows.undo_read')['result']=={},'undo has only one level','second undo is empty')
                            added=c.call('shows.grab',dict(imdb=IMDB,key='S01E01',stopped=True))['result'];inbox(0)
                            row=c.call('torrents.get',dict(hash=added['hash']))['result']['torrent']
                            values=c.call('shows.releases',dict(imdb=IMDB,unread_only=False))['result']['releases']
                            grabbed=next(r for r in values if r['key']=='S01E01')
                            case.check(row.get('origin')=='shows' and grabbed['read'] and grabbed['transfer']=='paused','explicit grab alone adds a paused record and read intent with Shows origin',json.dumps(dict(add=added,row=row,release=grabbed)))
                            # External CLI changes land through inode/mtime invalidation too.
                            result=h.cli('shows','seen',IMDB,'S01E02')
                            case.step('read history through the in-process CLI','shared store edit remains visible to daemon clients').ok(json.dumps(result))
                    case.step('retain Shows topic and counter evidence','snapshots/deltas, read flags and EZTV hit counts remain reviewable').ok(json.dumps(dict(frames=c.pushed,hits=hits(),requests=http.requests)))
            finally:
                if p.child.poll() is None:h.stop_serve(p)


def shows_inbox_semantics(binary,systemd):run('shows_inbox_semantics',binary,systemd)
def shows_check_never_downloads(binary,systemd):run('shows_check_never_downloads',binary,systemd)
def shows_check_only_when_asked(binary,systemd):run('shows_check_only_when_asked',binary,systemd)
def shows_cli_edit_visible(binary,systemd):run('shows_cli_edit_visible',binary,systemd)
