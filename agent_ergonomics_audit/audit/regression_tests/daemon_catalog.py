"""P3-07 CLI source agreement through real Nova/IMDb and owned daemons."""
import gzip
import json
from pathlib import Path
import shutil
import socket
import threading
import time

import daemon as d
from daemon_search import groups, live, owned as search_owned


def cli(*args, **kwargs):
    # daemon registers these cases while importing; load the CLI helper only
    # when a case runs so direct agent_stream imports remain acyclic.
    from agent_contract import cli as run
    return run(*args, **kwargs)


def engines(box, names=('shape',)):
    root=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/nova3/engines'
    root.mkdir(parents=True,exist_ok=True)
    for name in names:
        shutil.copyfile(Path(__file__).parent/'fixtures/providers'/(name+'.py'),root/(name+'.py'))
    return root.parent


def equal(case,args,first,second):
    if '--json' in args:
        first,second=json.loads(first),json.loads(second)
        first.pop('warnings',None);second.pop('warnings',None)
    else:
        first,second=first.split(),second.split()
    case.check(first==second,'daemon/local catalogue observations agree for '+repr(args),
               json.dumps(dict(up=first,down=second),ensure_ascii=False))


def cli_search_both_paths_identical(binary,systemd):
    with search_owned('cli_search_both_paths_identical',binary,systemd,providers=('shape',)) as (case,box,h,process,runtime):
        commands=[['search','Sintel','--wait','5','--limit','3','--json'],['search','Sintel','--wait','5','--limit','3'],
                  ['search','--plugins','--json'],['search','--plugins'],
                  ['search','Sintel','--wait','5','--limit','2','--json'],['search','empty','--wait','5','--json'],
                  ['search','fail','--wait','5','--json']]
        up=[]
        with d.ApiConn(h) as observer:
            observer.hello();observer.call('subscribe',dict(topics=['events']))
            for args in commands:
                _,result=cli(case,box,binary,*args,valid='--json' in args,code=5 if 'fail' in args else 0)
                up.append(result.stdout)
                if 'fail' in args:case.check('WARN search nova:' in result.stderr,'all-failed search retains provider diagnostic on stderr',result.stderr)
            def owned_events():
                observer.call('app.info',{})
                events=[entry for frame in observer.pushed for entry in frame.get('items',frame.get('added',[])) if entry.get('ev')=='search.done']
                return len(events)==5,json.dumps(events)
            h.wait_for(owned_events,'five CLI searches really completed in the daemon, including empty/failure')
            info=observer.call('app.info',{})['result']
            case.check(info['subscriptions']['total']==1,'CLI releases every temporary search/events subscription',json.dumps(info['subscriptions']))
        h.stop_serve(process)
        for args,expected in zip(commands,up):
            _,result=cli(case,box,binary,*args,valid='--json' in args,code=5 if 'fail' in args else 0)
            equal(case,args,expected,result.stdout)
        case.check(not h.engine_path.exists(),'local search never starts a daemon',str(h.engine_path))
        # The real local provider is the oracle: names, sizes, seeds and links
        # originate there, independently of either CLI/API projection.
        rows=json.loads(up[0])['results']
        expected=[('Sintel.2010.1080p.mkv',4096,9,'1'),('Big.Buck.Bunny.720p.mp4',8192,7,'2'),('Linux.iso',1024,0,'3')]
        case.check([(r['name'],r['size'],r['seeds']) for r in rows]==[(n,s,c) for n,s,c,_ in expected]
                   and all(r['providers']==['shape'] and r['magnet']=='magnet:?xt=urn:btih:'+digit*40
                           and r['leechers']==1 for r,(_,_,_,digit) in zip(rows,expected)),
                   'CLI search retains provider measurements, identity and provenance in API rows',json.dumps(rows))
        case.check(all(name in up[1] for name,_,_,_ in expected),'plain search names every provider result',up[1])


