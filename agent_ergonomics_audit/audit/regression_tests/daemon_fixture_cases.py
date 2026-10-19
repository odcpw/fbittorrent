"""P1-07b fixture lifetime and real peer-pair qualification."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

import daemon as d


@contextmanager
def launched(case,box,h,state,systemd,stdin=subprocess.PIPE,throttle=None):
    env=dict(box.env,FBT_TEST_LOG_ROOT=str(case.path.parents[1]/'fixture-workers'/case.path.stem/(state+'-'+str(time.time_ns()))))
    d.private_environment(box)
    command=[sys.executable,str(Path(d.__file__).resolve()),h.binary,'--serve-fixture',state,
             '--systemd' if systemd else '--no-systemd']
    if throttle is not None:command+=['--throttle',str(throttle)]
    child=case.popen(command,env=env,cwd=box.root,stdin=stdin,stdout=subprocess.PIPE,text=True,start_new_session=True)
    try:
        ready=select.select([child.stdout],[],[],25)[0]
        line=child.stdout.readline() if ready else ''
        if not case.check(line.startswith('FBTFIXTURE '),'real fixture command emits its owned socket and summary before the deadline',line):raise RuntimeError('fixture never ready')
        info=json.loads(line.removeprefix('FBTFIXTURE '))
        artifact=case.path.with_suffix('.'+state+'.fixture.json');artifact.write_text(json.dumps(info,indent=2))
        case.check(Path(info['socket']).exists() and all(d.process_alive(pid) for pid in info['pids']),
                   'reported fixture sockets/PIDs are real and owned',json.dumps(info))
        yield child,info
    finally:
        if child.poll() is None:child.terminate();child.wait(timeout=25)
        if child.stdin is not None and not child.stdin.closed:child.stdin.close()
        child.stdout.close()


def gone(h,info):
    def cleaned():
        roots=[root for root in info['roots'] if Path(root).exists()]
        alive=[pid for pid in [info['guardian'],*info['pids']] if d.process_alive(pid)]
        states={}
        for unit in info['units']:
            result=subprocess.run([d.shutil.which('systemctl',path=os.defpath),'--user','show',unit,'--property=LoadState','--value'],env=d.manager_environment(h.sandbox),cwd=h.sandbox.root,capture_output=True,text=True,timeout=3)
            states[unit]=(result.returncode,result.stdout.strip())
        ok=not roots and not alive and all(code==0 and state=='not-found' for code,state in states.values())
        return ok,json.dumps(dict(remaining_roots=roots,live_pids=alive,units=states))
    if not h.wait_for(cleaned,'all fixture units, processes and sandbox directories are removed',timeout=20):raise RuntimeError('fixture cleanup not proved')


def fixture_serving_states(binary,systemd):
    with d.Case('daemon','fixture_serving_states') as case,d.Sandbox() as box,d.Harness(case,binary,box) as h:
        for state in ['idle','seeding','paused','downloading']:
            with launched(case,box,h,state,systemd,throttle=65536 if state=='downloading' else None) as (child,info):
                case.check((info['row'] is None if state=='idle' else info['row']['state']==state),
                           'fixture recipe is realized by actual engine state '+state,json.dumps(info))
                if state=='downloading':case.check(info['throttle_bps']==65536,'fixture CLI forwards its explicit seed throttle',json.dumps(info))
                child.terminate();code=child.wait(timeout=25)
                case.check(code==0,'SIGTERM cleanly ends fixture launcher',f'exit={code}; state={state}')
                gone(h,info)


def fixture_launcher_killed(binary,systemd):
    with d.Case('daemon','fixture_launcher_killed') as case,d.Sandbox() as box,d.Harness(case,binary,box) as h:
        with launched(case,box,h,'downloading',systemd) as (child,info):
            child.kill();code=child.wait(timeout=5)
            case.check(code==-signal.SIGKILL,'kill the fixture launcher after actual downloading is observed',f'launcher={child.pid}; exit={code}')
            gone(h,info)


def fixture_daemon_killed(binary,systemd):
    with d.Case('daemon','fixture_daemon_killed') as case,d.Sandbox() as box,d.Harness(case,binary,box) as h:
        with launched(case,box,h,'downloading',systemd) as (child,info):
            os.kill(info['pids'][0],signal.SIGKILL)
            code=child.wait(timeout=25)
            case.check(code==0,'fixture guardian handles owned daemon death and releases its peer',f'killed={info["pids"][0]}; launcher exit={code}')
            gone(h,info)


def fixture_stdin_lifetime(binary,systemd):
    with d.Case('daemon','fixture_stdin_lifetime') as case,d.Sandbox() as box,d.Harness(case,binary,box) as h:
        with launched(case,box,h,'idle',systemd) as (child,info):
            child.stdin.close();code=child.wait(timeout=25)
            case.check(code==0,'UI-style pipe EOF tears down fixture cleanly',f'exit={code}')
            gone(h,info)
        with launched(case,box,h,'downloading',systemd,stdin=subprocess.DEVNULL) as (child,info):
            time.sleep(.2)
            case.check(child.poll() is None,'initially closed stdin retains a fixture until explicitly killed',json.dumps(info))
            child.terminate();code=child.wait(timeout=25)
            case.check(code==0,'closed-stdin fixture accepts explicit owner stop',str(code));gone(h,info)


def fixture_peer_pair(binary,systemd):
    with d.Case('daemon','fixture_peer_pair') as case:
        began=time.monotonic()
        with d.fixtures.daemon_pair(case,binary,systemd,32768) as pair:
            info=pair.description('downloading')
            row=pair.row
            before=row['completed_bytes'];time.sleep(1.1)
            later=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
            case.check(later['completed_bytes']>before and later['state']=='downloading',
                       'two real managed serve daemons exchange capped loopback bytes',json.dumps(dict(before=row,after=later)))
        case.check(all(not Path(root).exists() for root in info['roots']) and all(not d.process_alive(pid) for pid in info['pids']),
                   'peer-pair context removes both private daemons and sandboxes',json.dumps(info))
        case.check(time.monotonic()-began<180,'peer-pair CI case remains under three minutes',str(time.monotonic()-began))
