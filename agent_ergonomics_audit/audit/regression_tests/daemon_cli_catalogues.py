"""P3-07: counted local sources, real CLI routing and real interactive TUI."""
import json
import time
from pathlib import Path

import daemon as d
import fixtures
from daemon_catalog import cli, equal
from daemon_shows import IMDB


def atomic(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp=path.with_suffix('.case.tmp');tmp.write_bytes(value);tmp.replace(path)


def show_source(box,http):
    box.env['FBT_EZTV_BASE_URL']=http.base;box.env['FBT_IMDB_BASE_URL']=http.base
    box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
    cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
    title=dict(id=IMDB,name='Local Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')
    (cache/(IMDB+'.json')).write_text(json.dumps(title))
    names=cache.parent/'by-name-v2';names.mkdir(exist_ok=True);(names/'local_fixture.json').write_text(json.dumps(dict(id=IMDB)))
    releases=[]
    for index,episode in enumerate([1,2]):
        releases.append(dict(id=index+1,imdb_id=IMDB[2:],title=f'Local.Fixture.S01E{episode:02}.1080p',season='1',episode=str(episode),seeds=2,size_bytes='262144',date_released_unix=1792274400+index,magnet_url='magnet:?xt=urn:btih:'+f'{index+1:040x}'))
    (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=2,limit=100,page=1,torrents=releases)))


def feeds(box,http):
    text='<rss><channel><title>Local feed</title><item><guid>good</guid><title>Good fixture</title><description>Fixture description</description><pubDate>Sun, 18 Oct 2026 12:00:00 GMT</pubDate><enclosure url="'+http.base+'/owned.torrent" length="32768" type="application/x-bittorrent"/></item><item><guid>bad</guid><title>Malformed magnet</title><link>magnet:?xt=urn:btih:invalid</link></item></channel></rss>'
    for name in ['stale','fresh','new']:(box.root/(name+'.xml')).write_text(text)


def compare_commands(case,box,h,binary,process,path,baseline,commands):
    """Restore identical current-format state before each independent command."""
    outputs=[]
    with d.ApiConn(h) as observer:
        observer.hello('bar');observer.call('subscribe',dict(topics=['events']))
        for args,code,override in commands:
            atomic(path,baseline if override is None else override)
            _,result=cli(case,box,binary,*args,code=code,valid='--json' in args);outputs.append(result.stdout)
        rss=path.name=='rss.json'
        expected=sum((2 if len([x for x in args if x!='--json'])==2 else 1) for args,code,_ in commands if rss and args[:2]==['rss','refresh']) if rss else sum(1 for args,code,_ in commands if args[1] in ['check','follow'])
        def routed():
            observer.call('app.info')
            entries=[item for frame in observer.pushed if frame.get('topic')=='events' for item in frame.get('added',frame.get('items',[]))]
            observed=[entry for entry in entries if entry.get('ev')=='mutation.result' and entry.get('op')==('rss.refresh' if rss else 'shows.check')]
            return len(observed)==expected,json.dumps(dict(expected=expected,confirmed=observed))
        h.wait_for(routed,'each CLI refresh/check really completes in the running owner rather than silently falling back',timeout=3)
    h.stop_serve(process)
    for (args,code,override),expected in zip(commands,outputs):
        atomic(path,baseline if override is None else override)
        _,result=cli(case,box,binary,*args,code=code,valid='--json' in args)
        equal(case,args,expected,result.stdout)
        case.check(not d.DaemonContract.socket_path(box).exists(),'catalogue reads and edits do not start the owner',str(args))


def cli_rss_both_paths_identical(binary,systemd):
    with d.Case('daemon','cli_rss_both_paths_identical') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root) as http,d.Harness(case,binary,box,systemd) as h:
        feeds(box,http)
        for name in ['stale','fresh']:cli(case,box,binary,'rss','add',http.base+'/'+name+'.xml','--name',name.title(),'--json')
        path=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/rss.json';baseline=path.read_bytes()
        commands=[]
        for fmt in [['--json'],[]]:
            for args in [['rss','list'],['rss','items','Sta'],['rss','items','Fresh','--unread'],['rss','refresh','Stale'],['rss','refresh'],['rss','add',http.base+'/new.xml','--name','New'],['rss','remove','Fresh']]:
                commands.append((args+fmt,0,None))
        commands += [(['rss','items','missing','--json'],6,None),(['rss','items','s','--json'],7,None)]
        p=h.start_serve();h.receive_ready();h.wait_engine()
        try:
            compare_commands(case,box,h,binary,p,path,baseline,commands)
            case.check(sum(x=='/stale.xml' for x in http.requests)>=5,'both explicit refresh paths really fetched the source',str(http.requests))
        finally:
            if p.child.poll() is None:h.stop_serve(p)


