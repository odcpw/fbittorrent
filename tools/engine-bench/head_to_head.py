#!/usr/bin/env python3
"""Identical localhost swarms for fbittorrent and qBittorrent; private state only.

Normal trials use /proc counters. --profile trials are separate diagnostics.
Heavy runs must use the repository's shared flock/systemd memory wrapper.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import traceback
import urllib.request

from run import Run, Sandbox, private_environment, proc, delta, wait, ProcessRun, Sampler
from fixtures import bencode
from clients import Fbittorrent, Qbittorrent

MIB=1024**2
WORKLOADS={'single1':(1024*MIB,1),'single4':(4096*MIB,1),'multi2':(2048*MIB,1000)}


def memory_budget():
    """Record the current benchmark scope, including kernel/file-cache charges."""
    try:
        group=Path('/sys/fs/cgroup')/Path('/proc/self/cgroup').read_text().split(':')[-1].strip().lstrip('/')
        values={name:(group/name).read_text().strip() for name in
                ('memory.current','memory.peak','memory.high','memory.max','memory.events')}
        return dict(group=str(group),**values)
    except OSError:return None


def decode(data):
    def one(i):
        tag=data[i:i+1]
        if tag==b'i':
            j=data.index(b'e',i);return int(data[i+1:j]),j+1
        if tag in (b'd',b'l'):
            out={} if tag==b'd' else [];i+=1
            while data[i:i+1]!=b'e':
                v,i=one(i)
                if tag==b'd':
                    value,i=one(i);out[v]=value
                else:out.append(v)
            return out,i+1
        j=data.index(b':',i);n=int(data[i:j]);return data[j+1:j+1+n],j+1+n
    value,end=one(0)
    assert end==len(data)
    return value


def verify_payload(downloads,info):
    """Independently hash every actual piece, including cross-file boundaries."""
    name=info[b'name'].decode()
    files=([(downloads/name,info[b'length'])] if b'length' in info else
           [(downloads/name/Path(*(p.decode() for p in f[b'path'])),f[b'length']) for f in info[b'files']])
    sha=hashlib.sha256();piece=hashlib.sha1();used=0;index=0
    piece_len=info[b'piece length'];expected=info[b'pieces']
    for path,length in files:
        assert path.stat().st_size==length,(path,path.stat().st_size,length)
        with path.open('rb') as source:
            while chunk:=source.read(min(MIB,piece_len-used)):
                sha.update(chunk);piece.update(chunk);used+=len(chunk)
                if used==piece_len:
                    assert piece.digest()==expected[index*20:(index+1)*20],index
                    index+=1;piece=hashlib.sha1();used=0
    if used:
        assert piece.digest()==expected[index*20:(index+1)*20],index
        index+=1
    assert index*20==len(expected)
    return sha.hexdigest(),files[0][0]


class Match(Run):
    def spawn(self,argv,env=None,log='process'):
        if log=='client' and self.args.strace:
            argv=[self.args.strace,'-f','-c','-w','-o',self.output/'syscalls.txt',*argv]
        return super().spawn(argv,env,log)

    @contextmanager
    def setup_match(self,name,client,size,files,peers):
        self.output=self.args.output/name;self.output.mkdir(parents=True,exist_ok=False)
        self.sandbox=Sandbox(directory=self.args.disk_root);self.owners.register(self.sandbox.root)
        (self.output/'sandbox.json').write_text(json.dumps({'root':str(self.sandbox.root)}))
        self.sandbox.env['DBUS_SESSION_BUS_ADDRESS']='unix:path='+str(self.sandbox.root/'no-bus')
        self.sandbox.env.pop('RUST_MIN_STACK',None)
        private_environment(self.sandbox)
        self.load_begin=os.getloadavg();self.pressure_begin=self.pressure()
        self.budget_begin=memory_budget()
        self.client=None;self.profiler=None
        errors=[]
        try:
            cfg=Path(self.sandbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
            config=cfg.read_text()+'upload = "off"\n'
            public=bool(self.args.public_torrent)
            if public:
                config=config.replace('network_interface = "lo"\n','')+'download_limit_bps = 4194304\n'
            cfg.write_text(config)
            self.size=size;self.metadata=self.sandbox.root/'metadata';self.metadata.mkdir()
            prefix=self.sandbox.root/'prefix.mkv'
            subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=size=256x144:rate=24',
                '-t','4','-c:v','mpeg4','-q:v','5','-threads','1','-fflags','+bitexact',
                '-flags:v','+bitexact','-map_metadata','-1',prefix],env=self.sandbox.env,cwd=self.sandbox.root,
                check=True,timeout=30)
            torrent=self.metadata/'fixture.torrent'
            if public:
                torrent.write_bytes(self.args.public_torrent.read_bytes())
            else:
                self.tool('make',out=torrent,name='fixture.mkv' if files==1 else 'fixture-multi',size=size,
                    piece_len=256*1024,key=42,tracker='http://127.0.0.1:1/announce',
                    prefix=prefix,cache=self.args.output/'piece-cache')
            meta=decode(torrent.read_bytes())
            if files>1 and not public:
                info=meta[b'info'];del info[b'length']
                info[b'files']=[{b'length':size//files+(i<size%files),
                    b'path':[f'{i:04}.'.encode()+(b'mkv' if i==0 else b'bin')]} for i in range(files)]
            self.info=meta[b'info'];self.info_hash=hashlib.sha1(bencode(self.info)).hexdigest()
            torrent.write_bytes(bencode(meta))
            if not public:
                ready=self.sandbox.root/'seed-ready.json'
                self.seed=self.spawn([self.args.seed,'serve','--torrents',self.metadata,'--peers',peers,
                    '--tracker-port',0,'--ready',ready,'--rate',0],log='seed')
                wait(lambda:ready.exists() or self.seed.poll() is not None,10)
                binding=json.loads(ready.read_text())
                self.tracker=f"http://127.0.0.1:{binding['tracker_port']}"
                meta[b'announce']=(self.tracker+'/announce').encode();torrent.write_bytes(bencode(meta))
            # Both clients receive byte-identical info dictionaries and metadata
            # settings. Only the ephemeral localhost tracker address changes.
            (self.output/'metadata.json').write_text(json.dumps(dict(info_hash=self.info_hash,
                metadata_sha256=hashlib.sha256(bencode(self.info)).hexdigest(),bytes=size,files=files,peers=peers)))
            factory=Fbittorrent if client=='fbt' else Qbittorrent
            self.client=factory(self,self.args.binary if client=='fbt' else self.args.qbt,public=public)
            self.pid=self.client.pid
            if self.args.profile:
                self.profiler=Sampler(self.pid,self.output/'profile.json');self.profiler.start()
            yield torrent
        finally:
            original=sys.exception()
            started=time.monotonic()
            for action in ([self.profiler.stop] if self.profiler else [])+([self.client.stop] if self.client else []):
                try:action()
                except BaseException as error:errors.append(error)
            shutdown_ms=(time.monotonic()-started)*1000
            if hasattr(self,'seed') and self.seed.poll() is None:
                self.seed.terminate()
                try:self.seed.wait(3)
                except subprocess.TimeoutExpired as error:errors.append(error)
            survivors=self.owners.survivors()
            if survivors:
                errors.append(RuntimeError(f'owned survivors; emergency cleanup fails trial: {survivors}'))
                try:self.owners.cleanup(deadline=time.monotonic()+5)
                except BaseException as error:errors.append(error)
            for child in self.children:
                try:child.wait(2)
                except subprocess.TimeoutExpired as error:errors.append(error)
            for stream in self.handles:stream.close()
            remaining=self.owners.survivors()
            if not remaining:self.sandbox.temporary.cleanup()
            else:self.sandbox.temporary._finalizer.detach() # retain isolation for owned survivors
            # Retain failure cleanup evidence too: absence of a successful
            # result must never hide which children needed emergency cleanup.
            (self.output/'teardown.json').write_text(json.dumps(dict(shutdown_ms=shutdown_ms,
                survivors=len(remaining),emergency_cleanup=bool(survivors),
                qualified=not errors and original is None,
                memory_budget_after_cleanup=memory_budget(),
                children=[{'pid':child.pid,'returncode':child.returncode} for child in self.children])))
            if errors:
                if original:errors.insert(0,original)
                raise BaseExceptionGroup('client comparison teardown failed',errors)

    def download(self,name,client,size,files,peers,recheck=False):
        with self.setup_match(name,client,size,files,peers) as torrent:
            before=proc(self.pid);seed_before=proc(self.seed.pid) if hasattr(self,'seed') else None
            begin=time.monotonic();self.client.add(torrent,self.info_hash)
            first=prefix=ready=playable=None;last=None
            piece_size=self.info[b'piece length']
            target=min(8*MIB,size//files);prefix_count=(target+piece_size-1)//piece_size
            first_size=self.info.get(b'length',self.info.get(b'files',[{b'length':size}])[0][b'length'])
            playable_bytes=min(first_size,max(32*MIB,(first_size*3+99)//100))
            playable_count=(playable_bytes+piece_size-1)//piece_size
            tail_piece=(first_size-1)//piece_size
            next_progress=0
            while time.monotonic()-begin<self.args.timeout:
                row=self.client.observation();now=time.monotonic()-begin
                have=row['have']
                if now>=next_progress:
                    with (self.output/'progress.jsonl').open('a') as progress:
                        progress.write(json.dumps(dict(seconds=now,state=row['state'],completed=row['completed'],
                            have=sum(have) if have else None,**proc(self.pid)))+'\n')
                    next_progress=now+.5
                if have:
                    if first is None and any(have):first=now
                    if prefix is None and all(have[:prefix_count]):prefix=now
                    if last is None and have[-1]:last=now
                    if playable is None and all(have[:playable_count]) and have[tail_piece]:playable=now
                if ready is None and row['ready']:ready=now
                if row['complete']:
                    first=first if first is not None else now
                    prefix=prefix if prefix is not None else now
                    playable=playable if playable is not None else now
                    break
                time.sleep(.02)
            else:raise TimeoutError(f'{name}: {row}')
            measured=delta(before,proc(self.pid),time.monotonic()-begin,size)
            measured.update(admission=getattr(self.client,'admission',None),
                            memory_budget_begin=self.budget_begin,memory_budget_at_complete=memory_budget())
            if self.profiler:
                self.profiler.stop();self.profiler=None
            measured.update(first_piece_s=first,prefix_s=prefix,last_piece_s=last,playable_s=playable,app_ready_s=ready,
                seed_cpu_s=proc(self.seed.pid)['cpu_s']-seed_before['cpu_s'] if seed_before else None)
            stats=self.stats()['torrents'][0] if seed_before else None
            if stats:
                assert stats['uploaded_bytes']==0,stats
                if client=='fbt':assert stats['unchokes']==0,stats
            digest,video=verify_payload(self.sandbox.downloads,self.info)
            playable_file=self.sandbox.root/'playable-prefix.mkv'
            with video.open('rb') as source,playable_file.open('wb') as output:
                remaining=playable_bytes
                while remaining:
                    chunk=source.read(min(MIB,remaining));assert chunk
                    output.write(chunk);remaining-=len(chunk)
            if not self.args.public_torrent:
                subprocess.run(['ffmpeg','-v','error','-i',playable_file,'-frames:v','1','-f','null','-'],
                    env=self.sandbox.env,cwd=self.sandbox.root,check=True,timeout=30,stdout=subprocess.DEVNULL)
            measured.update(client=client,bytes=size,files=files,peers=peers,info_hash=self.info_hash,
                payload_sha256=digest,seed=stats,load_begin=self.load_begin,load_end=os.getloadavg(),
                instrumented=bool(self.args.profile or self.args.strace),pressure_begin=self.pressure_begin,pressure_end=self.pressure(),
                public=bool(self.args.public_torrent),uploaded_bytes=row['uploaded'])
            if recheck:
                if client=='fbt':
                    self.client.call('torrents.stop',{'hashes':[self.info_hash]})
                    wait(lambda:self.client.observation()['state']=='paused',30)
                else:
                    self.client.call('torrents/stop',{'hashes':self.info_hash})
                    wait(lambda:self.client.observation()['state'].startswith('stopped'),30)
                if self.args.profile:
                    self.profiler=Sampler(self.pid,self.output/'recheck-profile.json');self.profiler.start()
                before=proc(self.pid);begin=time.monotonic()
                self.client.recheck(self.args.timeout)
                measured['recheck']=delta(before,proc(self.pid),time.monotonic()-begin,size)
                if self.profiler:
                    self.profiler.stop();self.profiler=None
                assert verify_payload(self.sandbox.downloads,self.info)[0]==digest
            (self.output/'result.json').write_text(json.dumps(measured,indent=2)+'\n')
            print(json.dumps(dict(case=name,seconds=measured['seconds'],cpu_s=measured['cpu_s'],
                prefix_s=prefix,recheck=measured.get('recheck'))),flush=True)
            return measured


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--binary-source',required=True,help='revision or recorded patch that built the measured binary')
    p.add_argument('--qbt',type=Path,default=Path('/usr/bin/qbittorrent-nox'))
    p.add_argument('--seed',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--disk-root',type=Path,default=Path('/home/oliver/Projects/odcpw'))
    p.add_argument('--workloads',nargs='+',default=list(WORKLOADS))
    p.add_argument('--seeds',nargs='+',type=int,default=[1,4,16])
    p.add_argument('--runs',type=int,default=3)
    p.add_argument('--profile',action='store_true')
    p.add_argument('--strace',type=Path,help='separate diagnostic, never a timing comparison')
    p.add_argument('--timeout',type=float,default=180)
    p.add_argument('--pilot-mib',type=int,help='small harness pilot, excluded from comparisons')
    p.add_argument('--public-torrent',type=Path,help='small legal public sanity, capped at 200 MiB and 4 MiB/s')
    args=p.parse_args()
    if args.public_torrent:
        args.public_torrent=args.public_torrent.resolve()
        info=decode(args.public_torrent.read_bytes())[b'info']
        sizes=[info[b'length']] if b'length' in info else [f[b'length'] for f in info[b'files']]
        assert 0<sum(sizes)<=200*MIB,'public sanity exceeds byte cap'
        WORKLOADS['public']=(sum(sizes),len(sizes));args.workloads=['public'];args.seeds=[0];args.runs=1
    for key in ('binary','qbt','seed','output','disk_root'):setattr(args,key,getattr(args,key).resolve())
    if args.strace:args.strace=args.strace.resolve()
    args.output.mkdir(parents=True,exist_ok=True)
    manifest=dict(worktree=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),binary_source=args.binary_source,
        binaries={key:dict(path=str(getattr(args,key)),sha256=hashlib.file_digest(getattr(args,key).open('rb'),'sha256').hexdigest()) for key in ('binary','qbt','seed')},
        harness_sources={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in Path(__file__).parent.glob('*.py')},
        arguments={k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()})
    (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2))
    signal.signal(signal.SIGTERM,lambda *_:(_ for _ in ()).throw(KeyboardInterrupt()))
    owners=ProcessRun();failures=[]
    for index in range(args.runs):
        for workload in args.workloads:
            size,files=WORKLOADS[workload]
            if args.pilot_mib:size=args.pilot_mib*MIB
            for peers in args.seeds:
                digests=[]
                for client in (('fbt','qbt') if index%2==0 else ('qbt','fbt')):
                    name=f'{workload}-{peers}s-{index+1}-{client}'
                    run=Match(args,owners)
                    try:
                        result=run.download(name,client,size,files,peers,recheck=workload=='single4' and peers==1)
                        digests.append(result['payload_sha256'])
                    except Exception as error:
                        failure=dict(case=name,error=str(error),traceback=traceback.format_exc())
                        (run.output/'failure.json').write_text(json.dumps(failure,indent=2))
                        print(json.dumps(failure),flush=True);failures.append(failure)
                        if owners.survivors():raise
                assert len(set(digests))<=1, 'client payloads differ'
    assert not owners.survivors()
    if failures:raise SystemExit(1)


if __name__=='__main__':main()
