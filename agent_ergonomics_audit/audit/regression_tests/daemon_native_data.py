"""The only transport reads today's native stores without rewriting them."""
import hashlib
import json
from pathlib import Path
import time

import daemon as d


def native_data_unchanged(binary, systemd):
    with d.Case('daemon','native_data_unchanged') as case, d.Sandbox() as sandbox, d.Harness(case,binary,sandbox,systemd) as h:
        digest,size=d.saved_hash_fixture(sandbox,4*d.fixtures.MIB)
        config=Path(sandbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('network_interface = "lo"','network_interface = "fbmissing0"'))
        data=Path(sandbox.env['XDG_DATA_HOME'])/'fbittorrent'
        episode=dict(key='S01E01',season=1,episode=1,title='Saved episode',magnet='magnet:?xt=urn:btih:'+digest,
                     seeds=4,size=size,released=1,quality='1080p',candidates=1)
        (data/'shows.json').write_text(json.dumps(dict(shows=[dict(imdb='tt1234567',title='Saved show',year=2026,
            added=1,last_checked=int(time.time()),catalogue_complete=True,seen=['S01E01'],episodes=[episode])]))+'\n')
        article=dict(id='saved-item',title='Saved item',link='magnet:?xt=urn:btih:'+digest,torrentURL='',date='',description='',isRead=True,ts=1)
        (data/'rss.json').write_text(json.dumps(dict(version=1,feeds=[dict(feed=dict(name='Saved feed',uid='saved-feed',
            url='http://127.0.0.1:1/rss',title='Saved feed',last_build_date='',is_loading=False,has_error=False,articles=[article]),read_ids=['saved-item'],last_checked=int(time.time()))]))+'\n')
        paths=[config,data/'shows.json',data/'rss.json',data/'engine/torrents.json',data/'engine'/f'{digest}.torrent',sandbox.downloads/'daemon-hash.bin']
        def snapshot(): return {str(p.relative_to(sandbox.root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
        before=snapshot();process=h.start_serve()
        if h.receive_ready() and h.wait_engine():
            with d.ApiConn(h) as client:
                hello=client.hello()['result']
                rows=client.call('torrents.list',{})['result']['torrents']
                shows=client.call('shows.list')['result']['shows']
                feeds=client.call('rss.feeds')['result']['feeds']
                items=client.call('rss.items',dict(feed=feeds[0]['id']))['result']['items']
                case.check(hello['state']=='parked' and rows[0]['hash']==digest and rows[0]['name']=='daemon-hash.bin' and 'progress' not in rows[0],
                           'today\'s ledger opens with saved identity and truthful unknown progress',json.dumps(rows))
                case.check(shows[0]['title']=='Saved show' and shows[0]['unread']==0 and feeds[0]['name']=='Saved feed' and items[0]['read'],
                           'today\'s Shows and RSS retain titles, read history and cached entries',json.dumps(dict(shows=shows,feeds=feeds,items=items)))
                case.check(h.engine_path.name=='api.sock' and h.engine_path.exists() and not h.engine_path.with_name('engine.sock').exists(),
                           'the daemon creates only the API v1 listener')
                case.check(snapshot()==before,'opening and reading native stores preserves every input byte',json.dumps(dict(before=before,after=snapshot())))
            h.stop_serve(process)
        h.lock_free()