def cli_shows_both_paths_identical(binary,systemd):
    with d.Case('daemon','cli_shows_both_paths_identical') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        show_source(box,http)
        with d.Harness(case,binary,box,systemd) as h:
            cli(case,box,binary,'shows','follow',IMDB,'--json')
            path=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/shows.json';state=json.loads(path.read_text())
            state['shows'][0]['added']=1792270000;state['shows'][0]['last_checked']=1792274400
            baseline=json.dumps(state).encode();empty=json.dumps(dict(shows=[])).encode()
            commands=[]
            for fmt in [['--json'],[]]:
                for args,override in [(['shows','list'],None),(['shows','episodes','Local'],None),(['shows','episodes',IMDB,'--new'],None),(['shows','follow',IMDB],None),(['shows','follow',IMDB],empty),(['shows','unfollow','Local'],None),(['shows','seen',IMDB,'s01e01'],None),(['shows','seen',IMDB,'all'],None),(['shows','check','Local'],None),(['shows','check'],None)]:
                    commands.append((args+fmt,0,override))
            ambiguous=json.loads(baseline);other=dict(ambiguous['shows'][0],imdb='tt1234002',title='Local Other');ambiguous['shows'].append(other)
            commands += [(['shows','episodes','missing','--json'],6,None),(['shows','episodes','Local','--json'],7,json.dumps(ambiguous).encode()),(['shows','seen',IMDB,'S99E99','--json'],6,None)]
            p=h.start_serve();h.receive_ready();h.wait_engine()
            try:
                compare_commands(case,box,h,binary,p,path,baseline,commands)
            finally:
                if p.child.poll() is None:h.stop_serve(p)
            # A real malformed upstream must retain the synchronous all-failed exit 5.
            (box.root/'episodes.json').write_text('{malformed')
            h.notify.close();p=h.start_serve();h.receive_ready();h.wait_engine()
            try:compare_commands(case,box,h,binary,p,path,baseline,[(['shows','check','--json'],5,None),(['shows','check'],5,None)])
            finally:
                if p.child.poll() is None:h.stop_serve(p)


def cli_catalogue_grabs(binary,systemd):
    # The absent-owner half starts only this runtime-linked, sandboxed unit.
    with d.Case('daemon','cli_catalogue_grabs') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        feeds(box,http);show_source(box,http)
        payload=box.root/'owned.bin';payload.write_bytes(b'x'*32768)
        torrent=fixtures.torrent_fixture(box.root,[payload]);(box.root/'owned.torrent').write_bytes(torrent.torrent.read_bytes())
        cli(case,box,binary,'rss','add',http.base+'/stale.xml','--name','Local','--json')
        cli(case,box,binary,'shows','follow',IMDB,'--json')
        data=Path(box.env['XDG_DATA_HOME'])/'fbittorrent';paths=[data/'rss.json',data/'shows.json'];baselines=[p.read_bytes() for p in paths]
        with d.Harness(case,binary,box,systemd) as h,d.InstalledUnit(h) as unit:
            unit.autostart();outputs=[]
            commands=[['rss','grab','Local','good'],['shows','grab',IMDB,'S01E01']]
            unit.control('start',box.unit);h.wait_engine()
            for args in commands:
                for fmt in [['--json'],[]]:
                    for path,value in zip(paths,baselines):atomic(path,value)
                    value,result=cli(case,box,binary,*args,*fmt,valid=bool(fmt));outputs.append((args+fmt,result.stdout))
                    with d.ApiConn(h) as c:
                        c.hello();rows=c.call('torrents.list',{})['result']['torrents']
                        case.check(any(row.get('origin')==('rss' if args[0]=='rss' else 'shows') for row in rows),'grab reaches the typed origin operation before marking read',json.dumps(rows))
            unit.control('stop',box.unit)
            for args,expected in outputs:
                for path,value in zip(paths,baselines):atomic(path,value)
                case.check(not d.DaemonContract.socket_path(box).exists(),'grab begins with no owner process',str(args))
                _,result=cli(case,box,binary,*args,valid='--json' in args)
                unit.verify_child();equal(case,args,expected,result.stdout)
                unit.control('stop',box.unit)
            unit.no_launcher()


