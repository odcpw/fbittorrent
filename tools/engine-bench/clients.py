"""Private client adapters for the local head-to-head; no engine policy changes."""
import base64
import json
import os
from pathlib import Path
import signal
import socket
import struct
import time
from types import SimpleNamespace
import urllib.parse
import urllib.request

from run import ApiConn, Check, DaemonContract, wait


def owns_listener(pid,port):
    """A free-port probe alone cannot authorize mutating a WebUI on that port."""
    sockets=set()
    for path in Path(f'/proc/{pid}/fd').iterdir():
        try:sockets.add(os.readlink(path))
        except FileNotFoundError:pass
    for line in Path(f'/proc/{pid}/net/tcp').read_text().splitlines()[1:]:
        fields=line.split()
        if fields[1]==f'0100007F:{port:04X}' and fields[3]=='0A' and f'socket:[{fields[9]}]' in sockets:
            return True
    return False


class Fbittorrent:
    def __init__(self, run, binary, public=False):
        self.run = run
        self.child = run.spawn(DaemonContract.command(binary, 0, True), log='client')
        path = DaemonContract.socket_path(run.sandbox)
        wait(lambda: path.exists() or self.child.poll() is not None)
        if self.child.poll() is not None:
            raise RuntimeError((run.output/'client.log').read_text())
        self.conn = ApiConn(SimpleNamespace(sandbox=run.sandbox, case=Check()), timeout=30)
        self.pid = struct.unpack('3i', self.conn.socket.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]
        self.conn.hello()
        wait(lambda: self.call('app.info')['state'] == 'running')

    def call(self, op, args=None, **kw):
        return self.conn.call(op, args or {}, **kw)['result']

    def add(self, path, info_hash, stopped=False):
        self.hash = info_hash
        save_path=self.run.sandbox.downloads
        # Native save_path is the exact payload root; qBittorrent's Original
        # layout appends the multi-file name. Give both the identical layout.
        if b'files' in self.run.info:
            save_path=save_path/self.run.info[b'name'].decode()
            save_path.mkdir(exist_ok=True)
        return self.call('torrents.add', dict(source={'path':str(path)},
                         save_path=str(save_path), stopped=stopped, seed=False))

    def observation(self):
        row = self.call('torrents.get', {'hash':self.hash})['torrent']
        if row['state'] == 'error':
            raise RuntimeError(row)
        assert row.get('uploaded_bytes') in (None,0) and not row['upload_enabled'], row
        complete = row['state'] == 'done'
        have = None
        if row['checked'] and not complete:
            frame = self.conn.response(self.conn.send('torrents.pieces', {'hash':self.hash}))
            if frame.get('ok'):
                p = frame['result']; bits = base64.b64decode(p['have'])
                have = [bool(bits[i//8] & (128 >> (i%8))) for i in range(p['count'])]
            else:
                assert frame['error']['code'] == 'invalid_state', frame
        return dict(complete=complete, have=have, state=row['state'],
                    ready=(row.get('playable') or {}).get('ready', False),
                    completed=row.get('completed_bytes'), uploaded=row.get('uploaded_bytes'))

    def recheck(self, timeout):
        result = self.call('torrents.recheck', {'hashes':[self.hash]}, ref='bench-check')
        assert result['state'] == 'pending', result
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            event = self.conn.pushed.pop(0) if self.conn.pushed else self.conn.receive()
            if event.get('ev') == 'mutation.result' and event.get('ref') == 'bench-check':
                assert event['outcome'] == 'done', event
                return event
        raise TimeoutError('recheck')

    def stop(self):
        self.conn.close()
        if self.child.poll() is None:
            os.kill(self.pid,signal.SIGINT)
            self.child.wait(10)
        assert self.child.returncode == 0, self.child.returncode
        from behavior_checks import expected_daemon_stderr
        assert expected_daemon_stderr((self.run.output/'client.log').read_text()), 'daemon error/drain failure'


class Qbittorrent:
    def __init__(self, run, binary, public=False):
        self.run = run
        # Auth bypass is restricted to a fresh localhost-only instance. This
        # profile never refers to the owner's qBittorrent paths or credentials.
        profile = run.sandbox.root/'qb-profile'
        cfg = profile/'qBittorrent/config'
        cfg.mkdir(parents=True)
        with socket.socket() as probe:
            probe.bind(('127.0.0.1',0)); port=probe.getsockname()[1]
        (cfg/'qBittorrent.conf').write_text('[Preferences]\nWebUI\\Address=127.0.0.1\n'
            f'WebUI\\Port={port}\nWebUI\\LocalHostAuth=false\nWebUI\\HostHeaderValidation=false\n'
            'WebUI\\CSRFProtection=false\nConnection\\UPnP=false\nBittorrent\\DHT=false\n'
            'Bittorrent\\PeX=false\nBittorrent\\LSD=false\n')
        self.base = f'http://127.0.0.1:{port}/api/v2/'
        self.child = run.spawn([binary,'--confirm-legal-notice',f'--profile={profile}',f'--webui-port={port}'],log='client')
        self.pid = self.child.pid
        if run.args.strace:
            def target():
                children=Path(f'/proc/{self.child.pid}/task/{self.child.pid}/children').read_text().split()
                return int(children[0]) if len(children)==1 else None
            self.pid=wait(target,10)
        def alive():
            if self.child.poll() is not None:
                raise RuntimeError((run.output/'client.log').read_text())
            if not owns_listener(self.pid,port):return None
            try: return self.call('app/version')
            except (OSError,ValueError): return None
        self.version = wait(alive,15)
        settings = dict(save_path=str(run.sandbox.downloads), temp_path_enabled=False,
            queueing_enabled=False, preallocate_all=False, incomplete_files_ext=False,
            dht=public, pex=public, lsd=False, upnp=False, random_port=True,
            # Its global rate limiter also budgets outgoing request messages.
            # On a controlled local swarm all remote peers are seeds and the
            # wire counter must prove zero payload upload. Exempt LAN protocol
            # traffic so a 1 KiB/s cap does not turn into a download bottleneck.
            up_limit=1024, limit_lan_peers=public, dl_limit=4*1024**2 if public else 0,
            max_connec=200, max_connec_per_torrent=100, encryption=2,bittorrent_protocol=1,
            enable_multi_connections_from_same_ip=True,
            current_network_interface='' if public else 'lo',
            current_interface_address='' if public else '127.0.0.1')
        self.call('app/setPreferences', {'json':json.dumps(settings)})
        prefs = self.call('app/preferences')
        for key in ('up_limit','limit_lan_peers','dht','pex','lsd','upnp','current_network_interface','enable_multi_connections_from_same_ip','encryption','bittorrent_protocol'):
            assert prefs[key] == settings[key], (key,prefs[key],settings[key])
        self.build = self.call('app/buildInfo')
        (run.output/'qbt-preferences.json').write_text(json.dumps(prefs,indent=2))
        (run.output/'qbt-build.json').write_text(json.dumps(dict(version=self.version,build=self.build),indent=2))

    def call(self, endpoint, fields=None, file=None):
        headers = {'Referer':self.base}
        data = None
        if file:
            boundary='fbt-head-to-head-boundary'
            parts=[]
            for key,value in fields.items():
                parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode())
            parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="torrents"; filename="fixture.torrent"\r\nContent-Type: application/x-bittorrent\r\n\r\n'.encode()+Path(file).read_bytes()+f'\r\n--{boundary}--\r\n'.encode())
            data=b''.join(parts); headers['Content-Type']='multipart/form-data; boundary='+boundary
        elif fields is not None:
            data=urllib.parse.urlencode(fields).encode()
        request=urllib.request.Request(self.base+endpoint,data=data,headers=headers)
        with urllib.request.urlopen(request,timeout=15) as reply:
            text=reply.read().decode()
        try:return json.loads(text)
        except ValueError:return text

    def add(self,path,info_hash,stopped=False):
        self.hash=info_hash
        result=self.call('torrents/add',dict(savepath=str(self.run.sandbox.downloads),
            stopped=str(stopped).lower(), sequentialDownload='true',firstLastPiecePrio='true',
            autoTMM='false', contentLayout='Original'),file=path)
        assert result['success_count']==1 and result['failure_count']==0 and result['pending_count']==0,result
        assert result['added_torrent_ids']==[info_hash],result
        wait(lambda:self.call('torrents/info?hashes='+self.hash),15)
        props=self.call('torrents/info?hashes='+self.hash)[0]
        assert props['seq_dl'] and props['f_l_piece_prio'],props

    def observation(self):
        rows=self.call('torrents/info?hashes='+self.hash)
        if not rows:return dict(complete=False,have=None,state='admitting',ready=False,completed=0,uploaded=0)
        row=rows[0]
        if row['state'] in ('error','missingFiles'):raise RuntimeError(row)
        pieces=self.call('torrents/pieceStates?hash='+self.hash)
        have=[v==2 for v in pieces]
        return dict(complete=bool(have) and all(have),have=have,state=row['state'],
                    ready=False,completed=row.get('completed'),uploaded=row.get('uploaded'))

    def recheck(self,timeout):
        self.call('torrents/stop',{'hashes':self.hash})
        wait(lambda:self.call('torrents/info?hashes='+self.hash)[0]['state'].startswith('stopped'),10)
        self.call('torrents/recheck',{'hashes':self.hash})
        entered=False
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            row=self.call('torrents/info?hashes='+self.hash)[0]
            entered |= row['state'] in ('checkingUP','checkingDL','checkingResumeData')
            if entered and row['state'].startswith('stopped'):
                assert all(v==2 for v in self.call('torrents/pieceStates?hash='+self.hash))
                return row
            time.sleep(.02)
        raise TimeoutError(f'recheck: entered={entered}, row={row}')

    def stop(self):
        if self.child.poll() is None:
            self.call('app/shutdown',{})
            self.child.wait(10)
        assert self.child.returncode==0,self.child.returncode
