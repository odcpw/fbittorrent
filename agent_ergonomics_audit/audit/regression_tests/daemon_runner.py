"""Bounded process-isolated suite runner; no shared Suite stderr or case state."""
from concurrent.futures import ThreadPoolExecutor, as_completed
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time

import daemon as d
import fbtlog


def run(binary,names,systemd,jobs,validator=None,budget=None):
    started=time.monotonic()
    suite=fbtlog.get_suite('daemon')
    env=dict(os.environ)
    # Prepare two real executable variants once before workers start; concurrent
    # cargo writes must never replace an executable being launched by a case.
    if set(names)&{'upgrade_idle_exit_75','upgrade_pending_while_busy','restart_now'}:
        import daemon_upgrade
        with d.Case('daemon','upgrade_fixture_builds') as case,d.Sandbox() as box:
            old,new=daemon_upgrade.binaries(case,box)
            env.update(FBT_UPGRADE_OLD_BINARY=str(old),FBT_UPGRADE_NEW_BINARY=str(new))
    live=set();lock=threading.Lock()
    def worker(name):
        root=fbtlog.LOG_ROOT/'workers'/name
        root.mkdir(parents=True,exist_ok=True)
        command=[sys.executable,str(Path(d.__file__).resolve()),str(binary),'--case',name,'--jobs','1',
                 '--systemd' if systemd else '--no-systemd','--log-root',str(root)]
        if validator:command+=['--contract-validator',str(validator)]
        with (root/'worker.stdout.log').open('w') as out,(root/'worker.stderr.log').open('w') as err:
            process=subprocess.Popen(command,env=env,cwd=d.REPO,stdout=out,stderr=err)
            with lock:live.add(process)
            try:return name,process.wait(),root
            finally:
                with lock:live.discard(process)
    executor=ThreadPoolExecutor(max_workers=jobs)
    try:
        pending=[executor.submit(worker,name) for name in names]
        for future in as_completed(pending):
            name,code,root=future.result()
            finals=[];failed=False
            for log in (root/'daemon').glob('*.jsonl'):
                rows=[json.loads(line) for line in log.read_text().splitlines()]
                # JSONL artifacts (for example poster reader samples) are not
                # Case logs and have no final verdict. Keep them in workers/.
                if not rows or not isinstance(rows[0],dict) or rows[0].get('suite')!='daemon':continue
                failed|=not rows or not rows[-1].get('final') or any(row.get('ok') is False for row in rows)
                if rows and rows[-1].get('final'):finals.append(dict(rows[-1],case=log.stem))
                shutil.copy2(log,suite.directory/log.name)
                daemon_log=log.with_suffix('.daemon.log')
                if daemon_log.exists():shutil.copy2(daemon_log,suite.directory/daemon_log.name)
            # Make a worker crash/missing final an explicit failed JSONL step;
            # the umbrella summary must not silently omit an uncompleted case.
            skipped=code==0 and not failed and bool(finals) and all(row.get('status')=='skip' for row in finals)
            with d.Case('daemon','worker_'+name) as case:
                if skipped:case.skip('isolated worker skipped its coverage')
                else:case.check(code==0 and not failed and bool(finals),
                                'isolated case process exits 0 with a final passing result',
                                json.dumps(dict(exit=code,finals=finals)))
                case.write(dict(diagnostic='isolated worker output',artifacts=[str(root/'worker.stdout.log'),str(root/'worker.stderr.log')]))
            suite.results.extend(finals)
            print(('FAIL' if code or failed else 'SKIP' if skipped else 'ok')+': daemon/'+name,flush=True)
    finally:
        for future in locals().get('pending',[]):future.cancel()
        with lock:children=list(live)
        for child in children:
            if child.poll() is None:child.send_signal(signal.SIGINT)
        for child in children:
            try:child.wait(timeout=30)
            except subprocess.TimeoutExpired:child.kill();child.wait(timeout=5)
        executor.shutdown(wait=True,cancel_futures=True)
    elapsed=time.monotonic()-started
    # Shared-machine load is information, not a product failure. Individual
    # behavior checks still enforce their own stop/deadline requirements.
    print(f'INFO daemon timing: {elapsed:.3f} seconds ({jobs} isolated workers; '
          f'{len(names)} cases; reference budget={budget})')
    return fbtlog.suite_status('daemon')
