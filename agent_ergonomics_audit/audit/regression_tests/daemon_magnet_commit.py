"""Stage real-peer magnet metadata, commit it, and verify downloaded disk bytes."""
import hashlib
import json
from pathlib import Path
import tomllib
import urllib.parse
import daemon as d
import fixtures


def staged_magnet_downloads(binary, systemd):
    with d.api_owned('staged_magnet_downloads',binary,systemd) as (case,box,h,p),d.ApiConn(h) as client, \
            d.Sandbox() as seedbox,d.Harness(case,binary,seedbox,systemd) as seed:
        sp=seed.start_serve(idle_exit_secs=0)
        assert seed.receive_ready() and seed.wait_engine()
        client.hello()
        item=d.recheck_local_fixture(seedbox,name='Magnet committed.bin',size=1024*1024)
        item.info.pop('private')
        item.info_hash=hashlib.sha1(fixtures.bencode(item.info)).hexdigest()
        item.torrent.write_bytes(fixtures.bencode(dict(info=item.info)))
        with d.ApiConn(seed) as peer:
            peer.hello();peer.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(item.files[0].parent),seed=True))
            assert seed.wait_for(lambda:((r:=peer.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(r)), 'real seed verifies source bytes')
            port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
            with fixtures.tracker_server(fail_first=0,interval_s=1,peers=[('127.0.0.1',port)],directory=seedbox.root) as tracker:
                target=box.downloads/'committed'
                magnet='magnet:?xt=urn:btih:'+item.info_hash+'&tr='+urllib.parse.quote(tracker.url,safe='')
                client.call('torrents.add',dict(source=dict(magnet=magnet),stage=True,save_path=str(target)))
                assert h.wait_for(lambda:((r:=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']).get('file_count')==1,json.dumps(r)), 'staged magnet resolves metadata from a real peer')
                r=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                case.check(r['staged'] and r['state']=='paused' and not r['checked'] and not target.exists(),
                           'metadata alone neither admits transfer nor creates payload storage',json.dumps(r))
                client.call('torrents.commit',dict(hash=item.info_hash,files=[0],seed=False,save_path=str(target)))
                assert h.wait_for(lambda:((r:=client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='done',json.dumps(r)), 'committed real-peer magnet downloads to completion',timeout=25)
                downloaded=target/item.files[0].name
                expected=hashlib.sha256(item.files[0].read_bytes()).hexdigest()
                actual=hashlib.sha256(downloaded.read_bytes()).hexdigest()
                case.check(actual==expected and downloaded.stat().st_size==item.files[0].stat().st_size,
                           'commit receives the exact source payload, independently hashed from disk',json.dumps(dict(source_sha256=expected,download_sha256=actual,bytes=downloaded.stat().st_size)))
        seed.stop_serve(sp);seed.lock_free()


CASES=(staged_magnet_downloads,)
