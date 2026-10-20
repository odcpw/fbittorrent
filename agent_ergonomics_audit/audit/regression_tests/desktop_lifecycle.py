#!/usr/bin/env python3
"""Real 1/10/30-minute SIGSTOP recovery, stale peers, tmux and private shell.

The caller's daemon and desktop are never used. The owned daemon is resumed
before cleanup even when an assertion or interrupt aborts the experiment.
"""
import argparse, json, os, signal, sys, time, socket
from contextlib import contextmanager
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]; sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests')); sys.path.insert(0,str(ROOT/'plugin/tests'))
import private_net,fixtures,daemon as d,fbtlog
from tui_smoke import PaneUi
from dashboard import Session

def value(c,hash): return c.call('torrents.get',dict(hash=hash))['result']['torrent']
def ticks(pid):
 fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split();return (int(fields[11])+int(fields[12]))/os.sysconf('SC_CLK_TCK')
@contextmanager
def connection_after_resume(h):
 attempts=[];deadline=time.monotonic()+30;client=None
 while time.monotonic()<deadline:
  try:
   client=d.ApiConn(h,timeout=1)
   identifier=client.send('hello',dict(proto=1,client=dict(name='resume-probe',version='0.6.0',kind='test'),interactive=False))
   response=client.response(identifier)
   assert response.get('ok') is True,response
   break
  except (OSError,EOFError) as error:
   attempts.append(str(error))
   if client:client.close()
   client=None;time.sleep(.2)
 assert client is not None,attempts
 h.case.check(response.get('ok') is True,'resume handshake completes within its bounded recovery window',json.dumps(dict(transient_transport_errors=attempts)))
 try:yield client
 finally:client.close()

def run(binary,seconds,log,saturate=False):
 fbtlog.LOG_ROOT=log.resolve();private_net.enter()
 with fbtlog.Case('lifecycle',f'suspend_{seconds}') as case, fixtures.daemon_pair(case,binary,throttle=65536) as pair:
  ui=PaneUi(case,str(binary),pair.box,pair.h,session='fbt-suspend-'+str(seconds));q=None;pid=pair.h.daemon_pid();stopped=False;fillers=[]
  try:
   ui.start_owned();q=Session(case,pair.box);q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
   q.wait(lambda:pair.hash in q.ipc('renderedRows'))
   before=value(pair.client,pair.hash);start=time.monotonic();os.kill(pid,signal.SIGSTOP);stopped=True
   deadline=time.monotonic()+3
   while not Path(f'/proc/{pid}/status').read_text().split('State:')[1].lstrip().startswith('T'):
    assert time.monotonic()<deadline;time.sleep(.01)
   assert case.check(Path(f'/proc/{pid}/status').read_text().split('State:')[1].lstrip().startswith('T'),'owned daemon is stopped by SIGSTOP',str(pid))
   if saturate:
    for _ in range(8192):
     connection=socket.socket(socket.AF_UNIX);connection.setblocking(False)
     if connection.connect_ex(str(d.DaemonContract.socket_path(pair.box)))!=0:
      connection.close();break
     fillers.append(connection)
    assert case.check(bool(fillers) and len(fillers)<8192,'owned listener backlog is saturated while its daemon cannot accept',str(len(fillers)))
   # Destroy the old peer connection while the target cannot consume its EOF.
   seedbox,seed,sp,peer=pair.owned[1];seed.stop_serve(sp);seed.notify.close()
   replacement=seed.start_serve(idle_exit_secs=0);assert seed.receive_ready() and seed.wait_engine()
   while time.monotonic()-start < seconds:
    assert pair.p.child.poll() is None and replacement.child.poll() is None
    assert q.child.poll() is None and q.compositor.child.poll() is None
    time.sleep(max(0,min(1,seconds-(time.monotonic()-start))))
   for connection in fillers:connection.close()
   fillers=[]
   resume=time.monotonic();cpu=ticks(pid);os.kill(pid,signal.SIGCONT);stopped=False
   with connection_after_resume(pair.h) as client:
    assert pair.h.wait_for(lambda:((r:=value(client,pair.hash))['completed_bytes']>=before['completed_bytes']+262144 and r.get('dl_bps',0)>0,json.dumps(r)), 'stale peer reconnects and verified payload/rate recover without daemon restart',timeout=90)
    recovered=time.monotonic()-resume
    assert ui.observe('read resumed TUI','the TUI resumes the actual transfer row',lambda pane:'fixture-download.bin' in pane and 'Connecting' not in pane,timeout=15)
    try:q.wait(lambda:q.state().get('online') is True and pair.hash in q.ipc('renderedRows'),timeout=20)
    except TimeoutError:
     case.check(False,'headless shell reconnects after resume',json.dumps(dict(state=q.state(),rows=q.ipc('renderedRows'))));raise
    trackers=client.call('torrents.trackers',dict(hash=pair.hash))['result']['trackers']
    assert case.check(any(t.get('last_announce_at',0)>=time.time()-max(20,recovered+5) and t['status']=='ok' for t in trackers),'tracker re-announces after resume',json.dumps(trackers))
    assert case.check(ticks(pid)-cpu < max(5,recovered*.5),'resume does not burn CPU replaying elapsed timer ticks',json.dumps(dict(suspended=seconds,recovered_s=recovered,cpu_s=ticks(pid)-cpu,row=value(client,pair.hash))))
    ui.observe('capture healthy UI','actual downloading row remains navigable',lambda pane:'fixture-download.bin' in pane)
    case.path.with_suffix('.result.json').write_text(json.dumps(dict(suspended_s=seconds,recovered_s=recovered,cpu_s=ticks(pid)-cpu,trackers=trackers,row=value(client,pair.hash)),indent=2))
  finally:
   for connection in fillers:connection.close()
   if stopped:os.kill(pid,signal.SIGCONT)
   ui.close()
   if q:q.close()
 return fbtlog.suite_status('lifecycle')
if __name__=='__main__':
 def interrupt(*_):raise KeyboardInterrupt
 signal.signal(signal.SIGTERM,interrupt)
 p=argparse.ArgumentParser();p.add_argument('binary',type=Path);p.add_argument('seconds',type=int,choices=[60,600,1800]);p.add_argument('--saturate',action='store_true');p.add_argument('--log-root',type=Path,required=True);a=p.parse_args();sys.exit(run(a.binary.resolve(),a.seconds,a.log_root,a.saturate))
