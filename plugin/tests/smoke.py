#!/usr/bin/env python3
"""Service -> real private daemon -> reducer smoke, with isolated daemon lifecycle checks."""
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
import daemon
import fixtures
from fbtlog import Case,suite_status
from qs_env import prepare

spec=importlib.util.spec_from_file_location('qs_log',Path(__file__).with_name('qs-log.py'))
qs_log=importlib.util.module_from_spec(spec);spec.loader.exec_module(qs_log)


def readiness(case,harness):
    if not harness.receive_ready():raise RuntimeError('owned daemon READY missing')
    began=time.monotonic();last=None
    while time.monotonic()-began<10:
        last=subprocess.run([harness.binary,'--no-start','status','--quiet'],env=harness.sandbox.env,cwd=harness.sandbox.root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=2)
        if last.returncode==0:
            now=time.time()*1000
            return now
        time.sleep(.03)
    raise RuntimeError(f'owned status readiness exit {last.returncode}')


def main():
    binary=Path(os.environ.get('FBT_BIN',ROOT/'target/release/fbittorrent')).resolve()
    with Case('qs_smoke','private_lifecycle') as guard:
        import private_net
        with fixtures.Sandbox() as sandbox:
            config,env=prepare(sandbox,Path(__file__).with_name('smoke'))
            env['QS_NO_RELOAD_POPUP']='1'
            env['FBT_QS_VERSION']=subprocess.check_output(['qs','--version'],text=True).strip()
            probe=['timeout','15s','qs','--no-color','-p',str(config/'probe-ui.qml')]
            original=dict(os.environ)
            try:
                os.environ.clear();os.environ.update(env)
                probe_code=qs_log.run('qs_smoke',probe)
            finally:os.environ.clear();os.environ.update(original)
            guard.check(probe_code==0,'installed Commons/Ui probe passes',f'probe exit={probe_code}')
        with Case('qs_smoke','daemon_lifecycle') as case,fixtures.Sandbox() as sandbox,daemon.Harness(case,binary,sandbox,False) as harness:
            process=harness.start_serve(600,no_notify=True)
            ready_ms=readiness(case,harness)
            config,env=prepare(sandbox,Path(__file__).with_name('smoke'))
            stamp=config/'ready.json';stamp.write_text('{"ready_ms":0}')
            env.update(FBT_SMOKE_HASH=secrets.token_hex(20),FBT_SMOKE_READY_FILE=str(stamp),QS_NO_RELOAD_POPUP='1')
            restarts=0
            def on_line(line):
                nonlocal process,restarts
                if 'FBTQS-WAIT reconnect' not in line:return
                restarts+=1
                harness.stop_serve(process);harness.lock_free();harness.notify.close()
                process=harness.start_serve(600,no_notify=True)
                ready=readiness(case,harness)
                stamp.write_text(json.dumps(dict(ready_ms=ready)))
            original=dict(os.environ)
            try:
                os.environ.clear();os.environ.update(env)
                code=qs_log.run('qs_smoke',['timeout','60s','qs','--no-color','-p',str(config)],on_line=on_line)
            finally:
                os.environ.clear();os.environ.update(original)
                harness.stop_serve(process);harness.lock_free()
            case.check(code==0 and restarts==1,'smoke completes with one owned daemon restart',f'exit={code}; restarts={restarts}')
            text=case.suite.stderr_path.read_text()
            errors=[line for line in text.splitlines() if re.search(r'TypeError:|ReferenceError:|SyntaxError:|Failed to load configuration|Cannot assign|Binding loop detected',line)]
            case.check(not errors,'no QML errors during real mutation/reconnect flow',str(errors))
    return suite_status('qs_smoke')

if __name__=='__main__':
    import private_net
    private_net.enter()
    sys.exit(main())
