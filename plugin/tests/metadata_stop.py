#!/usr/bin/env python3
"""Real metadata waits and shutdown through CLI, tmux TUI and private QML.

Default: exercise the real 90-second deadline. --seconds 2 --stalled-peer
cancels an active BEP 9 exchange. --quiet-stop closes every client before
SIGTERM. --loopback checks successful metadata exchange from a real seed.
Every child is confined to private XDG and a loopback-only network namespace.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from contextlib import contextmanager
import json
import os
from pathlib import Path
import signal
import socket
import re
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

from failure_messages import surfaces
import failure_messages
import daemon
import fbtlog
import fixtures
from process_cleanup import ProcessRun


class StalledMetadataPeer:
    """A real TCP peer acknowledges BEP 9, then withholds the requested block."""
    def __init__(self,hash_):
        self.hash=bytes.fromhex(hash_);self.stop=threading.Event();self.requested=threading.Event()
        self.listener=socket.socket();self.listener.bind(('127.0.0.1',0));self.listener.listen(1);self.listener.settimeout(.1)
        self.port=self.listener.getsockname()[1];self.connection=None;self.error=None
        self.thread=threading.Thread(target=self.run,name='stalled-metadata-peer');self.thread.start()

    def exact(self,connection,length):
        data=b''
        while len(data)<length and not self.stop.is_set():
            try:part=connection.recv(length-len(data))
            except socket.timeout:continue
            if not part:raise EOFError()
            data+=part
        if len(data)!=length:raise EOFError()
        return data

    def run(self):
        try:
            while not self.stop.is_set():
                try:connection,_=self.listener.accept();break
                except socket.timeout:continue
            else:return
            self.connection=connection;connection.settimeout(.1)
            handshake=self.exact(connection,68)
            assert handshake[28:48]==self.hash,handshake.hex()
            connection.sendall(b'\x13BitTorrent protocol'+b'\0\0\0\0\0\x10\0\0'+self.hash+b'-FB0001-'+b'0'*12)
            payload=b'\x14\x00'+fixtures.bencode(dict(m=dict(ut_metadata=3),metadata_size=16384))
            connection.sendall(struct.pack('!I',len(payload))+payload)
            while not self.stop.is_set():
                length=struct.unpack('!I',self.exact(connection,4))[0]
                if length>65536:raise ValueError('peer frame exceeds fixture bound')
                payload=self.exact(connection,length)
                if payload.startswith(b'\x14\x03'):self.requested.set()
        except (EOFError,OSError):pass
        except Exception as error:self.error=repr(error)
        finally:
            if self.connection:self.connection.close()

    def close(self):
        self.stop.set();self.listener.close()
        if self.connection:self.connection.close()
        self.thread.join(timeout=2)
        assert not self.thread.is_alive(), 'owned metadata peer did not stop'


def capture(case, process, label):
    """Read only the registered private child; never relax system ptrace policy."""
    pid=getattr(process,'child',process).pid
    path=case.path.with_suffix('.'+label+'.bt.txt')
    with path.open('w') as output:
        try:
            result=subprocess.run(['gdb','-q','-nx','--batch','-ex','set debuginfod enabled off',
                                   '-ex','set pagination off','-ex','thread apply all bt','-ex','detach','-p',str(pid)],
                                  stdout=output,stderr=subprocess.STDOUT,timeout=20)
            output.write('\nGDB exit: '+str(result.returncode)+'\n')
        except (OSError,subprocess.TimeoutExpired) as error:
            # A failed diagnostic must never bypass the real stop request.
            output.write('\nStack capture failed: '+str(error)+'\n')
    return path


@contextmanager
def stop_after_failure(case, harness):
    """Observation/IPC errors must not bypass the stop deadline and stacks."""
    try:
        yield
    finally:
        process=harness.failure_process
        if process.child.poll() is None:
            step=case.step('stop after metadata observation','SIGTERM drains the private daemon within ten seconds; no SIGKILL')
            began=time.monotonic();process.child.send_signal(signal.SIGTERM)
            artifacts=[]
            try:code=process.child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                code=None;artifacts.append(capture(case,process,'interrupted_shutdown_hung'))
            elapsed=time.monotonic()-began
            step.close(code==0 and elapsed<10 and not harness.engine_path.exists(),
                       json.dumps(dict(exit=code,elapsed_s=elapsed)),artifacts)


def original(binary, index):
    """Run the unchanged original assertions, capturing both waits on failure."""
    @contextmanager
    def diagnosed_surfaces(binary,name,incompatible=False):
        with tempfile.TemporaryDirectory(prefix='fbt-meta-debug-',dir='/tmp') as root:
            wrapper=Path(root)/'fbittorrent'
            wrapper.write_text('#!'+sys.executable+'\nimport ctypes,os,sys\n'
                               'assert ctypes.CDLL(None).prctl(0x59616d61,ctypes.c_ulong(-1),0,0,0)==0\n'
                               'os.execv('+repr(str(binary))+',['+repr(str(binary))+',*sys.argv[1:]])\n')
            wrapper.chmod(0o700)
            with surfaces(wrapper,name,incompatible) as context,stop_after_failure(context[0],context[2]):
                try:yield context
                except BaseException:
                    capture(context[0],context[2].failure_process,'original_metadata_failure')
                    raise
                finally:
                    # Match the reported failure's teardown order, with the
                    # full stop deadline observed before emergency cleanup.
                    context[5].close();context[4].close();context[3].close()
    failure_messages.surfaces=diagnosed_surfaces
    try:failure_messages.metadata(binary)
    finally:failure_messages.surfaces=surfaces


def one(binary, index, readers=4, seconds=96, ui_poll=False, stalled_peer=False, kill_controller=False, quiet_stop=False, fail_observation=False):
    # PR_SET_PTRACER applies only to these disposable children, not the host.
    # The original binary and real network/metadata implementation are unchanged.
    with tempfile.TemporaryDirectory(prefix='fbt-meta-debug-',dir='/tmp') as root:
        wrapper=Path(root)/'fbittorrent'
        wrapper.write_text('#!'+sys.executable+'\nimport ctypes,os,sys\n'
                           'assert ctypes.CDLL(None).prctl(0x59616d61,ctypes.c_ulong(-1),0,0,0)==0\n'
                           'os.execv('+repr(str(binary))+',['+repr(str(binary))+',*sys.argv[1:]])\n')
        wrapper.chmod(0o700)
        with surfaces(wrapper,'metadata_stop_'+str(index)) as (case,box,h,c,q,ui),stop_after_failure(case,h):
            daemon.allow_manager_reads(h)
            hash_='c'*40
            magnet='magnet:?xt=urn:btih:'+hash_+'&dn=Peerless%20fixture&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce&x.pe=127.0.0.1:1'
            if stalled_peer:
                peer=StalledMetadataPeer(hash_);h.resources.callback(peer.close)
                tracker=h.resources.enter_context(fixtures.tracker_server(fail_first=0,interval_s=1,peers=[('127.0.0.1',peer.port)],directory=box.root))
                magnet='magnet:?'+urllib.parse.urlencode(dict(xt='urn:btih:'+hash_,dn='Peerless fixture',tr=tracker.url))
            began=time.monotonic()
            q.shell('summon',json.dumps(dict(view='torrents',add=dict(source=dict(magnet=magnet)))))
            q.wait(lambda:q.state()['view']['add'].get('owned'))
            if kill_controller:os.kill(os.getpid(),signal.SIGKILL)
            if fail_observation:raise RuntimeError('planted metadata observation failure')
            if stalled_peer:
                case.check(peer.requested.wait(5),'real loopback peer receives a BEP 9 request before cancellation',str(peer.error))
            stop=threading.Event();closing=threading.Event();counts=[0]*readers;errors=[]
            def read(index):
                try:
                    with daemon.ApiConn(h) as reader:
                        reader.hello()
                        while not stop.is_set():
                            step=case.step('loaded metadata read','matching row, or an explicit stopping/EOF response after down')
                            identifier=reader.send('torrents.get',dict(hash=hash_))
                            while True:
                                reply=reader.receive()
                                if reply is None or 'ev' not in reply:break
                            failure=(reply or {}).get('error',{})
                            if closing.is_set() and (reply is None or (failure.get('code')=='engine_unavailable' and failure.get('retryable') is True and failure.get('details',{}).get('state')=='stopping')):
                                step.close(True,reader.diagnostic(reply));break
                            valid=reply is not None and reply.get('id')==identifier and reply.get('ok') is True
                            step.close(valid,reader.diagnostic(reply))
                            if not valid:raise RuntimeError(json.dumps(reply))
                            counts[index]+=1
                            stop.wait(.05)
                except Exception as error:
                    if not stop.is_set():errors.append(str(error))
            workers=[threading.Thread(target=read,args=(i,),name='metadata-load-'+str(i)) for i in range(readers)]
            for worker in workers:worker.start()
            try:
                row={}
                sheet={}
                while time.monotonic()-began<seconds:
                    if ui_poll:
                        sheet=q.state()['view']['add']
                    row=c.call('torrents.get',dict(hash=hash_))['result']['torrent']
                    if (sheet.get('phase')=='metadata_timeout' if ui_poll else row.get('reason')=='metadata_timeout'):break
                    time.sleep(.02 if ui_poll else .2)
                if ui_poll and sheet.get('phase')=='metadata_timeout':
                    until=time.monotonic()+5
                    while row.get('reason')!='metadata_timeout' and time.monotonic()<until:
                        row=c.call('torrents.get',dict(hash=hash_))['result']['torrent']
                        time.sleep(.02)
                if not row:row=c.call('torrents.get',dict(hash=hash_))['result']['torrent']
                elapsed=time.monotonic()-began
                deadline_ok=(row.get('reason')=='metadata_timeout' and 89<=elapsed<96) if seconds>=90 else row.get('staged') is True and row.get('size') is None and row.get('error') is None
                artifacts=[]
                if not deadline_ok:artifacts.append(capture(case,h.failure_process,'metadata_missing'))
                case.step('real metadata deadline under parallel readers' if seconds>=90 else 'unresolved metadata during cancellation',
                          'metadata_timeout within 96 seconds' if seconds>=90 else 'unresolved staged record remains owned until down').close(deadline_ok,json.dumps(dict(elapsed_s=elapsed,row=row,reads=counts,reader_errors=errors)),artifacts)
                if seconds>=90:
                    q.ipc('popup')
                    q.wait(lambda:any('retry' in t.lower() for t in q.state()['popup']['texts']))
                    assert ui.contains('Enter','retry',literal=False)

                case.check(all(n>0 for n in counts) and not errors,'every load reader completed real requests without failures before shutdown',json.dumps(dict(reads=counts,errors=errors)))
                closing.set()
                if quiet_stop:
                    stop.set()
                    for worker in workers:worker.join(timeout=12)
                    ui.close();q.close();c.close()
                began_down=time.monotonic()
                step=case.step('SIGTERM after closing clients' if quiet_stop else 'CLI down with metadata and readers active',
                               'exit 0; owner and all sockets gone within ten seconds; no SIGKILL')
                artifacts=[];down=None
                try:
                    if quiet_stop:
                        h.failure_process.child.send_signal(signal.SIGTERM)
                        h.failure_process.child.wait(timeout=10)
                        code=0;output='SIGTERM reached the owned daemon PID'
                    else:
                        down=case.popen([str(wrapper),'--no-start','down','--json'],env=box.env,cwd=box.root,stdout=subprocess.PIPE,text=True)
                        output,_=down.communicate(timeout=10)
                        remaining=max(0,10-(time.monotonic()-began_down))
                        h.failure_process.child.wait(timeout=remaining)
                        code=down.returncode
                except subprocess.TimeoutExpired:
                    code=None;output='down/daemon missed ten-second deadline'
                    if down is not None and down.poll() is None:
                        artifacts.append(capture(case,down,'down_hung'))
                        down.send_signal(signal.SIGTERM);down.wait(timeout=3)
                down_elapsed=time.monotonic()-began_down
                alive=h.failure_process.child.poll() is None
                if alive:artifacts.append(capture(case,h.failure_process,'shutdown_hung'))
                step.close(code==0 and (quiet_stop or json.loads(output).get('ok') is True) and h.failure_process.child.returncode==0 and not alive and down_elapsed<10 and not h.engine_path.exists(),json.dumps(dict(exit=code,stdout=output,elapsed_s=down_elapsed,owner_alive=alive,owner_exit=h.failure_process.child.returncode)),artifacts)
                case.check(not row.get('upload_enabled') and not list(box.downloads.iterdir()),'unresolved metadata keeps upload off and never creates payload files',str(box.downloads))
                if not alive:
                    h.lock_free()
                    case.check(not h.failure_process.group_members(),'no private process-group descendant survives down',str(h.failure_process.group_members()))
                    case.check(daemon.expected_daemon_stderr(h.stderr()),'ordered drain precedes unlink and lock release',h.stderr()[-1800:])
            finally:
                stop.set()
                for worker in workers:worker.join(timeout=12)
                case.check(not any(worker.is_alive() for worker in workers),'all load readers stop within their bounds',str(counts))


def loopback(binary, index):
    """Exercise actual BEP 9 success repeatedly, independently of UI startup."""
    from local_magnet import LocalMagnet
    with fbtlog.Case('failure_messages','metadata_loopback_'+str(index)) as case,LocalMagnet(case,binary) as swarm:
        daemon.allow_manager_reads(swarm.h)
        for iteration in range(4):
            item,magnet,_=swarm.magnet(iteration)
            began=time.monotonic()
            swarm.client.call('torrents.add',dict(source=dict(magnet=magnet),stage=True))
            row={}
            while time.monotonic()-began<10:
                row=swarm.client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
                if row.get('size'):break
                time.sleep(.05)
            files=swarm.client.call('torrents.files',dict(hash=item.info_hash))['result']['files']
            case.step('local metadata exchange','real seed supplies the exact file list within ten seconds, without payload admission').close(
                row.get('size')==8*1024*1024 and len(files)==1 and row.get('staged') is True,
                json.dumps(dict(elapsed_s=time.monotonic()-began,row=row,files=files)))
            assert row.get('size'),row
            swarm.client.call('torrents.discard',dict(hash=item.info_hash))
        swarm.h.cli('down',timeout=10)
        swarm.p.wait(timeout=10,code=0)
        swarm.h.lock_free()


def main():
    import private_net
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--runs',type=int,default=1)
    parser.add_argument('--jobs',type=int,default=3)
    parser.add_argument('--readers',type=int,default=4)
    parser.add_argument('--one',type=int)
    parser.add_argument('--seconds',type=float,default=96)
    parser.add_argument('--loopback',action='store_true')
    parser.add_argument('--original',action='store_true',help='run the original failure_messages.metadata assertions unchanged')
    parser.add_argument('--ui-poll',action='store_true')
    parser.add_argument('--stalled-peer',action='store_true')
    parser.add_argument('--quiet-stop',action='store_true',help='close clients first, then signal the owned daemon directly')
    parser.add_argument('--kill-controller',action='store_true',help='negative control: kill only this private test controller after staging')
    parser.add_argument('--fail-observation',action='store_true',help='negative control: interrupt metadata observation and verify the stop guard')
    parser.add_argument('--log-root',required=True,type=Path)
    args=parser.parse_args()
    if not 1<=args.jobs<=4 or args.runs<1 or not 1<=args.readers<=16:
        parser.error('use positive runs, 1..4 jobs and 1..16 readers')
    if not (0<args.seconds<90 or args.seconds==96):
        parser.error('--seconds is an early cancellation delay below 90, or 96 for the real deadline')
    if (args.loopback or args.original) and (args.kill_controller or args.stalled_peer or args.quiet_stop or args.ui_poll or args.fail_observation or args.seconds!=96):
        parser.error('--loopback and --original are separate journeys without cancellation controls')
    if args.loopback and args.original:
        parser.error('select only one metadata journey')
    binary=args.binary.resolve();root=args.log_root.resolve()
    if args.one is not None:
        private_net.enter();fbtlog.LOG_ROOT=root
        if args.original:original(binary,args.one)
        elif args.loopback:loopback(binary,args.one)
        else:one(binary,args.one,args.readers,args.seconds,args.ui_poll,args.stalled_peer,args.kill_controller,args.quiet_stop,args.fail_observation)
        return fbtlog.suite_status('failure_messages')
    root.mkdir(parents=True,exist_ok=True)
    def run(index):
        directory=root/f'run_{index:03}';directory.mkdir()
        command=[sys.executable,'-B',__file__,'--binary',str(binary),'--one',str(index),'--seconds',str(args.seconds),'--readers',str(args.readers),'--log-root',str(directory)]
        if args.loopback:command.append('--loopback')
        if args.original:command.append('--original')
        if args.ui_poll:command.append('--ui-poll')
        if args.stalled_peer:command.append('--stalled-peer')
        if args.kill_controller:command.append('--kill-controller')
        if args.quiet_stop:command.append('--quiet-stop')
        if args.fail_observation:command.append('--fail-observation')
        began=time.monotonic()
        owned=ProcessRun();offsets={}
        def register_roots():
            # Only this explicitly launched case's generated spawn records can
            # register roots. Never infer ownership from executable/prefix scans.
            for log in directory.glob('failure_messages/*.jsonl'):
                with log.open() as source:
                    source.seek(offsets.get(log,0))
                    while True:
                        position=source.tell();line=source.readline()
                        if not line or not line.endswith('\n'):
                            source.seek(position);break
                        if '"action": "spawn ' not in line:continue
                        match=re.search(r'XDG root=(/tmp/fbt-d-[a-zA-Z0-9_-]+)\.',line)
                        if match and Path(match[1]).is_dir() and match[1] not in owned.roots:
                            owned.register(match[1]);owned.survivors()
                    offsets[log]=source.tell()
        timed_out=False
        with (directory/'output.log').open('w') as out:
            child=subprocess.Popen(command,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                end=began+210
                while child.poll() is None and time.monotonic()<end:
                    register_roots()
                    try:child.wait(timeout=.1)
                    except subprocess.TimeoutExpired:pass
                if child.poll() is None:
                    timed_out=True;child.send_signal(signal.SIGINT)
                    try:child.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        owned.cleanup();child.kill();child.wait(timeout=5)
            except BaseException:
                if child.poll() is None:
                    child.send_signal(signal.SIGINT)
                    try:child.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        try:owned.cleanup()
                        finally:child.kill();child.wait(timeout=5)
                raise
            finally:
                try:register_roots()
                finally:
                    survivors=owned.survivors()
                    owned.cleanup()
        return dict(run=index,exit=child.returncode,seconds=time.monotonic()-began,logs=str(directory),
                    timed_out=timed_out,escaped=[dict(pid=p.pid,name=p.name,root=p.root) for p in survivors],
                    survivors_after=[p.pid for p in owned.survivors()])
    results=[]
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        pending=[pool.submit(run,i) for i in range(1,args.runs+1)]
        for future in as_completed(pending):
            result=future.result();results.append(result);print(json.dumps(result),flush=True)
            (root/'summary.json').write_text(json.dumps(dict(runs=sorted(results,key=lambda r:r['run'])),indent=2)+'\n')
    return int(any(r['exit']!=0 or r['timed_out'] or r['escaped'] or r['survivors_after'] for r in results))


if __name__=='__main__':raise SystemExit(main())