def cli_search_dedupe_both_paths(binary,systemd):
    with search_owned('cli_search_dedupe_both_paths',binary,systemd) as (case,box,h,process,runtime):
        value,up=cli(case,box,binary,'search','Sintel','--wait','5','--limit','1','--json')
        row=value['results'][0]
        case.check(value['complete'] is True and value['count']==1 and row['name']=='辛特尔' and row['seeds']==9 and row['providers']==['alpha','beta'] and row['provider_url']=='https://beta.invalid',
                   'shared normalization repairs text, dedupes before limit and keeps the winning provider',json.dumps(value,ensure_ascii=False))
        h.stop_serve(process)
        _,down=cli(case,box,binary,'search','Sintel','--wait','5','--limit','1','--json')
        equal(case,['--json'],up.stdout,down.stdout)


def cli_search_timeout_cleans_provider(binary,systemd):
    with search_owned('cli_search_timeout_cleans_provider',binary,systemd,providers=('alpha',)) as (case,box,h,process,runtime):
        for mode in ['daemon','local']:
            if mode=='local':h.stop_serve(process)
            value,result=cli(case,box,binary,'search','hold-cli','--wait','1','--json')
            case.check(value['complete'] is False and value['count']==1,'bounded wait returns partial results in '+mode,result.stdout)
            h.wait_for(lambda:(bool(groups(runtime)) and all(not live(g['pgid']) for g in groups(runtime)),json.dumps(groups(runtime))),
                       'CLI cleans its entire provider process group in '+mode)


