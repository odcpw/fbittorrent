#!/usr/bin/env python3
"""Follow/read/undo/review/download using an owned daemon and dashboard."""
import argparse
import json
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/"agent_ergonomics_audit/audit/regression_tests"))
import daemon
import fixtures
from fbtlog import Case,suite_status

IMDB='tt1234001'
def run(binary):
    with Case('qs_shows','journey') as case,fixtures.Sandbox() as box:
        source=box.root/'source';source.mkdir()
        routes={'/api/get-torrents':dict(file='episodes.json')}
        for index in [1,2]:
            path=source/f'episode-{index}.mkv';path.write_bytes(bytes([index])*262144)
            torrent=fixtures.torrent_fixture(source,[path],name=f'Episode {index}')
            routes[f'/episode-{index}.torrent']=dict(file=str(torrent.torrent.relative_to(box.root)))
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        (cache/(IMDB+'.json')).write_text(json.dumps(dict(id=IMDB,name='Owned Shows Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')))
        with fixtures.HttpFixture(box.root,routes) as http:
            box.env.update(FBT_EZTV_BASE_URL=http.base,FBT_IMDB_BASE_URL=http.base)
            box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
            episodes=[dict(id=i,imdb_id=IMDB[2:],title=f'Owned.Show.S01E0{i}.1080p',season='1',episode=str(i),seeds=2,size_bytes='262144',date_released_unix=1792274400+i,magnet_url=http.base+f'/episode-{i}.torrent') for i in [1,2]]
            (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=2,limit=100,page=1,torrents=episodes)))
            with daemon.Harness(case,binary,box,False) as harness:
                process=harness.start_serve(idle_exit_secs=0)
                assert harness.receive_ready() and harness.wait_engine()
                with daemon.ApiConn(harness) as client:
                    client.hello()
                    client.call('subscribe',dict(topics=['events']))
                    def confirmed(op,args):
                        value=client.call(op,args)['result']
                        if value.get('state')!='pending':return value
                        def completed():
                            client.call('app.info');events=[]
                            for frame in client.pushed:events.extend([frame]+frame.get('added',[])+frame.get('items',[]))
                            return next((event for event in events if event.get('ev')=='mutation.result' and event.get('ref')==value['ref']),None)
                        event=None
                        def ready():
                            nonlocal event
                            event=completed();return bool(event),str(event)
                        assert harness.wait_for(ready,'catalogue mutation completes')
                        assert event['outcome']=='done',event
                        return event.get('result',{})
                    confirmed('shows.follow',dict(imdb=IMDB))
                    confirmed('shows.check',dict(imdb=IMDB))
                    def release():return next(row for row in client.call('shows.releases',dict(imdb=IMDB))['result']['releases'] if row['key']=='S01E02')
                    staged=confirmed('shows.grab',dict(imdb=IMDB,key='S01E02',stage=True))
                    assert not release()['read'] and release()['transfer']=='none' and not list(box.downloads.iterdir())
                    confirmed('torrents.discard',dict(hash=staged['hash']))
                    assert not release()['read'] and client.call('torrents.list',{})['result']['torrents']==[]
                    staged=confirmed('shows.grab',dict(imdb=IMDB,key='S01E02',stage=True))
                    confirmed('torrents.commit',dict(hash=staged['hash'],files=[0],seed=False,save_path=str(box.downloads)))
                    assert release()['read'] and release()['transfer']!='none'
                    assert client.call('torrents.get',dict(hash=staged['hash']))['result']['torrent']['origin']=='shows'
                    print('PASS S01 source hook: stage leaves unread/no payload; discard stays unread; commit marks read and transfer',flush=True)
                    harness.stop_serve(process);harness.lock_free();return

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    args=parser.parse_args();run(args.binary.resolve())
    raise SystemExit(suite_status("qs_shows"))
