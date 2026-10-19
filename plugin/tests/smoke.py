#!/usr/bin/env python3
"""Service -> real private daemon -> reducer smoke, with an owner bar-count guard."""
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import socket
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


def owner_info(path):
    if not path.is_socket():return dict(skip='no owner api.sock; no daemon started')
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(2);connection.connect(str(path))
            with connection.makefile('rb') as stream:
                def call(frame):
                    connection.sendall((json.dumps(frame)+'\n').encode())
                    while True:
                        line=stream.readline(1048577)
                        if not line or len(line)>1048576 or not line.endswith(b'\n'):raise ValueError('invalid or oversized owner reply')
                        reply=json.loads(line)
                        if reply.get('id')==frame['id']:
                            if not reply.get('ok'):raise ValueError(reply.get('error',{}).get('code','protocol error'))
                            return reply['result']
                call(dict(id=1,op='hello',args=dict(proto=1,client=dict(name='smoke-count-probe',version='smoke',kind='test'),interactive=False)))
                info=call(dict(id=2,op='app.info'))
                return dict(instance=info['instance'],bar=info['clients']['bar'])
    except (OSError,ValueError,KeyError) as error:return dict(skip='owner count unavailable: '+str(error))


def readiness(case,harness):
    if not harness.receive_ready():raise RuntimeError('owned daemon READY missing')
    help_text=subprocess.run([harness.binary,'status','--help'],env=harness.sandbox.env,stdout=subprocess.PIPE,text=True,check=True).stdout
    status_flag='--quiet' if '--quiet' in help_text else '--json'
    began=time.monotonic();last=None
    while time.monotonic()-began<10:
        last=subprocess.run([harness.binary,'--no-start','status',status_flag],env=harness.sandbox.env,cwd=harness.sandbox.root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=2)
        if last.returncode==0:
            now=time.time()*1000
            case.check(True,'isolated read-only status is ready within 10 s',f'{status_flag}: exit=0 after {round((time.monotonic()-began)*1000)} ms; --json is the current pre-P1-11 compatibility path')
            return now
        time.sleep(.03)
    raise RuntimeError(f'owned status readiness exit {last.returncode}')


def main():
    binary=Path(os.environ.get('FBT_BIN',ROOT/'target/release/fbittorrent')).resolve()
    owner_path=Path(os.environ.get('XDG_RUNTIME_DIR',f'/run/user/{os.getuid()}'))/'fbittorrent/api.sock'
    before=owner_info(owner_path)
    with Case('qs_smoke','owner_guard') as guard:
        guard.check(True,'record read-only owner bar count before isolated tests',json.dumps(before))
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
        after=owner_info(owner_path)
        if before.get('skip') or after.get('skip'):
            guard.check(True,'owner bar-count guard is explicitly skipped without a usable running owner daemon',json.dumps(dict(before=before,after=after)))
        elif before['instance']!=after['instance']:
            guard.check(True,'owner bar-count guard skipped because owner instance changed independently',json.dumps(dict(before=before,after=after)))
        else:
            guard.check(before['bar']==after['bar'],'owner clients.bar is unchanged by private tests',json.dumps(dict(before=before,after=after)))
    return suite_status('qs_smoke')

if __name__=='__main__':sys.exit(main())
