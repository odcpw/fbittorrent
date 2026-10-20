#!/usr/bin/env python3
"""Behavior controls for gate orchestration, with small disposable commands."""
import contextlib
import fcntl
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate


class GateTests(unittest.TestCase):
    def test_python_entrypoint_keeps_its_sibling_imports(self):
        with tempfile.TemporaryDirectory(prefix='fbt-python-entry-') as directory:
            root=Path(directory)
            (root/'sibling.py').write_text('VALUE="actual sibling module"\n')
            script=root/'journey.py'
            script.write_text('from sibling import VALUE\nassert VALUE=="actual sibling module"\nprint(VALUE)\n')
            check=gate.python_check('entrypoint',str(script))
            self.assertTrue(gate.run_checks([check],root/'logs',dict(os.environ)))
            self.assertEqual(check.status,'PASS')
            self.assertIn('actual sibling module',Path(check.log).read_text())

    def test_release_wrapper_reuses_an_inherited_lock(self):
        with tempfile.NamedTemporaryFile(prefix='fbt-gate-lock-') as lock, patch.object(gate, 'RELEASE_LOCK', lock.name):
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            command = gate.release_scope(['true'])
            self.assertEqual(command[0], 'systemd-run')
            self.assertIn('MemoryMax=14G', command)
            self.assertIn('FBT_GATE_RELEASE_SCOPE=1', command)
        with patch.object(gate, 'RELEASE_LOCK', '/nonexistent/gate-lock'):
            self.assertEqual(gate.release_scope(['true'])[:4], ['flock', '-w', '7200', '/nonexistent/gate-lock'])

    def test_retry_is_serial_and_retains_both_attempts(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary)
            script = root / 'case.py'
            script.write_text('''import json,sys,time
from pathlib import Path
root=Path(sys.argv[1]);name=sys.argv[2]
log=Path(sys.argv[sys.argv.index('--log-root')+1]);(log/'daemon').mkdir()
marker=root/(name+'.ran');again=marker.exists();marker.touch()
if name=='slow':time.sleep(.1);(root/'slow.done').touch()
if name=='flaky' and again:assert (root/'slow.done').exists()
ok=name=='slow' or (name=='flaky' and again)
(log/'daemon'/f'{name}.jsonl').write_text(json.dumps(dict(suite='daemon',case=name,final=True,ok=ok))+'\\n')
print('attempt',2 if again else 1)
sys.exit(0 if ok else 1)
''')
            checks = [gate.Check('daemon/' + name, [sys.executable, str(script), str(root), name], case=True)
                      for name in ('flaky', 'slow', 'broken')]
            self.assertFalse(gate.run_checks(checks, root / 'logs', dict(os.environ), jobs=3))
            self.assertEqual([c.status for c in checks], ['FLAKY', 'PASS', 'FAIL'])
            for name in ('flaky', 'broken'):
                first = root / 'logs/daemon' / name / 'first'
                retry = first.parent / 'retry'
                self.assertIn('attempt 1', (first / 'output.log').read_text())
                self.assertIn('attempt 2', (retry / 'output.log').read_text())
                self.assertFalse(json.loads((first / 'daemon' / (name + '.jsonl')).read_text())['ok'])
            self.assertFalse((root / 'logs/daemon/slow/retry').exists())

    def test_missing_final_cannot_pass(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            check = gate.Check('daemon/crashed', [sys.executable, '-c', 'print("no verdict")'], case=True)
            self.assertFalse(gate.run_checks([check], Path(temporary), dict(os.environ)))
            self.assertEqual(check.status, 'FAIL')

    def test_rust_retry_executes_only_the_failed_test(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary)
            cargo = root / 'cargo'
            cargo.write_text('''#!/usr/bin/env python3
import json,sys
from pathlib import Path
root=Path(__file__).parent
with (root/'calls').open('a') as out:out.write(json.dumps(sys.argv[1:])+'\\n')
if '--exact' in sys.argv:
    assert 'engine::flaky' in sys.argv
    print('test result: ok. 1 passed; 0 failed; 10 filtered out;')
else:
    print('interleaved output hides the test name\\nfailures:\\n    engine::flaky\\n\\ntest result: FAILED. 10 passed; 1 failed;')
    sys.exit(1)
''')
            cargo.chmod(0o700)
            rustc = root / 'rustc'; rustc.write_text("#!/bin/sh\necho 'host: test-host'\n"); rustc.chmod(0o700)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'])
            result = gate.rust_tests('fast', root / 'logs', env)
            self.assertEqual(result.status, 'FLAKY')
            calls = [json.loads(line) for line in (root / 'calls').read_text().splitlines()]
            self.assertEqual(len(calls), 2)
            self.assertEqual(calls[1][-5:], ['engine::flaky', '--', '--exact', '--nocapture', '--test-threads=1'])
            self.assertTrue((root / 'logs/rust_tests/first/output.log').is_file())
            self.assertTrue((root / 'logs/rust_retry/engine_flaky/retry/output.log').is_file())
            cargo.write_text("#!/bin/sh\necho 'test result: ok. 0 passed; 0 failed;'\n")
            result = gate.rust_tests('fast', root / 'empty', env)
            self.assertEqual(result.status, 'FAIL')

    def test_hung_rust_test_is_killed_named_and_retried_alone(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary)
            binary = root / 'tests'
            binary.write_text('''#!/usr/bin/env python3
import subprocess,sys,time
from pathlib import Path
root=Path(__file__).parent
if '--list' in sys.argv:
    if '--ignored' in sys.argv:print('tests::preexisting_ignore: test');sys.exit(0)
    print('tests::preexisting_ignore: test')
    if '--exact' not in sys.argv:print('tests::good: test')
    print('tests::parked: test');sys.exit(0)
if '--exact' in sys.argv:
    assert 'tests::parked' in sys.argv and '--test-threads=1' in sys.argv
    print('test tests::parked ... ok\\ntest result: ok. 1 passed; 0 failed;');sys.exit(0)
child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'])
(root/'child.pid').write_text(str(child.pid))
print('test tests::good ... ok',flush=True)
print('test tests::parked has been running for over 60 seconds',flush=True)
time.sleep(60)
''')
            binary.chmod(0o700)
            cargo = root / 'cargo'
            cargo.write_text('''#!/usr/bin/env python3
import json,subprocess,sys
from pathlib import Path
root=Path(__file__).parent
with (root/'calls').open('a') as out:out.write(json.dumps(sys.argv[1:])+'\\n')
runner=json.loads(sys.argv[sys.argv.index('--config')+1].split('=',1)[1])
split=sys.argv.index('--');args=sys.argv[split+1:]
if '--exact' in args:args.insert(0,sys.argv[split-1])
sys.exit(subprocess.call([*runner,str(root/'tests'),*args]))
''')
            cargo.chmod(0o700)
            rustc = root / 'rustc'; rustc.write_text("#!/bin/sh\necho 'host: test-host'\n"); rustc.chmod(0o700)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'], FBT_GATE_RUST_TIMEOUT_SECS='.15')
            started = time.monotonic()
            result = gate.rust_tests('full', root / 'logs', env)
            self.assertLess(time.monotonic()-started, 5)
            self.assertEqual(result.status, 'FLAKY')
            self.assertEqual(result.detail, 'tests::parked')
            self.assertEqual(len((root / 'calls').read_text().splitlines()), 2)
            self.assertIn('GATE-HUNG-TEST: tests::parked', (root / 'logs/rust_tests/first/output.log').read_text())
            proc = Path('/proc') / (root / 'child.pid').read_text() / 'stat'
            self.assertTrue(not proc.exists() or proc.read_text().split(') ',1)[1].startswith('Z'), 'timed-out child must not remain running')

    def test_hung_workflow_is_bounded_and_stays_red_after_retry(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary)
            check = gate.Check('daemon/parked', [sys.executable, '-c', 'import time;time.sleep(60)'], timeout=.1)
            started = time.monotonic()
            self.assertFalse(gate.run_checks([check], root, dict(os.environ)))
            self.assertLess(time.monotonic()-started, 5)
            self.assertEqual(check.status, 'FAIL')
            for attempt in ('first', 'retry'):
                self.assertIn('GATE-HUNG-STEP: daemon/parked', (root / 'daemon/parked' / attempt / 'output.log').read_text())

    def test_missing_current_artifact_never_selects_old_binary(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary); old = root / 'target/debug/deps/fbittorrent-old'
            old.parent.mkdir(parents=True); old.write_text('old'); old.chmod(0o700)
            log = root / 'build.log'; log.write_text('$ cargo build --locked\ntransport omitted artifact identity\n')
            with patch.object(gate, 'ROOT', root), self.assertRaises(RuntimeError): gate.artifacts(log, test=False)

    def test_optional_commands_dispatch_and_missing_script_fails(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary); plugins = root / 'plugin/tests'; plugins.mkdir(parents=True)
            for name in ('qs-store.sh', 'qs-smoke.sh', 'qs-dashboard.sh', 'qs-visual.sh'):
                (plugins / name).write_text('echo ' + name + ' >> dispatch.log\n')
            env = {key:value for key,value in os.environ.items() if key not in {'FBT_TEST_'+flag for flag in gate.FLAGS}}
            env.update(FBT_TEST_QS='1', FBT_TEST_VISUAL='1')
            with patch.object(gate, 'ROOT', root):
                checks = gate.optional_checks(env, Path('/missing/binary'))
                self.assertEqual(next(c for c in checks if c.name == 'visual').command[-3:], ['--milestone', '--binary', '/missing/binary'])
                self.assertEqual(next(c for c in checks if c.name == 'qs_palette').command[2:4], ['--palette', '/missing/binary'])
                self.assertTrue(gate.run_checks(checks, root / 'first', env))
                self.assertTrue({'qs-store.sh', 'qs-smoke.sh', 'qs-dashboard.sh', 'qs-visual.sh'} <= set((root / 'dispatch.log').read_text().splitlines()))
                (plugins / 'qs-store.sh').unlink()
                checks = gate.optional_checks(env, Path('/missing/binary'))
                self.assertFalse(gate.run_checks(checks, root / 'missing', env))
                self.assertEqual(next(c for c in checks if c.name == 'qs_store').status, 'FAIL')
                self.assertTrue((root / 'missing/qs_store/retry/output.log').is_file())

    def test_suite_timing_is_information(self):
        registry = gate.modules()
        import daemon_runner
        import fbtlog
        old_root, old_suites = fbtlog.LOG_ROOT, fbtlog._suites
        try:
            with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
                fbtlog.LOG_ROOT = Path(temporary); fbtlog._suites = {}
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    code = daemon_runner.run(Path('/unused/binary'), ['fixture_payloads'], False, 1, budget=0)
                self.assertEqual(code, 0, output.getvalue())
                self.assertIn('INFO daemon timing:', output.getvalue())
                self.assertFalse((Path(temporary) / 'daemon/suite_wall_budget.jsonl').exists())
                rows = [json.loads(line) for line in (Path(temporary) / 'daemon/fixture_payloads.jsonl').read_text().splitlines()]
                self.assertTrue(rows[-1]['final'] and rows[-1]['ok'])
        finally:
            fbtlog.LOG_ROOT, fbtlog._suites = old_root, old_suites

    def test_unit_runner_uses_private_home_and_cleans_it(self):
        program = "import os,json,pathlib;pathlib.Path(os.environ['HOME'],'owned').touch();print(json.dumps({k:v for k,v in os.environ.items() if k.startswith('XDG_') or k in ['HOME','FBITTORRENT_UNIT','WAYLAND_DISPLAY']}))"
        result = subprocess.run([sys.executable, '-B', str(gate.ROOT / 'scripts/gate.py'), '--unit',
                                 sys.executable, '-c', program], capture_output=True, text=True, check=True)
        env = json.loads(result.stdout)
        home = Path(env['HOME'])
        self.assertNotEqual(home, Path.home())
        self.assertFalse(home.parent.exists())
        for key in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            self.assertEqual(Path(env[key]).parent, home.parent)
        self.assertNotIn('WAYLAND_DISPLAY', env)
        self.assertTrue(env['FBITTORRENT_UNIT'].startswith('fbt-fixture-'))

    def test_full_optins_and_default_coverage(self):
        registry = gate.modules()
        checks = gate.cases('full', Path('/missing/binary'), {}, registry)
        names = {c.name for c in checks}
        self.assertTrue({'daemon/' + name for name in registry['daemon'].CASES
                         if name not in ('install_atomic', 'deferred_add')} <= names)
        self.assertTrue({'agent_contract/' + name for name in registry['agent_contract'].CASES} <= names)
        self.assertTrue({'native/' + name for name in registry['native'].CASES + registry['native'].ATTACH_CASES} <= names)
        self.assertTrue({'tui_smoke/' + name for name in registry['tui_smoke'].CASES} <= names)
        self.assertTrue({'tui_journey/' + name for name in registry['tui_journey'].SOURCES} <= names)
        self.assertEqual(gate.optional_checks({}, Path('/missing/binary')), [])
        enabled = {key: '1' for key in ('FBT_TEST_QS', 'FBT_TEST_VISUAL', 'FBT_TEST_WIDGET', 'FBT_TEST_VPN')}
        chosen = gate.optional_checks(enabled, Path('/missing/binary'))
        self.assertTrue({'qs_store', 'qs_smoke', 'qs_palette', 'visual', 'widget', 'vpn'} <= {c.name for c in chosen})
        self.assertFalse(gate.optional_checks({'FBT_TEST_QS': '0'}, Path('/missing/binary')))
        self.assertFalse(gate.optional_checks({'FBT_TEST_LEGACY': '1'}, Path('/missing/binary')), 'removed backends cannot be reinstated by an old opt-in')
        self.assertTrue(any(c.name == 'tui_controls' for c in gate.full_checks({}, Path('/missing/binary'))))
        resources = gate.resource_checks({'FBT_TEST_RESOURCES': '1', 'FBT_TEST_DAEMON_RESOURCES': '1'})
        self.assertEqual(sum(c.name == 'performance_build' for c in resources), 1)
        self.assertEqual([c.name for c in resources], ['performance_build', 'daemon_budget'])
        canonical = gate.resource_checks({'FBT_TEST_RESOURCES': '1'})
        self.assertEqual([c.command for c in canonical], [c.command for c in resources])
        for value in ('--daemon', '--matrix', '--baseline'):
            self.assertIn(value, canonical[-1].command)
        self.assertEqual(canonical[-1].command[canonical[-1].command.index('--runs')+1], '3')
        self.assertEqual(canonical[-1].command[canonical[-1].command.index('--seconds')+1], '60')
        daemon_only = gate.resource_checks({'FBT_TEST_DAEMON_RESOURCES': '1'})
        self.assertTrue(any('--daemon' in c.command for c in daemon_only))
        self.assertFalse(gate.resource_checks({}))

    def test_all_alias_help_does_not_start_build(self):
        result = subprocess.run(['bash', str(gate.HARNESS / 'all.sh'), '--help'], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        for flag in (*gate.FLAGS, 'VERBOSE', 'FORCE_FAIL', 'SWAY'):
            self.assertIn('FBT_TEST_' + flag, result.stdout)

    def test_build_abort_never_uses_stale_success(self):
        with tempfile.TemporaryDirectory(prefix='fbt-gate-control-') as temporary:
            root = Path(temporary); scripts = root / 'scripts'; scripts.mkdir()
            for name in ('gate.py', 'gate_safety.py', 'gate-fast.sh'): shutil.copy2(gate.ROOT / 'scripts' / name, scripts / name)
            harness = root / 'agent_ergonomics_audit/audit/regression_tests'; harness.mkdir(parents=True)
            for name in ('daemon', 'native', 'agent_contract', 'tui_smoke', 'tui_journey'):
                (harness / (name + '.py')).write_text('SYSTEMD_CASES=set()\n')
            home = root / 'home'; shims = home / '.rch/shims'; shims.mkdir(parents=True)
            binary = root / 'target/debug/test-binary'; binary.parent.mkdir(parents=True); binary.write_text('never execute'); binary.chmod(0o700)
            artifact = dict(reason='compiler-artifact', target=dict(name='fbittorrent'), profile=dict(test=True), executable=str(binary))
            cargo = shims / 'cargo'
            cargo.write_text('#!/bin/sh\necho "$1" >>"$HOME/calls"\nif [ "$1" = test ]; then\nprintf \'%s\\n\' \'' + json.dumps(artifact) + '\'\necho "test result: ok. 1 passed; 0 failed;"\nexit 0\nfi\necho planted-build-failure\nexit 137\n')
            cargo.chmod(0o700)
            rustc = shims / 'rustc'; rustc.write_text("#!/bin/sh\necho 'host: x86_64-unknown-linux-gnu'\n"); rustc.chmod(0o700)
            stale = root / 'target/test-logs/native/old.jsonl'; stale.parent.mkdir(parents=True)
            stale.write_text('{"final":true,"ok":true}\n')
            archive = root / 'target/test-logs/archived/keep.jsonl'; archive.parent.mkdir(); archive.write_text('prior evidence\n')
            result = subprocess.run(['bash', str(scripts / 'gate-fast.sh')], env=dict(os.environ, HOME=str(home)),
                                    capture_output=True, text=True, timeout=15)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('planted-build-failure', next((root / 'target').glob('gate-fast-*/build/first/output.log')).read_text())
            self.assertEqual((home / 'calls').read_text().splitlines(), ['test', 'build', 'build'])
            summary = json.loads(next((root / 'target').glob('gate-fast-*/summary.json')).read_text())
            native = [c for c in summary['checks'] if c['name'].startswith('native/')]
            self.assertTrue(native and all(c['status'] == 'NOT RUN' for c in native))
            self.assertFalse(summary['ok'])
            self.assertEqual(stale.read_text(), '{"final":true,"ok":true}\n')
            self.assertEqual(archive.read_text(), 'prior evidence\n')
            self.assertNotIn('old.jsonl', result.stdout)


def run_named(*names):
    """Existing ci_test/fbtlog_test retain their controls through this runner."""
    output = io.StringIO()
    result = unittest.TextTestRunner(stream=output).run(unittest.TestSuite(GateTests(name) for name in names))
    return result.wasSuccessful(), output.getvalue()


if __name__ == '__main__': unittest.main()
