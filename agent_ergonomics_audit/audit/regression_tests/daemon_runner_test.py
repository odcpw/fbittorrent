"""Real CLI scheduling regression: a failed worker remains failed in JSONL."""
import json
import os
from pathlib import Path
import subprocess
import sys

import daemon as d
import fbtlog


def run(binary):
    with d.Case('harness','bounded_daemon_workers') as case,d.Sandbox() as box:
        d.private_environment(box)
        for forced in (False,True):
            root=case.path.parent/('forced' if forced else 'passing')
            env=dict(box.env)
            if forced:env['FBT_TEST_FORCE_FAIL']='daemon/restart_closed_never_starts'
            command=[sys.executable,str(Path(d.__file__).resolve()),str(binary),'--no-systemd','--jobs','2',
                     '--case','restart_closed_never_starts','--case','log_down_never_starts','--case','notify_off','--case','imdb_poster_atomic','--log-root',str(root)]
            out=case.path.with_suffix('.'+str(forced)+'.stdout.log')
            with out.open('w') as stream:
                process=case.popen(command,env=env,cwd=box.root,stdout=stream)
                code=process.wait(timeout=30)
            failed=[];finals=[]
            for log in (root/'daemon').glob('*.jsonl'):
                rows=[json.loads(line) for line in log.read_text().splitlines()]
                failed.extend(row for row in rows if row.get('ok') is False)
                if rows and rows[-1].get('final'):finals.append(log.stem)
            case.check(code==(1 if forced else 0) and bool(failed)==forced and
                       all(name in finals for name in ('restart_closed_never_starts','log_down_never_starts','notify_off_config','notify_off_flag','imdb_poster_atomic')),
                       'bounded independent CLI workers retain every final and propagate a case failure',
                       json.dumps(dict(exit=code,forced=forced,failed_steps=len(failed),finals=finals)))
            case.step('worker artifacts','stdout and per-case stderr retained without cross-case mixing').ok(str(root),[out,root])
    return fbtlog.suite_status('harness')

if __name__=='__main__':
    fbtlog.LOG_ROOT=Path(sys.argv[2]);sys.exit(run(Path(sys.argv[1]).resolve()))
