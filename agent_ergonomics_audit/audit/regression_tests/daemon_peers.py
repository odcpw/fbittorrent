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
    def __init__(self,case,binary,systemd,*,payload_bytes=8*1024*1024+16384*2,upload_bps=65536):
        self.case,self.binary,self.systemd=case,binary,systemd
        self.payload_bytes,self.upload_bps=payload_bytes,upload_bps
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
            payload=self.nodes[0][0].downloads/'peer-payload.bin';payload.write_bytes(os.urandom(self.payload_bytes))
            for index,path in enumerate(('seed','leech','observer')):
                box,h,_,client,_=self.nodes[index]
                metainfo=fixtures.torrent_fixture(box.root,[payload],tracker=http.base+'/'+path)
                self.hash=metainfo.info_hash
                client.call('torrents.add',dict(source=dict(path=str(metainfo.torrent)),save_path=str(box.downloads),seed=index==0))
                if index==0:
                    h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=self.hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'local seed verifies real payload')
                    d.DaemonContract.throttle(client,self.hash,self.upload_bps)
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


def tui_peers_tab(binary, systemd):
    """Real peers over the TUI's one connection, visible-only topic and rate updates."""
    import native
    with d.Case('daemon', 'tui_peers_tab') as case, Swarm(case, binary, systemd, payload_bytes=32*1024*1024, upload_bps=262144) as swarm:
        if not swarm.h.wait_for(swarm.observe, 'actual seed and leecher arrive before opening the TUI', timeout=20):
            return
        topic='torrent:'+swarm.hash+'.peers'
        def subscriptions():
            return swarm.client.call('app.info')['result']['subscriptions']['by_topic']
        ui=native.AttachedTui(case,str(binary),swarm.box,swarm.h,session='fbt-fbt-p5-02-f3k')
        try:
            native.prepare_imdb_fixture(swarm.box.env,['peer-payload.bin'])
            target,done=ui.start()
            ui.assert_pane(target,"peer-payload.bin")
            case.check(subscriptions().get(topic,0)==0,'hidden Peers tab has no detail subscription',json.dumps(subscriptions()))
            ui.keys(target,'Enter',literal=False)
            ui.assert_pane(target,'1 seeds / 2 peers connected')
            ui.keys(target,']]')
            ui.assert_pane(target,'[Peers]')
            swarm.h.wait_for(lambda:((counts:=subscriptions()).get(topic)==1,json.dumps(counts)), 'visible Peers tab owns exactly one detail subscription')
            for width,height in [(120,30),(160,45),(80,24)]:
                resized=ui.tmux('resize-window','-t',target,'-x',str(width),'-y',str(height))
                case.check(resized.returncode==0,'resize Peers tab to '+str((width,height)),resized.stderr)
                ui.assert_pane(target,'127.0.0.1:')
                ui.assert_pane(target,'yes')
                ui.assert_pane(target,'no')
                ui.assert_pane(target,'↓/s')
                pane=case.path.with_suffix('.peers-'+str(width)+'.txt');pane.write_text(ui.capture(target))
                case.check('[Peers]' in pane.read_text(),'real local peers render at '+str(width)+' columns',str(pane)+'\n'+pane.read_text())
            def moving():
                peers=swarm.client.call('torrents.peers',dict(hash=swarm.hash))['result']['peers']
                return any(p.get('dl_bps',0)>0 for p in peers),json.dumps(peers)
            swarm.h.wait_for(moving,'local transfer produces actual positive peer rates',timeout=12)
            observations=[]
            for index in range(2):
                if index:time.sleep(2)
                peers=swarm.client.call('torrents.peers',dict(hash=swarm.hash))['result']['peers']
                pane=ui.capture(target)
                observations.append((peers,pane))
                case.check(any(p.get('dl_bps',0)>0 for p in peers) and '↓/s' in pane,'peer rates read '+str(index+1)+' while the real tab is visible',json.dumps(dict(peers=peers,pane=pane)))
            previous={p['addr']:p['downloaded'] for p in observations[0][0]}
            case.check(any(p['downloaded']>previous.get(p['addr'],p['downloaded']) for p in observations[1][0]) and observations[0][1]!=observations[1][1],
                       'two reads two seconds apart show moving real counters and a changed terminal',json.dumps(observations))
            ui.keys(target,']')
            ui.assert_pane(target,'[Log]')
            swarm.h.wait_for(lambda:((counts:=subscriptions()).get(topic,0)==0,json.dumps(counts)),'leaving Peers releases the detail topic')
            ui.keys(target,'[')
            ui.assert_pane(target,'[Peers]')
            swarm.h.wait_for(lambda:((counts:=subscriptions()).get(topic)==1,json.dumps(counts)),'returning to Peers starts one fresh subscription')
            ui.keys(target,'Escape',literal=False)
            swarm.h.wait_for(lambda:((counts:=subscriptions()).get(topic,0)==0,json.dumps(counts)),'closing the details modal releases the final peer sampler')
            ui.keys(target,'q')
            swarm.h.wait_for(lambda:(done.exists(),ui.capture(target)),'TUI detaches cleanly')
        finally:
            ui.close()
