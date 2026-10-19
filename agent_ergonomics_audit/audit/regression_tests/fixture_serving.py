"""Real managed fixture owner. A guardian cleans up even after launcher SIGKILL."""
from contextlib import ExitStack
import base64
import ctypes
import json
import os
from pathlib import Path
import select
import signal
import socket
import stat
import subprocess
import sys
import threading
import tomllib

import daemon as d
import fixtures


class Pair:
    def __init__(self,case,binary,systemd=False,throttle=32768,state='downloading',extra_peers=()):
        self.case,self.binary,self.systemd,self.throttle,self.state=case,Path(binary).resolve(),systemd,throttle,state
        self.stack=ExitStack();self.owned=[];self.hash=None;self.row=None;self.extra_peers=extra_peers

    def process(self):
        box=self.stack.enter_context(d.Sandbox())
        h=self.stack.enter_context(d.Harness(self.case,self.binary,box,self.systemd))
        p=h.start_serve(idle_exit_secs=0)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('fixture daemon did not become ready')
        client=self.stack.enter_context(d.ApiConn(h));client.hello()
        self.owned.append((box,h,p,client));return box,h,p,client

    def __enter__(self):
        try:
            self.box,self.h,self.p,self.client=self.process()
            recipe=json.loads((fixtures.HERE/'fixtures/states'/f'{self.state}.json').read_text())
            if self.state=='idle':return self
            if self.state=='downloading':
                seedbox,seed,sp,peer=self.process()
                cfg=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())
                peers=socket.inet_aton('127.0.0.1')+int(cfg['peer_port']).to_bytes(2,'big')
                body=fixtures.bencode(dict(interval=1,peers=peers))
                http=self.stack.enter_context(fixtures.HttpFixture(seedbox.root,{'/announce':dict(base64=base64.b64encode(body).decode())}))
                payload=seedbox.downloads/'fixture-download.bin';payload.write_bytes(os.urandom(recipe['payload_bytes']))
                url=http.base+'/announce'
                if self.extra_peers:
                    target_cfg=tomllib.loads((Path(self.box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())
                    tracker=self.stack.enter_context(fixtures.tracker_server(fail_first=0,interval_s=1,
                        peers=[('127.0.0.1',cfg['peer_port']),*self.extra_peers],only_port=target_cfg['peer_port'],directory=seedbox.root))
                    url=tracker.url
                item=fixtures.torrent_fixture(seedbox.root,[payload],tracker=url)
                peer.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(seedbox.downloads),seed=True))
                seed.wait_for(lambda:((row:=peer.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'fixture seed verifies exact local payload')
                d.DaemonContract.throttle(peer,item.info_hash,self.throttle or None)
                self.client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(self.box.downloads)))
                self.seed,self.seed_client,self.payload=seed,peer,payload
            else:
                root=self.box.downloads if self.state=='seeding' else self.box.root/'source'
                root.mkdir(exist_ok=True)
                payload=root/'fixture-state.bin';payload.write_bytes(os.urandom(recipe['payload_bytes']))
                item=fixtures.torrent_fixture(root,[payload])
                self.client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(self.box.downloads),seed=self.state=='seeding',stopped=self.state=='paused'))
                if self.state=='seeding':
                    if not self.h.wait_for(lambda:((row:=self.client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'seeding fixture verifies before live cap'):raise RuntimeError('seed not ready')
                    d.DaemonContract.throttle(self.client,item.info_hash,self.throttle or None)
            self.hash=item.info_hash
            def ready():
                self.row=self.client.call('torrents.get',dict(hash=self.hash))['result']['torrent']
                ok=self.row['state']==self.state
                if self.state=='downloading':ok &= self.row.get('dl_bps',0)>0 and 0<self.row.get('completed_bytes',0)<self.row['size']
                return ok,json.dumps(self.row)
            if not self.h.wait_for(ready,'fixture reaches actual '+self.state,timeout=20):raise RuntimeError('fixture state not observed')
            return self
        except BaseException:
            self.stack.close();raise

    def __exit__(self,*_):self.stack.close()

    def description(self,state):
        roots=[str(box.root) for box,_,_,_ in self.owned]
        pids=[h.daemon_pid() for _,h,_,_ in self.owned]
        return dict(runtime=self.box.env['XDG_RUNTIME_DIR'],root=str(self.box.root),state=state,row=self.row,
                    socket=str(d.DaemonContract.socket_path(self.box)),roots=roots,pids=pids,
                    guardian=os.getpid(),units=[p.unit for _,_,p,_ in self.owned] if self.systemd else [],
                    throttle_bps=self.throttle)


def proxy(args):
    command=[sys.executable,str(Path(d.__file__).resolve()),str(args.binary.resolve()),'--serve-fixture',args.serve_fixture,'--fixture-worker',
             '--systemd' if args.systemd else '--no-systemd']
    if args.throttle is not None:command+=['--throttle',str(args.throttle)]
    if args.log_root is not None:command+=['--log-root',str(args.log_root)]
    child=subprocess.Popen(command,start_new_session=True)
    stopped=threading.Event()
    def stop(*_):
        stopped.set()
        try:child.send_signal(signal.SIGTERM)
        except ProcessLookupError:pass
    previous={number:signal.signal(number,stop) for number in (signal.SIGINT,signal.SIGTERM)}
    try:return child.wait()
    finally:
        if child.poll() is None:stop();child.wait(timeout=20)
        for number,handler in previous.items():signal.signal(number,handler)


def serve(binary,state,systemd,throttle):
    stop=threading.Event()
    previous={number:signal.signal(number,lambda *_:stop.set()) for number in (signal.SIGINT,signal.SIGTERM)}
    # The worker owns every resource. Kernel parent-death notification is set
    # after the handler, before any sandbox/client, and the startup race checked.
    parent=os.getppid()
    if ctypes.CDLL(None,use_errno=True).prctl(1,signal.SIGTERM,0,0,0)!=0:raise OSError(ctypes.get_errno(),'PR_SET_PDEATHSIG')
    if parent==1 or os.getppid()!=parent:stop.set()
    canonical='idle' if state=='empty' else state
    recipe=json.loads((fixtures.HERE/'fixtures/states'/f'{canonical}.json').read_text())
    throttle=recipe['throttle_bps'] if throttle is None else throttle
    try:
        with d.Case('daemon_fixture',state) as case:
            if stop.is_set():return 0
            with fixtures.daemon_pair(case,binary,systemd,throttle,canonical) as pair:
                description=pair.description(state)
                description['log']=str(case.path)
                print('FBTFIXTURE '+json.dumps(description),flush=True)
                poll=select.poll()
                # Existing UI callers close a pipe to signal teardown. A closed
                # stdin at invocation (e.g. /dev/null) is not an owner stop.
                pipe=stat.S_ISFIFO(os.fstat(0).st_mode)
                poll.register(0,select.POLLIN|select.POLLHUP)
                watch_stdin=pipe and not poll.poll(0)
                reason='signal'
                while not stop.wait(.1):
                    if any(p.child.poll() is not None for _,_,p,_ in pair.owned):reason='owned daemon exited';break
                    if watch_stdin and poll.poll(0):
                        if not os.read(0,1):reason='stdin EOF';break
        return d.suite_status("daemon_fixture")
    finally:
        for number,handler in previous.items():signal.signal(number,handler)
