"""P5-01 real local three-member swarm, peer bitfields, rates and detail lifetime."""
from contextlib import ExitStack
import base64
import json
import os
import socket
import time
import tomllib
import daemon as d
import fixtures


class Swarm:
    def __init__(self,case,binary,systemd):
        self.case,self.binary,self.systemd=case,binary,systemd
        self.stack=ExitStack();self.nodes=[]

    def __enter__(self):
        try:
            for _ in range(3):
                box=self.stack.enter_context(d.Sandbox())
                h=self.stack.enter_context(d.Harness(self.case,self.binary,box,self.systemd))
                process=h.start_serve(idle_exit_secs=0)
                if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('isolated swarm daemon not ready')
                client=self.stack.enter_context(d.ApiConn(h));client.hello()
                config=tomllib.loads((d.Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())
                self.nodes.append((box,h,process,client,config['peer_port']))
            def compact(indices):
                peers=b''.join(socket.inet_aton('127.0.0.1')+int(self.nodes[i][4]).to_bytes(2,'big') for i in indices)
                return dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=peers))).decode())
            # Direction is deliberate: seed dials nobody; leecher dials seed;
            # observer dials both, so duplicate incoming connections cannot fake two peers.
            http=self.stack.enter_context(fixtures.HttpFixture(self.nodes[0][0].root,{'/seed':compact([]),'/leech':compact([0]),'/observer':compact([0,1])}))
            payload=self.nodes[0][0].downloads/'peer-payload.bin';payload.write_bytes(os.urandom(8*1024*1024+16384*2))
            for index,path in enumerate(('seed','leech','observer')):
                box,h,_,client,_=self.nodes[index]
                metainfo=fixtures.torrent_fixture(box.root,[payload],tracker=http.base+'/'+path)
                self.hash=metainfo.info_hash
                client.call('torrents.add',dict(source=dict(path=str(metainfo.torrent)),save_path=str(box.downloads),seed=index==0))
                if index==0:
                    h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=self.hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'local seed verifies real payload')
                    d.DaemonContract.throttle(client,self.hash,65536)
                if index==1:
                    h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=self.hash))['result']['torrent']).get('completed_bytes',0)>0 and row['state']=='downloading',json.dumps(row)),'incomplete local leecher receives actual payload')
            self.box,self.h,self.p,self.client,_=self.nodes[2]
            return self
        except BaseException:
            self.stack.close();raise

    def __exit__(self,*_):self.stack.close()

    def observe(self):
        self.peers=self.client.call('torrents.peers',dict(hash=self.hash)).get('result',{}).get('peers',[])
        self.row=self.client.call('torrents.get',dict(hash=self.hash)).get('result',{}).get('torrent',{})
        ready=len(self.peers)==2 and sorted(p.get('seed') for p in self.peers)==[False,True] and self.row.get('seeds')==1
        return ready,json.dumps(dict(torrent=self.row,peers=self.peers))


def peers_local_swarm(binary,systemd):
    with d.Case('daemon','peers_local_swarm') as case,Swarm(case,binary,systemd) as swarm:
        c,h=swarm.client,swarm.h
        if not h.wait_for(swarm.observe,'observer has exactly two live peers: one seed, one incomplete leecher',timeout=20):return
        case.check(all(p['state']=='live' and p['conn'] in ('tcp','tcp_mse') and p['incoming'] is False for p in swarm.peers),
                   'actual outgoing peer direction, transport and state are reported',json.dumps(swarm.peers))
        value=h.cli('show',swarm.hash)
        case.check(value.get('seeds')==value.get('num_seeds')==1 and len(value.get('peers',[]))==2,
                   'CLI show contains true connected seed count and the real peer list',json.dumps(value))
        topic='torrent:'+swarm.hash+'.peers'
        sub=c.call('subscribe',dict(topics=[topic]))['result']['sub']
        c.socket.settimeout(8)
        frames=list(c.pushed);c.pushed.clear();deadline=time.monotonic()+8
        while not any(f.get('ev')=='snapshot.end' and f.get('topic')==topic for f in frames) and time.monotonic()<deadline:
            frame=c.receive()
            if frame is None:break
            frames.append(frame)
        peers=[p for f in frames if f.get('topic')==topic for p in f.get('items',[])]
        case.check(len(peers)==2 and sorted(p['seed'] for p in peers)==[False,True],
                   'subscribed detail snapshot agrees with actual seed/leecher flags',json.dumps(frames))
        c.call('unsubscribe',dict(sub=sub))
        info=c.call('app.info')['result']
        case.check(info['subscriptions']['by_topic'].get(topic,0)==0,'last unsubscribe releases the live detail topic',json.dumps(info['subscriptions']))
        c.call('torrents.stop',dict(hashes=[swarm.hash]))
        def stopped():
            row=c.call('torrents.get',dict(hash=swarm.hash)).get('result',{}).get('torrent',{})
            peers=c.call('torrents.peers',dict(hash=swarm.hash)).get('result',{}).get('peers')
            return row.get('state')=='paused' and 'seeds' not in row and peers==[],json.dumps(dict(torrent=row,peers=peers))
        h.wait_for(stopped,'paused torrent omits unknown seeds and has no live peers')


def peer_rates_move(binary,systemd):
    with d.Case('daemon','peer_rates_move') as case,Swarm(case,binary,systemd) as swarm:
        if not swarm.h.wait_for(swarm.observe,'actual seed and leecher bitfields arrive before rate observations',timeout=20):return
        c=swarm.client;topic='torrent:'+swarm.hash+'.peers'
        c.call('subscribe',dict(topics=[topic]))
        previous={p['addr']:p['downloaded'] for p in swarm.peers}
        def rates():
            peers=c.call('torrents.peers',dict(hash=swarm.hash)).get('result',{}).get('peers',[])
            moving=[p for p in peers if p.get('downloaded',0)>previous.get(p['addr'],p.get('downloaded',0)) and p.get('dl_bps',0)>0]
            return bool(moving),json.dumps(peers)
        swarm.h.wait_for(rates,'successive real byte counters produce positive per-peer rates after at least one second',timeout=12)
        frames=list(c.pushed);c.pushed.clear();deadline=time.monotonic()+8
        while not any(f.get('ev')=='delta' and any(p.get('dl_bps',0)>0 for p in f.get('changed',{}).values()) for f in frames) and time.monotonic()<deadline:
            frame=c.receive()
            if frame is None:break
            frames.append(frame)
            if len(frames)>128:raise RuntimeError('bounded peer frame capture exceeded')
        case.check(any(f.get('ev')=='delta' and any(p.get('dl_bps',0)>0 for p in f.get('changed',{}).values()) for f in frames),
                   'one-second detail deltas carry actual positive peer rates and counters keyed by addr',json.dumps(frames))
