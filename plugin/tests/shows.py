#!/usr/bin/env python3
"""Follow/read/undo/review/download using an owned daemon and dashboard."""
import argparse
import subprocess
import traceback
import json
from pathlib import Path
from dashboard import Session
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
                    q=Session(case,box)
                    keyboard=subprocess.Popen(["wtype","-s","600000"],env=q.env)
                    def keys(args,predicate):
                        q.compositor.ipc('[title="fbittorrent"] focus')
                        return q.keys(args,predicate)
                    def view():return q.state().get('view') or {}
                    def state():return view().get('shows') or {}
                    def sheet():return view().get('add') or {}
                    def releases():return client.call('shows.releases',dict(imdb=IMDB,unread_only=False))['result']['releases']
                    try:
                        q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"shows"}')
                        q.wait(lambda:'shows' in view())
                        keys(['n'],lambda:state().get('panel')=='follow')
                        keys([IMDB,'-k','Return'],lambda:len(state().get('shows',[]))==1 and not state().get('panel'))
                        assert client.call('torrents.list',{})['result']['torrents']==[]
                        keys(['r'],lambda:len(state().get('rows',[]))==2)
                        assert all(not row['read'] and row['transfer']=='none' for row in releases())
                        assert client.call('torrents.list',{})['result']['torrents']==[], 'checking never adds a torrent'
                        keys(['l'],lambda:state().get('focus')=='releases')
                        keys(['m'],lambda:len(state().get('rows',[]))==1)
                        keys(['u'],lambda:len(state().get('rows',[]))==2)
                        keys(['M'],lambda:not state().get('rows'))
                        # Read history comes from the real daemon; the pushed
                        # inbox is unread-only. Exercise both independent controls
                        # through the actual Dropdown buttons and keyboard focus.
                        assert q.ipc('showsDropdown','read')=='true';q.wait(lambda:view().get('controls',{}).get('readOpen'))
                        keys(['j','-k','Return'],lambda:state().get('readFilter')=='all' and len(state().get('rows',[]))==2)
                        assert q.ipc('showsDropdown','read')=='true';q.wait(lambda:view().get('controls',{}).get('readOpen'))
                        keys(['j','-k','Return'],lambda:state().get('readFilter')=='read' and len(state().get('rows',[]))==2)
                        assert q.ipc('showsDropdown','quality')=='true';q.wait(lambda:view().get('controls',{}).get('qualityOpen'))
                        keys(['jjj','-k','Return'],lambda:state().get('qualityFilter')=='720p' and not state().get('rows'))
                        assert q.ipc('showsDropdown','quality')=='true';q.wait(lambda:view().get('controls',{}).get('qualityOpen'))
                        keys(['kkk','-k','Return'],lambda:state().get('qualityFilter')=='' and len(state().get('rows',[]))==2)
                        assert q.ipc('showsDropdown','read')=='true';q.wait(lambda:view().get('controls',{}).get('readOpen'))
                        keys(['kk','-k','Return'],lambda:state().get('readFilter')=='unread' and not state().get('rows'))
                        keys(['hj'],lambda:state().get('selected')==IMDB)
                        keys(['l'],lambda:state().get('focus')=='releases' and len(state().get('rows',[]))==2)
                        keys(['-k','space'],lambda:any(not row['read'] for row in state().get('rows',[])))
                        selected=state()['rows'][state()['cursor']];key=selected['key']
                        keys(['-k','Return'],lambda:sheet().get('visible'))
                        q.wait(lambda:sheet().get('phase')=='ready')
                        stage=client.call('torrents.get',dict(hash=sheet()['hash']))['result']['torrent']
                        assert stage['staged'] and not stage['checked'] and not list(box.downloads.iterdir())
                        assert next(row for row in releases() if row['key']==key)['read'] is False
                        keys(['-k','Escape'],lambda:not sheet().get('visible'))
                        assert client.call('torrents.list',{})['result']['torrents']==[] and not list(box.downloads.iterdir())
                        assert next(row for row in releases() if row['key']==key)['read'] is False, 'cancel must leave unread'
                        keys(['-k','Return'],lambda:sheet().get('visible'));q.wait(lambda:sheet().get('phase')=='ready')
                        info_hash=sheet()['hash'];keys(['-k','Return'],lambda:not sheet().get('visible'))
                        def started():
                            row=client.call('torrents.get',dict(hash=info_hash))['result']['torrent']
                            return row if not row['staged'] else None
                        added=q.wait(started)
                        assert added['origin']=='shows' and added['intent']=='run'
                        keys(['3'],lambda:'shows' in view())
                        q.wait(lambda:any(row['key']==key and row['read'] and row['transfer']!='none' for row in state().get('rows',[])))
                        q.compositor.ipc('[title="fbittorrent"] floating enable, resize set width 900 px height 600 px')
                        q.wait(lambda:view()['window']['width']<1050)
                        keys(['h'],lambda:state()['focus']=='shows')
                        keys(['l'],lambda:state()['focus']=='releases')
                        image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists())
                        keys(['x'],lambda:state().get('panel')=='remove' and view().get('unfollowCancel'))
                        keys(['-k','Return'],lambda:not state().get('panel'))
                        assert len(client.call('shows.list')['result']['shows'])==1,'Cancel must be the initial unfollow choice'
                        before={p.name:p.read_bytes() for p in box.downloads.iterdir() if p.is_file()}
                        keys(['x'],lambda:state().get('panel')=='remove')
                        keys(['-k','Right','-k','Return'],lambda:not state().get('shows'))
                        assert client.call('torrents.get',dict(hash=info_hash))['ok']
                        assert all((box.downloads/name).read_bytes()==data for name,data in before.items()),'unfollow keeps downloads'
                        q.shell('hide');assert client.call('app.info')['ok'],'window close keeps the owner running'
                        print('PASS S01: follow/check, mark/undo/all, review/cancel, commit/read/transfer, narrow navigation, safe unfollow',flush=True)
                    except Exception:
                        print(json.dumps(q.state()),flush=True);traceback.print_exc();raise
                    finally:
                        keyboard.terminate();keyboard.wait(timeout=3);q.close()
                harness.stop_serve(process);harness.lock_free()

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    args=parser.parse_args();run(args.binary.resolve())
    raise SystemExit(suite_status("qs_shows"))