def cli_catalogue_chunked(binary,systemd):
    import hashlib
    import subprocess
    with d.api_owned('cli_catalogue_chunked',binary,systemd) as (case,box,h,p),fixtures.HttpFixture(box.root) as http:
        (box.root/'large.xml').write_text('<rss><channel>'+''.join('<item><guid>'+str(i)+'</guid><title>Row '+str(i)+'</title><description>'+('x'*2300)+'</description></item>' for i in range(500))+'</channel></rss>')
        with d.ApiConn(h) as c:
            c.hello();feed=c.call('rss.add',dict(url=http.base+'/large.xml',name='Large'))['result']['feed']
            c.call('rss.items',dict(feed=feed['id']),error='invalid_message')
        outputs=[]
        for local in [False,True]:
            if local:h.stop_serve(p)
            args=[str(binary),'rss','items','Large','--json']
            result=subprocess.run(args,env=box.env,capture_output=True,text=True,timeout=20)
            value=json.loads(result.stdout)
            artifact=case.path.with_suffix('.local.stdout' if local else '.api.stdout');artifact.write_text(result.stdout)
            case.step('read oversized RSS catalogue','500 full descriptions survive bounded API chunks and local rendering').close(result.returncode==0 and value['count']==500 and all(len(item['description'])==2300 for item in value['items']),json.dumps(dict(local=local,code=result.returncode,bytes=len(result.stdout),count=value['count'])),artifacts=[artifact])
            outputs.append(result.stdout)
        case.check(outputs[0]==outputs[1],'oversized collection output agrees byte-for-byte',json.dumps(dict(sha256=[hashlib.sha256(x.encode()).hexdigest() for x in outputs])))


def tui_open_one_refresh(binary,systemd):
    import native
    with d.Case('daemon','tui_open_one_refresh') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        feeds(box,http);show_source(box,http)
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve();h.receive_ready();h.wait_engine()
            try:
                with d.ApiConn(h) as c:
                    c.hello('bar')
                    for name in ['stale','fresh']:c.call('rss.add',dict(url=http.base+'/'+name+'.xml',name=name.title()))
                    c.call('shows.follow',dict(imdb=IMDB))
                    path=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/rss.json';state=json.loads(path.read_text())
                    for row in state['feeds']:
                        if row['feed']['name']=='Stale':row['last_checked']=int(time.time())-1801
                    atomic(path,json.dumps(state).encode())
                    counts=lambda:{name:sum(x=='/'+name+'.xml' for x in http.requests) for name in ['stale','fresh']}
                    tui=native.AttachedTui(case,str(binary),box,h,session='fbt-fbt-p3-07-c2p')
                    try:
                        for lap in range(2):
                            target,done=tui.start();tui.keys(target,'4');tui.assert_pane(target,'Fresh')
                            h.wait_for(lambda:(counts()==dict(stale=2,fresh=1),json.dumps(counts())),'only stale feed refreshes on first TUI open',timeout=5)
                            time.sleep(.4)
                            case.check(counts()==dict(stale=2,fresh=1),'fresh reconnect and initial file load do not fetch a second time',json.dumps(dict(lap=lap,hits=counts(),requests=http.requests)))
                            tui.keys(target,'3');tui.assert_pane(target,'Local Fixture');tui.assert_pane(target,'S01E02')
                            case.check(sum(x.startswith('/api/get-torrents?') for x in http.requests)==1,'daemon alone checks stale Shows once across TUI opens',str(http.requests))
                            if lap==0:
                                c.call('rss.add',dict(url=http.base+'/new.xml',name='Pushed feed'))
                                tui.keys(target,'4');tui.assert_pane(target,'Pushed feed')
                                cli(case,box,binary,'shows','seen',IMDB,'all','--json')
                                tui.keys(target,'3')
                                h.wait_for(lambda:('S01E02' not in tui.capture(target),tui.capture(target)),'pushed Shows read changes rebuild the cached inbox')
                                cli(case,box,binary,'shows','seen',IMDB,'S01E02','--json')
                                tui.assert_pane(target,'S01E02')
                            tui.keys(target,'q');h.wait_for(lambda:(done.exists(),str(done)),'TUI quits while the owner keeps running')
                            case.check(p.child.poll() is None,'q closes only the interactive client',str(p.child.poll()))
                    finally:tui.close()
            finally:
                if p.child.poll() is None:h.stop_serve(p)


CASES=(cli_rss_both_paths_identical,cli_shows_both_paths_identical,cli_catalogue_grabs,cli_catalogue_chunked,tui_open_one_refresh)