def cli_search_parked_fallback(binary,systemd):
    with d.Case('daemon','cli_search_parked_fallback') as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        config=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        d.select_replay_interface(config,'fbmissing0');runtime=engines(box,('alpha',))
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as observer:
                hello=observer.hello()['result'];case.check(hello['state']=='parked','owner is really parked before fallback',json.dumps(hello))
                value,result=cli(case,box,binary,'search','hold-refused','--wait','5','--json',code=1)
                case.check(value.get('ok') is False and 'Nova search VPN interface unavailable' in result.stdout
                           and not (runtime/'alpha-raw.txt').exists() and not groups(runtime),
                           'parked fallback refuses unbound Nova before any provider starts',result.stdout)
                sub=observer.call('subscribe',dict(topics=['events']))['result']['sub']
                from daemon_bus import RevisionStream
                frames=RevisionStream(observer).snapshot(sub,['events'])
                events=[item for frame in frames for item in frame.get('items',[])]
                case.check(not any(e.get('ev')=='search.done' for e in events),'parked fallback never creates an owner search session',json.dumps(events))
                # The generic fixture shutdown asserts a live engine drain;
                # restore that precondition after the parked refusal control.
                observer.call('app.settings.set',dict(network_interface='lo'))
                h.wait_for(lambda:((value:=observer.call('app.info')['result'])['state']=='running',json.dumps(value)),
                           'restore the fixture engine after the parked catalogue control')
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def cli_search_silent_hello_fallback(binary,systemd):
    # A real stalled local stream exercises the 200 ms hello deadline, without
    # returning fabricated API data or contacting a service outside this sandbox.
    with d.Case('daemon','cli_search_silent_hello_fallback') as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        # Keep an unbound fallback control for these local-only providers.
        d.select_replay_interface(Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml',None)
        engines(box);path=d.DaemonContract.socket_path(box);path.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
        listener=socket.socket(socket.AF_UNIX);listener.bind(str(path));listener.listen(1);listener.settimeout(5)
        stop=threading.Event()
        def silent():
            connection,_=listener.accept()
            with connection:stop.wait(5)
        thread=threading.Thread(target=silent);thread.start()
        try:
            began=time.monotonic();value,_=cli(case,box,binary,'search','Sintel','--wait','5','--json')
            elapsed=time.monotonic()-began
            case.check(value['count']==3 and elapsed<2,'silent hello falls back after its 200 ms probe instead of waiting for an engine',json.dumps(dict(elapsed_s=elapsed,value=value)))
        finally:stop.set();listener.close();thread.join(6);path.unlink(missing_ok=True)
        case.check(not thread.is_alive() and not h.engine_path.exists(),'silent fixture drains and no daemon was launched',str(h.engine_path))


def cli_imdb_both_paths_identical(binary,systemd):
    with d.imdb_owned('cli_imdb_both_paths_identical',binary,systemd,with_process=True) as (case,box,h,source,directory,cache,original):
        commands=[['imdb','Local Fixture','--json'],['imdb','Local Fixture'],['imdb','tt1234001','--json']]
        outputs=[]
        for args in commands:
            _,result=cli(case,box,binary,*args,valid='--json' in args);outputs.append(result.stdout)
        card=json.loads(outputs[0])['card']
        case.check(card['id']=='tt1234001' and card['title']=='Local Fixture' and card['year']==2026
                   and card['kind']=='tvSeries' and abs(card['rating']-8.2)<.00001
                   and card['url']=='https://www.imdb.com/title/tt1234001/' and Path(card['poster_path']).is_file(),
                   'CLI card retains the local source identity, rating and usable poster',json.dumps(card))
        h.wait_for(lambda:(h.stderr().count('imdb index loaded')==1,h.stderr()),'CLI lookups use the daemon index once',timeout=2)
        # The original helper owns this process; close it before local runs.
        h.stop_serve(original)
        for args,expected in zip(commands,outputs):
            _,result=cli(case,box,binary,*args,valid='--json' in args);equal(case,args,expected,result.stdout)
        h.notify.close();process=h.start_serve();h.receive_ready();h.wait_engine()
        try:
            cli(case,box,binary,'imdb','tt1234001','--json')
            h.wait_for(lambda:(h.stderr().count('imdb index loaded')==2,h.stderr()),'new owner has loaded exactly one additional index before refresh',timeout=2)
            before=h.stderr().count('imdb index loaded')
            changed=b'tconst\taverageRating\tnumVotes\ntt1234001\t9.1\t4321\n'
            (directory/'ratings.tsv.gz').write_bytes(gzip.compress(changed,mtime=0))
            refreshed,result=cli(case,box,binary,'imdb','tt1234001','--refresh','--json')
            h.wait_for(lambda:(h.stderr().count('imdb index loaded')==before+1,h.stderr()),'forced refresh log has reached the captured stderr',timeout=2)
            case.check(abs(refreshed['card']['rating']-9.1)<.00001 and refreshed['card']['votes']==4321 and h.stderr().count('imdb index loaded')==before+1,
                       '--refresh replaces the running daemon index exactly once',json.dumps(dict(result=refreshed,index_loads_before=before,index_loads_after=h.stderr().count('imdb index loaded'))))
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def cli_search_install_waits(binary,systemd):
    # A local TLS endpoint that closes after receiving a real ClientHello:
    # installation must await its pending mutation's failure, never report the
    # initial acknowledgement as success. No certificate bypass or public URL.
    import socketserver
    class RejectTLS(socketserver.BaseRequestHandler):
        def handle(self):
            self.request.settimeout(2)
            hello=self.request.recv(4096)
            if hello:time.sleep(.5)
    with d.api_owned('cli_search_install_waits',binary,systemd) as (case,box,h,process):
        with socketserver.ThreadingTCPServer(('127.0.0.1',0),RejectTLS) as source:
            thread=threading.Thread(target=source.serve_forever);thread.start()
            try:
                url=f'https://127.0.0.1:{source.server_address[1]}/shape.py'
                outputs=[]
                for mode in ('daemon','local'):
                    if mode=='local':h.stop_serve(process)
                    began=time.monotonic()
                    _,result=cli(case,box,binary,'search','--install-url',url,'--json',code=5)
                    elapsed=time.monotonic()-began
                    case.check(elapsed>=.45 and elapsed<15,'installation waits for the real upstream failure in '+mode,f'elapsed={elapsed:.3f}s; stdout={result.stdout}')
                    outputs.append(result.stdout)
                equal(case,['--json'],*outputs)
            finally:source.shutdown();thread.join(3)


CASES=(cli_search_both_paths_identical,cli_search_dedupe_both_paths,cli_search_timeout_cleans_provider,
       cli_search_parked_fallback,cli_search_silent_hello_fallback,cli_imdb_both_paths_identical,cli_search_install_waits)
