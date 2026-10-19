"""Local wire fixture for UI protocol tests, never a live-daemon acceptance stand-in."""
from copy import deepcopy
import json
from pathlib import Path
import socket
import threading

ROOT=Path(__file__).resolve().parents[2]


class ScriptedApi:
    def __init__(self, sandbox):
        self.path=Path(sandbox.env['XDG_RUNTIME_DIR'])/'fbittorrent/api.sock'
        self.path.parent.mkdir(exist_ok=True)
        self.socket=socket.socket(socket.AF_UNIX)
        self.socket.bind(str(self.path));self.socket.listen();self.socket.settimeout(.1)
        hello=[json.loads(l) for l in (ROOT/'contracts/ipc/v1/hello.ndjson').read_text().splitlines()]
        self.hello=next(r['frame']['result'] for r in hello if r['dir']=='s2c')
        frames=[json.loads(l) for l in (ROOT/'contracts/ipc/v1/subscribe.ndjson').read_text().splitlines()]
        self.frames=[r['frame'] for r in frames if r['dir']=='s2c' and r['frame'].get('topic') in ('summary','torrents')]
        self.records=[];self.peers=[];self.workers=[];self.lock=threading.Lock();self.done=threading.Event()
        self.thread=threading.Thread(target=self.accept);self.thread.start()

    def accept(self):
        while not self.done.is_set():
            try: peer,_=self.socket.accept()
            except socket.timeout: continue
            except OSError: break
            peer.settimeout(1)
            with self.lock: self.peers.append(peer)
            worker=threading.Thread(target=self.serve,args=(peer,));self.workers.append(worker);worker.start()

    def serve(self, peer):
        sub=0
        try:
            with peer.makefile('rb') as reader:
                while not self.done.is_set():
                    line=reader.readline(65537)
                    if not line or len(line)>65536: break
                    frame=json.loads(line)
                    with self.lock: self.records.append(frame)
                    op=frame['op']
                    if op=='hello': result=deepcopy(self.hello)
                    elif op=='subscribe':
                        sub+=1;result=dict(sub=sub,topics=frame['args']['topics'])
                    else: result={}
                    peer.sendall(json.dumps(dict(id=frame['id'],ok=True,result=result),separators=(',',':')).encode()+b'\n')
                    if op=='subscribe':
                        revision=0
                        for original in self.frames:
                            if original['topic'] not in result['topics']: continue
                            event=deepcopy(original);revision+=1;event.update(sub=sub,rev=revision)
                            peer.sendall(json.dumps(event,separators=(',',':')).encode()+b'\n')
        except (OSError,ValueError): pass
        finally: peer.close()

    def close(self):
        self.done.set();self.socket.close()
        with self.lock:
            for peer in self.peers:
                try: peer.shutdown(socket.SHUT_RDWR)
                except OSError: pass
        self.thread.join(2)
        for worker in self.workers: worker.join(2)
        if self.thread.is_alive() or any(t.is_alive() for t in self.workers): raise TimeoutError('scripted API thread did not join')
