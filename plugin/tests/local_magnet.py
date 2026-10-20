"""Real metainfo/peer exchange for UI timing, confined to the private network."""
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import tomllib
import urllib.parse

import dashboard  # register the repository regression helpers on sys.path
import daemon as d
import fixtures
from visual_fixture import stop_owned


class LocalMagnet:
    def __init__(self,case,binary):self.case=case;self.binary=binary;self.stack=ExitStack()
    def process(self):
        box=self.stack.enter_context(d.Sandbox())
        h=self.stack.enter_context(d.Harness(self.case,self.binary,box,False))
        p=h.start_serve(idle_exit_secs=0);self.stack.callback(stop_owned,h,p)
        assert h.receive_ready() and h.wait_engine()
        client=self.stack.enter_context(d.ApiConn(h));client.hello()
        return box,h,p,client
    def __enter__(self):
        try:
            self.box,self.h,self.p,self.client=self.process()
            self.seedbox,self.seed,self.sp,self.peer=self.process()
            port=tomllib.loads((Path(self.seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
            self.tracker=self.stack.enter_context(fixtures.tracker_server(fail_first=0,interval_s=1,
                peers=[('127.0.0.1',port)],directory=self.seedbox.root))
            self.payload=self.seedbox.root/'source.bin';self.payload.write_bytes(os.urandom(8*fixtures.MIB))
            self.peer.call('transfer.set_limits',dict(upload_bps=32768))
            return self
        except BaseException:self.stack.close();raise
    def __exit__(self,*_):self.stack.close()
    def stop(self):
        self.h.stop_serve(self.p);self.h.notify.close();self.client.close()
    def start(self):
        self.p=self.h.start_serve(idle_exit_secs=0);self.stack.callback(stop_owned,self.h,self.p)
        assert self.h.receive_ready() and self.h.wait_engine()
        self.client=self.stack.enter_context(d.ApiConn(self.h));self.client.hello()
    def paused(self,count):
        directory=self.box.root/'paused';directory.mkdir()
        length=8*fixtures.MIB;piece_length=16384
        pieces=hashlib.sha1(bytes(piece_length)).digest()*(length//piece_length)
        for i in range(count):
            info={'name':f'Table fixture {i:03d}.bin','piece length':piece_length,'private':1,'length':length,'pieces':pieces}
            path=directory/f'{i}.torrent';path.write_bytes(fixtures.bencode({'info':info}))
            self.client.call('torrents.add',dict(source=dict(path=str(path)),save_path=str(self.box.downloads),stopped=True))
        rows=self.client.call('torrents.list',{})['result']['torrents']
        assert len(rows)==count and all(row['state']=='paused' for row in rows),rows
    def magnet(self,index):
        payload=self.seedbox.downloads/f'Latency {index}.bin';os.link(self.payload,payload)
        item=fixtures.torrent_fixture(self.seedbox.root,[payload],name='probe-'+str(index),tracker=self.tracker.url)
        item.info.pop('private')
        item.info_hash=hashlib.sha1(fixtures.bencode(item.info)).hexdigest()
        item.torrent.write_bytes(fixtures.bencode(dict(info=item.info,announce=self.tracker.url)))
        self.peer.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(self.seedbox.downloads),seed=True))
        assert self.seed.wait_for(lambda:((row:=self.peer.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)), 'local probe seed verifies')
        magnet='magnet:?'+urllib.parse.urlencode(dict(xt='urn:btih:'+item.info_hash,dn=payload.name,tr=self.tracker.url))
        return item,magnet,payload.name
    def remove(self,item):
        self.client.call('torrents.remove',dict(hashes=[item.info_hash],delete_files=False))
        self.peer.call('torrents.remove',dict(hashes=[item.info_hash],delete_files=False))
