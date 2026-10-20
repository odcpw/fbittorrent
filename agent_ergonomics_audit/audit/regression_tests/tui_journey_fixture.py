"""Real capped seed and local catalogues shared by the five M3 TUI journeys."""
from contextlib import ExitStack
import base64
import hashlib
import json
import os
from pathlib import Path
import socket
import tomllib
import urllib.parse
import daemon as d
import fixtures
import native

IMDB='tt1234001'


class Fixture:
    def __init__(self,case,binary,source):
        self.case,self.binary,self.source=case,binary,source
        self.stack=ExitStack()

    def __enter__(self):
        try:return self.start()
        except BaseException:self.stack.close();raise

    def start(self):
        self.box=self.stack.enter_context(fixtures.Sandbox())
        # Nova cannot honour a VPN binding. This Search journey already runs
        # inside a verified loopback-only namespace; opt into its local providers.
        if self.source=='search':
            d.select_replay_interface(Path(self.box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml',None)
        seedbox=self.stack.enter_context(fixtures.Sandbox())
        self.seed=self.stack.enter_context(d.Harness(self.case,self.binary,seedbox,False))
        self.seed_process=self.seed.start_serve(idle_exit_secs=0)
        if not(self.seed.receive_ready() and self.seed.wait_engine()):raise RuntimeError('private seed did not start')
        seedport=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        peer=socket.inet_aton('127.0.0.1')+int(seedport).to_bytes(2,'big')
        response=lambda peers:dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=peers))).decode())
        self.http=self.stack.enter_context(fixtures.HttpFixture(self.box.root,{'/seed':response(b''),'/announce':response(peer),'/api/get-torrents':dict(file='episodes.json')}))
        self.name='Journey.'+self.source+'.S01E01.1080p'
        directory=seedbox.downloads/self.name;directory.mkdir()
        video=directory/'Episode.mkv';video.write_bytes(os.urandom(16*fixtures.MIB))
        subtitles=directory/'English.srt';subtitles.write_text('1\n00:00:01 --> 00:00:02\nPrivate journey fixture.\n')
        sample=directory/'sample.mkv';sample.write_bytes(os.urandom(16384))
        self.torrent=fixtures.torrent_fixture(self.box.root,[video,subtitles,sample],name=self.name,tracker=self.http.base+'/announce')
        seedtorrent=fixtures.torrent_fixture(seedbox.root,[video,subtitles,sample],name=self.name,tracker=self.http.base+'/seed')
        # Private seeds correctly refuse BEP 9. Public metainfo is safe here:
        # the verified kernel namespace has loopback only and no external route.
        for torrent,tracker in [(self.torrent,self.http.base+'/announce'),(seedtorrent,self.http.base+'/seed')]:
            torrent.info['private']=0;torrent.info_hash=hashlib.sha1(fixtures.bencode(torrent.info)).hexdigest()
            torrent.torrent.write_bytes(fixtures.bencode(dict(info=torrent.info,announce=tracker)))
            torrent.magnet='magnet:?'+urllib.parse.urlencode(dict(xt='urn:btih:'+torrent.info_hash,dn=self.name,tr=tracker))
        self.other_name='Second.'+self.source+'.bin'
        other=seedbox.downloads/self.other_name;other.write_bytes(os.urandom(8*fixtures.MIB))
        self.other=fixtures.torrent_fixture(self.box.root,[other],name='second',tracker=self.http.base+'/announce')
        seedother=fixtures.torrent_fixture(seedbox.root,[other],name='second',tracker=self.http.base+'/seed')
        with d.ApiConn(self.seed) as client:
            client.hello()
            for torrent in (seedtorrent,seedother):
                client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(directory if torrent is seedtorrent else seedbox.downloads),seed=True))
                if not self.seed.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'seed verifies real fixture pieces'):raise RuntimeError('fixture seed did not verify')
                client.call('torrents.set_upload_limit',dict(hashes=[torrent.info_hash],bps=131072))
        self.box.env.update(FBT_EZTV_BASE_URL=self.http.base,FBT_IMDB_BASE_URL=self.http.base)
        self.box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
        self.feed=self.box.root/'feed.xml'
        self.feed.write_text('<rss><channel><title>Journey feed</title><item><guid>journey-release</guid><title>'+self.name+'</title><enclosure url="'+self.http.base+'/'+self.torrent.torrent.name+'" type="application/x-bittorrent"/></item></channel></rss>')
        (self.box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=1,limit=100,page=1,torrents=[dict(id=1,imdb_id=IMDB[2:],title=self.name,season='1',episode='1',seeds=1,size_bytes=str(16*fixtures.MIB),date_released_unix=1792274401,magnet_url=self.torrent.magnet)])))
        engines=Path(self.box.env['XDG_DATA_HOME'])/'fbittorrent/nova3/engines';engines.mkdir(parents=True)
        for provider in ('journeya','journeyb'):
            (engines/(provider+'.py')).write_text('class '+provider+':\n    name="Local journey provider"\n    url="http://127.0.0.1"\n    def search(self,query):\n        from novaprinter import prettyPrinter\n        prettyPrinter('+repr(dict(link=self.torrent.magnet,name=self.name,size=16*fixtures.MIB,seeds=1,leech=-1,engine_url=self.http.base))+')\n')
        native.prepare_imdb_fixture(self.box.env,[self.name,self.other_name])
        theme=Path(self.box.env['XDG_STATE_HOME'])/'omarchy/current/theme/colors.toml';theme.parent.mkdir(parents=True,exist_ok=True)
        theme.write_bytes((fixtures.HERE.parents[2]/'contracts/ui/themes/owner.toml').read_bytes())
        self.poster()
        self.h=self.stack.enter_context(d.Harness(self.case,self.binary,self.box,False))
        self.process=self.h.start_serve(idle_exit_secs=0)
        if not(self.h.receive_ready() and self.h.wait_engine()):raise RuntimeError('private journey owner did not start')
        self.client=self.stack.enter_context(d.ApiConn(self.h));self.client.hello()
        return self

    def poster(self):
        import re,shutil
        cache=Path(self.box.env['XDG_CACHE_HOME'])/'fbittorrent'
        title=dict(id=IMDB,name='Journey Fixture',year=2026,kind='TV series',stars='Local cast',poster_url=self.http.base+'/poster.jpg',rating=7.9,votes=100,blurb='Private local journey catalogue.',wiki_url='')
        for kind,key,value in [('by-name-v2',self.name,dict(id=IMDB)),('by-id',IMDB,title)]:
            directory=cache/'imdb'/kind;directory.mkdir(parents=True,exist_ok=True)
            (directory/(re.sub('[^a-z0-9]','_',key.lower())[:120]+'.json')).write_text(json.dumps(value))
        posters=cache/'posters';posters.mkdir(exist_ok=True)
        shutil.copyfile(fixtures.HERE/'fixtures/imdb/poster.jpg',posters/(IMDB+'.jpg'))
        (cache/'title.ratings.tsv').write_text('tconst\taverageRating\tnumVotes\n'+IMDB+'\t7.9\t100\n')

    def row(self):return self.client.call('torrents.get',dict(hash=self.torrent.info_hash))['result']['torrent']
    def rows(self):return self.client.call('torrents.list',{})['result']['torrents']
    def __exit__(self,*_):self.stack.close()
