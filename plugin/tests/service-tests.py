#!/usr/bin/env python3
"""Real daemon fallback and scripted API subscription/reload checks; no live shell."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import sys
import time
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'agent_ergonomics_audit/audit/regression_tests'))
import daemon
import fixtures
from fbtlog import Case, suite_status
from qs_env import prepare, ROOT


class Shell:
    def __init__(self, case, sandbox, hooks=True):
        self.case = case
        self.config, self.env = prepare(sandbox, Path(__file__).with_name('service'), wayland=True)
        fixtures.path_shim('fbittorrent', 'exit 0', sandbox.root)
        self.env['FBT_TEST_HOOKS']='1' if hooks else '0'
        self.env['FBT_TEST_BINARY']=str(sandbox.root/'shims/fbittorrent')
        self.output = case.path.with_suffix('.qs.log')
        self.stream = self.output.open('w')
        self.child = case.popen(['qs', '--no-color', '-p', str(self.config)], env=self.env, stdout=self.stream)

    def ipc(self, method, *args, target="fbt-test"):
        result = subprocess.run(['qs', 'ipc', '-p', str(self.config), 'call', target, method, *map(str,args)],
                                env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=3)
        if result.returncode:
            raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()

    def wait(self, predicate, timeout=10):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            if self.child.poll() is not None:
                raise RuntimeError('owned Quickshell exited: ' + str(self.child.returncode))
            try:
                last = predicate()
                if last: return last
            except (RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
                last = str(error)
            time.sleep(.02)
        raise TimeoutError('owned shell condition timed out: ' + str(last))

    def state(self):
        return json.loads(self.ipc('state'))

    def close(self):
        if self.child.poll() is None:
            subprocess.run(['qs','kill','--pid',str(self.child.pid)], env=self.env, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=3)
            try: self.child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.child.kill(); self.child.wait(timeout=3)
        self.stream.close()
        self.case.check(self.child.returncode == 0, 'owned Quickshell closes gracefully', f'exit={self.child.returncode}; log={self.output}')
        text=self.output.read_text()
        errors=[line for line in text.splitlines() if any(token in line for token in ('TypeError:', 'ReferenceError:', 'SyntaxError:', 'Failed to load configuration', 'Cannot assign'))]
        self.case.check(not errors, 'plugin has no QML/JavaScript load or runtime errors',str(errors))


@contextmanager
def shell(case, sandbox, **options):
    process = Shell(case, sandbox, **options)
    try: yield process
    finally: process.close()


def real_fallback(binary):
    with Case('qs_service','bar_single_connection') as case, fixtures.Sandbox() as sandbox, \
            daemon.Harness(case,binary,sandbox,False) as harness:
        process=harness.start_serve()
        if not (harness.receive_ready() and harness.wait_engine()): raise RuntimeError('fixture daemon startup failed')
        try:
            with shell(case,sandbox) as q:
                q.wait(lambda: q.state()['online'])
                with daemon.ApiConn(harness) as client:
                    client.hello()
                    info=client.call('app.info')['result']
                    case.check(info['clients']['bar'] == 1 and int(q.ipc('widgets')) == 2,
                               'two headless monitor-widget instances share exactly one real daemon bar connection',
                               f"clients={info['clients']}; widgets={q.ipc('widgets')}")
                    actual=json.loads(q.ipc('state',target='oliver.torrents'))
                    case.check(actual['online'] and actual['instance']==info['instance'], 'real bar IPC hook reports the shared service identity',json.dumps(actual))
                    screens=json.loads(q.ipc('screens'))
                    case.check(True, 'record isolated widget screen bindings',json.dumps(dict(screens=screens,physical_two_monitors=len(set(screens))==2,
                               note='two screen bindings' if len(set(screens))==2 else 'SKIP physical second monitor: only one exposed; owner policy forbids changing the live compositor')))
                    case.check(q.ipc('mode') == 'poll', 'P1-06 fallback uses socket polling', q.ipc('mode'))
                    log=sandbox.root/'shims/fbittorrent.log'
                    case.check(not log.exists(), 'no CLI process is spawned for automatic status', str(log))
                    q.ipc('start')
                    q.ipc('start')
                    q.wait(lambda: log.exists())
                    case.check(log.read_text().splitlines()==['["up"]'], 'explicit Start runs exactly one fbittorrent up',log.read_text())
                    q.ipc('middle')
                    case.check(q.ipc('summon')=='oliver.torrents:{"view":"torrents"}',
                               'default middle click summons the dashboard through the shell',q.ipc('summon'))
                    fixtures.path_shim('wl-paste', "printf '%s' 'magnet:?xt=urn:btih:"+'d'*40+"&dn=Clipboard%20preview'", sandbox.root)
                    q.ipc('clipboard')
                    offer=q.wait(lambda: json.loads(q.ipc('offer')))
                    case.check(offer['name']=='Clipboard preview',
                               'clipboard read creates a named offer before a mutation is sent',json.dumps(offer))
                    case.check(not json.loads(q.ipc('state'))['pending'], 'offer itself has no mutation ref',q.ipc('state'))
                    listed=client.call('torrents.list',{})['result']['torrents']
                    case.check(listed==[], 'real daemon list remains empty until the offer is accepted',json.dumps(listed))
                    q.ipc('cancel')
                    case.check(q.ipc('offer')=='null','Cancel clears the clipboard offer',q.ipc('offer'))
                    q.ipc('pause','f'*40)
                    q.wait(lambda: bool(q.ipc('error')))
                    case.check(q.ipc('error')=='not found', 'real typed API error reaches the service without optimistic row changes',q.ipc('error'))
        finally:
            harness.stop_serve(process);harness.lock_free()


def scripted_reload():
    from scripted_api import ScriptedApi
    with Case('qs_service','reload_reconnects') as case, fixtures.Sandbox() as sandbox:
        source=ScriptedApi(sandbox)
        try:
            with shell(case,sandbox) as q:
                initial=q.wait(lambda: (state if (state:=q.state()).get('subs',{}).get('summary',0)>0 else None))
                case.check(q.ipc('backend')=='parked', 'confirmed summary state supersedes initial hello state',q.ipc('backend'))
                generation=q.ipc('generation')
                began=time.monotonic()
                q.ipc('reload')
                def reloaded():
                    state=q.state()
                    return state if q.ipc('generation')!=generation and state.get('subs',{}).get('summary',0)>0 else None
                after=q.wait(reloaded,1)
                elapsed=round((time.monotonic()-began)*1000)
                case.check(elapsed<1000 and initial['instance']==after['instance'],
                           'isolated plugin hard reload reconnects and resubscribes within 1 s, instance unchanged',
                           f'elapsed={elapsed} ms; before={initial!r}; after={after!r}; source=scripted frozen-transcript API')
                case.check(after['subs']['summary']>=1 and isinstance(after['pending'],list),
                           'state hook exposes subscription revisions and pending refs',json.dumps(after))
                artifact=case.path.with_suffix('.wire.ndjson')
                artifact.write_text(''.join(json.dumps(r)+'\n' for r in source.records))
                case.check(sum(r['op']=='hello' for r in source.records)==2,
                           'one hello per isolated service lifetime',f'hello count={sum(r["op"]=="hello" for r in source.records)}; wire={artifact}')
        finally: source.close()


def hook_guard():
    with Case('qs_service','runtime_hook_guard') as case, fixtures.Sandbox() as sandbox, shell(case,sandbox,hooks=False) as q:
        q.wait(lambda: q.ipc('widgets')=='2')
        response=q.ipc('useRuntimeDir',str(sandbox.root/'unused'),target='oliver.torrents')
        case.check(response.startswith('refused:'), 'useRuntimeDir refuses shells without FBT_TEST_HOOKS=1',response)
        case.check(not (sandbox.root/'shims/fbittorrent.log').exists(),'offline reconnect never launches the daemon','recording fbittorrent shim untouched')


if __name__=='__main__':
    import private_net
    private_net.enter()
    real_fallback(Path(sys.argv[1]).resolve())
    scripted_reload()
    hook_guard()
    sys.exit(suite_status('qs_service'))
