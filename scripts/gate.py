#!/usr/bin/env python3
"""Command orchestration only: all app checks live in the existing harnesses.

Each failed case gets one serial retry in a fresh process/sandbox/log directory.
Both attempts remain available. Timing retry successes are FLAKY; safety failures
remain FAIL, and an explicit opt-out is SKIP rather than passed coverage.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
import importlib
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from gate_safety import strict_check, strict_rust_test

ROOT = Path(__file__).resolve().parents[1]
HARNESS = ROOT / 'agent_ergonomics_audit/audit/regression_tests'
PYTHON = sys.executable
RELEASE_LOCK = '/tmp/fbittorrent-release.lock'
WORKER_FAILURE = re.compile(r'thread .*panicked|obligation.*leak|closing native (?:torrent session|runtime):|native torrent session did not drain')
# These are existing cases, not a second implementation of their assertions.
# q_detaches includes a real loopback download that keeps moving after detach.
FAST = {
    'native': ['q_detaches', 'q_confirms'],
    'daemon': ['zero_upload', 'api_torrent_operations', 'mutation_magnets_and_file_removal', 'daemon_api_probe',
               'add_starts_unit_no_launcher', 'down_stops',
               'idle_exit_bar_does_not', 'dashboard_bar_presence',
               'idle_exit_disabled_shutdown_drains', 'search_dedupe_and_repair',
               'rss_cli_edit_visible', 'shows_inbox_semantics', 'setup_idempotent'],
    'agent_contract': ['reads_never_start'],
}
FLAGS = ('QS', 'VISUAL', 'WIDGET', 'VPN', 'RESOURCES', 'DAEMON_RESOURCES')


def release_scope(command):
    # Reuse the documented outer flock when all.sh inherits it from its parent.
    held = False
    for path in Path('/proc/self/fd').iterdir():
        try:
            if os.readlink(path) == RELEASE_LOCK:
                fcntl.flock(int(path.name), fcntl.LOCK_EX | fcntl.LOCK_NB)
                held = True
        except (OSError, ValueError): pass
    lock = [] if held else ['flock', '-w', '7200', RELEASE_LOCK]
    return [*lock, 'systemd-run', '--user', '--scope', '-p', 'MemoryHigh=10G', '-p', 'MemoryMax=14G',
            '--quiet', '--', 'env', 'FBT_GATE_RELEASE_SCOPE=1', *command]


@dataclass
class Check:
    name: str
    command: list
    case: bool = False
    status: str = 'NOT RUN'
    seconds: float = 0
    log: str = ''
    timeout: float = 900
    detail: str = ''


def bounded_run(command, env, output, timeout, grace=10):
    """Bound the owned process group, including a hung compiler or harness."""
    child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=output,
                             stderr=subprocess.STDOUT, start_new_session=True)
    def stop():
        for sig, seconds in ((signal.SIGINT, grace), (signal.SIGTERM, 2), (signal.SIGKILL, 2)):
            try: os.killpg(child.pid, sig)
            except ProcessLookupError: pass
            try: child.wait(timeout=seconds); break
            except subprocess.TimeoutExpired: pass
        # A parent may exit on SIGINT while its child ignores it.
        try: os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError: pass
    try:
        return child.wait(timeout=timeout), False
    except subprocess.TimeoutExpired:
        stop()
        return 124, True
    except BaseException:
        stop()
        raise


def unit_environment(command, rust_test=False):
    """Cargo runner: compilation keeps its toolchain HOME; test execution doesn't."""
    with tempfile.TemporaryDirectory(prefix='fbt-unit-', dir='/tmp') as directory:
        root = Path(directory)
        env = dict(os.environ, TMPDIR='/tmp')  # short Unix socket paths
        for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME',
                    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            path = root / key.lower(); path.mkdir(mode=0o700); env[key] = str(path)
        env['FBITTORRENT_UNIT'] = 'fbt-fixture-' + root.name + '.service'
        env['DBUS_SESSION_BUS_ADDRESS'] = 'unix:path=' + str(root / 'absent-bus')
        shims = root / 'shims'; shims.mkdir()
        for name in ('systemctl', 'xdg-open', 'notify-send', 'omarchy-launch-or-focus-tui', 'omarchy-restart-shell'):
            path = shims / name; path.write_text('#!/bin/sh\nexit 99\n'); path.chmod(0o700)
        env['PATH'] = str(shims) + os.pathsep + env.get('PATH', '')
        for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK', 'HYPRLAND_INSTANCE_SIGNATURE'):
            env.pop(key, None)
        names = set()
        if rust_test:
            listing = root / 'tests.list'
            for flags in (['--list'], ['--list', '--ignored']):
                with listing.open('w') as out:
                    code, _ = bounded_run([*command, *flags], env, out, 10, grace=0)
                if code:
                    print('Rust test discovery failed: ' + str(command[0]), flush=True)
                    print(listing.read_text(), end='')
                    return code
                listed = set(re.findall(r'^(\S+): test$', listing.read_text(), re.M))
                if '--ignored' in flags: names -= listed
                else: names = listed
        limit = float(env.get('FBT_GATE_RUST_TIMEOUT_SECS' if rust_test else 'FBT_GATE_CASE_TIMEOUT_SECS',
                              '120' if rust_test else '180'))
        if not 0 < limit < float('inf'): raise ValueError('hang guard must be finite and positive')
        output = root / 'output.log'
        with output.open('w') as out:
            code, hung = bounded_run(command, env, out, limit, grace=0 if rust_test else 10)
        body = output.read_text(errors='replace')
        print(body, end='', flush=True)
        if hung:
            finished = set(re.findall(r'\btest (\S+) \.\.\. (?:ok|FAILED|ignored)\b', body))
            active = set(re.findall(r'test (\S+) has been running for over \d+ seconds', body))
            unfinished = (names - finished) | (active & names)
            print(f'GATE-HUNG-BINARY: {command[0]} exceeded {limit:g}s', flush=True)
            for name in sorted(active or unfinished): print('GATE-HUNG-TEST: ' + name, flush=True)
            # Include queued tests as well: killing a binary must not silently
            # drop coverage just because only a few workers were stuck.
            for name in sorted(unfinished): print('GATE-UNFINISHED-TEST: ' + name, flush=True)
        return code


def modules():
    sys.path.insert(0, str(HARNESS))
    return {name: importlib.import_module(name) for name in ('daemon', 'native', 'agent_contract', 'tui_smoke', 'tui_journey')}


def opted_in(env, name):
    return env.get('FBT_TEST_' + name) == '1'


def headless_sway(env):
    found = env.get('FBT_TEST_SWAY') or shutil.which('sway')
    if found: return found
    # The extracted test-only compositor can be shared across worktrees; it
    # still starts with each fixture's private display, runtime and libraries.
    trees = subprocess.check_output(['git', 'worktree', 'list', '--porcelain'], cwd=ROOT, text=True, timeout=10)
    roots = [ROOT, *(Path(line[9:]) for line in trees.splitlines() if line.startswith('worktree '))]
    for root in roots:
        binary = root / 'plugin/tests/target/headless-tools/usr/bin/sway'
        if binary.is_file() and os.access(binary, os.X_OK): return str(binary)


def cases(mode, binary, env, registry):
    selected = FAST if mode == 'fast' else {
        'native': list(registry['native'].CASES + registry['native'].ATTACH_CASES),
        'daemon': [name for name in registry['daemon'].CASES
                   if (name != 'install_atomic' or opted_in(env, 'INSTALL'))
                   and (name != 'deferred_add' or opted_in(env, 'VPN'))],
        'agent_contract': list(registry['agent_contract'].CASES),
        'tui_smoke': list(registry['tui_smoke'].CASES),
        'tui_journey': list(registry['tui_journey'].SOURCES),
    }
    result = []
    for suite, names in selected.items():
        for name in names:
            command = [PYTHON, '-B', str(HARNESS / (suite + '.py')), str(binary), '--case', name]
            if suite == 'daemon':
                command += ['--jobs', '1', '--systemd' if name in registry['daemon'].SYSTEMD_CASES else '--no-systemd']
            timeout = 720 if name == 'rev_soak_10min' and opted_in(env, 'SOAK') else float(env.get('FBT_GATE_CASE_TIMEOUT_SECS', '180'))
            result.append(Check(suite + '/' + name, command, case=True, timeout=timeout))
    # These drive the actual plugin client on a private headless compositor.
    result.append(python_check('plugin_stop_rule','plugin/tests/stop_policy.py',
                              '--binary',str(binary),'--case','power','--log-root','{logs}'))
    if mode == 'full':
        result += [python_check('plugin_glance_policy','plugin/tests/stop_policy.py',
                                '--binary',str(binary),'--case','glance','--log-root','{logs}'),
                   python_check('plugin_default_idle','plugin/tests/stop_policy.py',
                                '--binary',str(binary),'--case','default_delay','--log-root','{logs}')]
    return result


def optional_checks(env, binary):
    result = []
    for flag, name, command in [
        ('QS', 'qs_store', ['bash', 'plugin/tests/qs-store.sh']),
        ('QS', 'qs_smoke', ['bash', 'plugin/tests/qs-smoke.sh']),
        ('QS', 'qs_palette', ['bash', 'plugin/tests/qs-dashboard.sh', '--palette', str(binary), '--log-root', '{logs}']),
        ('VISUAL', 'visual', ['bash', 'plugin/tests/qs-visual.sh', '--milestone', '--binary', str(binary)]),
        ('WIDGET', 'widget', [PYTHON, str(Path(__file__).resolve()), '--unit', 'bash', str(HARNESS / 'widget_keys.sh')]),
        ('VPN', 'vpn', [PYTHON, str(HARNESS / 'ci.py'), '--suite', 'gate_vpn', '--isolate-state', '--',
                       PYTHON, str(HARNESS / 'vpn.py'), str(binary)]),
    ]:
        if opted_in(env, flag): result.append(Check(name, command))
    return result


def python_check(name, path, *args):
    # Override fbtlog in this process only; nested fixture children retain their
    # own roots. No global log reset can erase another gate's evidence.
    bootstrap = ('import sys,runpy;from pathlib import Path;'
                 'sys.path.insert(0,sys.argv[1]);import fbtlog;'
                 'fbtlog.LOG_ROOT=Path(sys.argv[2]);sys.argv=sys.argv[3:];'
                 'sys.path.insert(0,str(Path(sys.argv[0]).parent));'
                 'runpy.run_path(sys.argv[0],run_name="__main__")')
    return Check(name, [PYTHON, '-B', '-c', bootstrap, str(HARNESS), '{logs}', str(ROOT / path), *args])


def run_once(check, logs, env, attempt):
    directory = logs / check.name / ('first' if attempt == 1 else 'retry')
    directory.mkdir(parents=True)
    command = [str(part).replace('{logs}', str(directory)) for part in check.command]
    if check.case: command += ['--log-root', str(directory)]
    log = directory / 'output.log'
    started = time.monotonic()
    try:
        with log.open('w') as output:
            output.write('$ ' + ' '.join(command) + '\n'); output.flush()
            timeout = float(env.get('FBT_GATE_STEP_TIMEOUT_SECS', str(check.timeout)))
            if not 0 < timeout < float('inf'): raise ValueError('hang guard must be finite and positive')
            code, hung = bounded_run(command, env, output, timeout, grace=30 if check.case else 10)
            if hung:
                message = f'GATE-HUNG-STEP: {check.name} exceeded {timeout:g}s; owned process group stopped'
                output.write(message + '\n'); print(message, flush=True)
            output.write(f'\nGATE-EXIT: {code}\n')
    except OSError as error:
        log.write_text(str(error)); code = 127
    check.seconds += time.monotonic() - started
    check.log = str(log)
    ok = code == 0
    skipped = False
    if check.case:
        # A harness crash or missing final is a failure even if a wrapper exits 0.
        finals = []
        try:
            for path in directory.glob('*/*.jsonl'):
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                if not rows or not isinstance(rows[0], dict) or 'suite' not in rows[0]: continue
                ok &= bool(rows[-1].get('final')) and not any(row.get('ok') is False for row in rows)
                finals += [row for row in rows if row.get('final')]
        except (ValueError, OSError, AttributeError): ok = False
        ok &= bool(finals)
        skipped = bool(finals) and all(row.get('status') == 'skip' for row in finals)
    if ok and skipped:
        check.status = 'FAIL' if strict_check(check.name) else 'SKIP'
        check.detail = 'coverage skipped; no passed case' if not strict_check(check.name) else 'safety coverage was not executed'
        return check.status == 'SKIP'
    check.status = ('PASS' if attempt == 1 else 'FAIL' if strict_check(check.name) else 'FLAKY') if ok else 'FAIL'
    return ok


def run_checks(checks, logs, env, jobs=1):
    # Drain the complete first pass before retrying. A retry never competes with
    # another gate case and cannot reuse its process or temporary XDG directory.
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        list(pool.map(lambda check: run_once(check, logs, env, 1), checks))
    for check in checks:
        if check.status == 'FAIL':
            print('Retry in isolation: ' + check.name, flush=True)
            run_once(check, logs, env, 2)
    return not any(check.status == 'FAIL' for check in checks)


def artifacts(log, test):
    paths = []
    for line in log.read_text().splitlines():
        try: item = json.loads(line)
        except ValueError: continue
        if not isinstance(item, dict): continue
        if item.get('reason') != 'compiler-artifact' or item.get('target', {}).get('name') != 'fbittorrent': continue
        if item.get('profile', {}).get('test') != test or not item.get('executable'): continue
        paths.append(Path(item['executable']))
    command = log.read_text().splitlines()[0]
    named_profile = re.search(r'--profile\s+(\S+)', command)
    profile = named_profile[1] if named_profile else 'release' if not test and '--release' in command else 'debug'
    for path in reversed(paths):
        for candidate in (path, ROOT / 'target' / profile / path.name, ROOT / 'target' / profile / 'deps' / path.name):
            if candidate.is_file() and os.access(candidate, os.X_OK): return candidate
    # Never select the newest old binary after a transport/cache failure. Only
    # an artifact identified by this Cargo invocation may run workflow cases.
    raise RuntimeError('Cargo invocation did not identify a delivered executable; see ' + str(log))


def rust_tests(mode, logs, env):
    host = subprocess.check_output(['rustc', '-vV'], env=env, text=True, timeout=15).split('host: ', 1)[1].splitlines()[0]
    runner = json.dumps([PYTHON, str(Path(__file__).resolve()), '--rust-test'])
    command = ['cargo', 'test', '--locked', '--no-fail-fast', '--message-format=json',
               '--config', f'target.{host}.runner={runner}']
    if mode == 'full': command += ['-p', 'fbittorrent', '-p', 'rqbit-runtime']
    command += ['--', '--nocapture']
    check = Check('rust_tests', command)
    ok = run_once(check, logs, env, 1)
    body = Path(check.log).read_text()
    ok &= bool(re.search(r'^test result: (?:ok|FAILED)\. [1-9][0-9]* (?:passed|failed)', body, re.M))
    failures = set(re.findall(r'^test (\S+) \.\.\. FAILED$', body, re.M))
    unfinished = set(re.findall(r'^GATE-UNFINISHED-TEST: (\S+)$', body, re.M))
    failures.update(unfinished)
    for name in re.findall(r'^GATE-HUNG-TEST: (\S+)$', body, re.M): print('HUNG: ' + name, flush=True)
    # libtest's parallel output can interleave a test name with diagnostics.
    # Its final failure list still identifies the cases unambiguously.
    for result in re.finditer(r'^test result: FAILED\.', body, re.M):
        start = body.rfind('\nfailures:\n', 0, result.start())
        if start >= 0:
            failures.update(name for line in body[start+11:result.start()].splitlines()
                            if re.fullmatch(r'[A-Za-z_][A-Za-z_0-9:]*', name := line.strip()))
    failures = sorted(failures)
    check.detail = ', '.join(failures)
    worker_failure = bool(WORKER_FAILURE.search(body))
    if not ok and failures:
        retries = []
        for name in failures:
            # Both packages are searched in full mode; demand a real executed
            # test below so a typo/renamed test can never pass as "0 tests".
            retry_command = command[:-2] + [name, '--', '--exact', '--nocapture', '--test-threads=1']
            retry = Check('rust_retry/' + name.replace('::', '_'), retry_command)
            passed = run_once(retry, logs, env, 2)
            passed &= bool(re.search(r'^test result: ok\. [1-9][0-9]* passed;', Path(retry.log).read_text(), re.M))
            passed &= not WORKER_FAILURE.search(Path(retry.log).read_text())
            retries.append(passed)
            check.seconds += retry.seconds
        # A failing named test explains its own panic. Unattributed worker
        # panics on an otherwise-green suite must never be called a pass.
        safety_failed = any(strict_rust_test(name) for name in failures)
        check.status = 'FLAKY' if not safety_failed and all(retries) and ('test result: FAILED.' in body or unfinished) else 'FAIL'
        if safety_failed: check.detail += '; safety failure retained despite retry'
    elif not ok or worker_failure:
        # Compiler/transport failure or an un-attributed background panic has
        # no narrower case to replay. Retry this stage once, with serial tests.
        check.command += ['--test-threads=1']
        passed = run_once(check, logs, env, 2)
        retried = Path(check.log).read_text()
        passed &= bool(re.search(r'^test result: ok\. [1-9][0-9]* passed;', retried, re.M))
        check.status = 'FLAKY' if passed and not worker_failure and not WORKER_FAILURE.search(retried) else 'FAIL'
    return check


def full_checks(env, binary):
    return [python_check('resource_controls', 'agent_ergonomics_audit/audit/regression_tests/resources_test.py'),
            python_check('logging', 'agent_ergonomics_audit/audit/regression_tests/fbtlog_test.py'),
            python_check('reality_controls','agent_ergonomics_audit/audit/regression_tests/test_reality_controls.py'),
            python_check('owner_facts_controls','agent_ergonomics_audit/audit/regression_tests/owner_torrent_facts_test.py'),
            python_check('qml_logging', 'plugin/tests/qs-log.py', '--self-test'),
            Check('hygiene', ['bash', 'scripts/check-gitignore.sh']),
            Check('licences', ['bash', 'scripts/check-nova3-licences.sh']),
            Check('widget_model', ['bash', 'plugin/tests/lint.sh']),
            Check('tracker_text', ['node','--test','plugin/tests/tracker-text.test.js']),
            python_check('tracker_privacy','plugin/tests/tracker_privacy.py','--binary',str(binary),'--log-root','{logs}'),
            python_check('widget_online', 'scripts/check-widget-online.py', '--self-test'),
            python_check('tui_controls', 'agent_ergonomics_audit/audit/regression_tests/tui_smoke_test.py', str(binary)),
            python_check('ci_gates', 'agent_ergonomics_audit/audit/regression_tests/ci_test.py'),
            python_check('gate_controls', 'scripts/gate_test.py'),
            python_check('safety_controls', 'scripts/gate_safety_test.py'),
            python_check('upload_probe_controls', 'agent_ergonomics_audit/audit/regression_tests/upload_wire_test.py'),
            *optional_checks(env, binary)]


def resource_checks(env):
    # The TUI now attaches to a daemon and q detaches. Measure the current
    # ownership model against P1-08's recorded daemon workloads instead of
    # invoking P0-06's embedded-engine TUI start/quit benchmark.
    if not any(opted_in(env, flag) for flag in ('RESOURCES', 'DAEMON_RESOURCES')):
        return []
    check = python_check('daemon_budget', str(HARNESS.relative_to(ROOT) / 'resources.py'),
        '{perf_binary}', '--daemon', '--matrix', '--runs', '3', '--seconds', '60',
        '--baseline', str(HARNESS / 'baselines/resources.json'), '--output', '{logs}/measurement.json')
    check.timeout = 2100
    return [Check('performance_build', ['cargo', 'build', '--profile', 'release-perf', '--locked',
                                      '--message-format=json'], timeout=1200), check]


def summary(checks, elapsed, logs, mode):
    groups = {}
    for check in checks:
        if check.status == 'FLAKY' and strict_check(check.name): check.status = 'FAIL'
        group = check.name.split('/')[0]
        if group in ('logging', 'qml_logging', 'ci_gates', 'gate_controls', 'widget_online'): group = 'gate_controls'
        if group in ('hygiene', 'licences', 'widget_model'): group = 'hygiene'
        if group == 'tui_controls': group = 'tui_smoke'
        if group == 'upgrade_fixtures': group = 'daemon'
        if group in ('qs_store', 'qs_smoke', 'qs_palette', 'visual', 'widget', 'vpn'): group = 'optional'
        counts = groups.setdefault(group, dict(PASS=0, FLAKY=0, FAIL=0, SKIP=0, **{'NOT RUN': 0}))
        counts[check.status] += 1
    print('\nGate          PASS  FLAKY  FAIL  SKIP  NOT RUN')
    for name, counts in groups.items():
        print(f'{name:13} {counts["PASS"]:4}  {counts["FLAKY"]:5}  {counts["FAIL"]:4}  {counts["SKIP"]:4}  {counts["NOT RUN"]:7}')
    problems = [check for check in checks if check.status in ('FAIL', 'FLAKY')]
    for check in problems[:6]:
        detail = check.detail if len(check.detail) <= 180 else check.detail[:180] + '... (see logs)'
        print(f'{check.status}: {check.name}' + (': ' + detail if detail else ''))
    if len(problems) > 6: print(f'... {len(problems)-6} more; see summary.json in the log directory')
    failed = any(check.status in ('FAIL', 'NOT RUN') for check in checks)
    print(f'Wall: {elapsed:.1f}s' + (' (warm-cache target: <=300s; timing is informational)' if mode == 'fast' else ''))
    print('Logs: ' + str(logs))
    print('Gate: ' + ('FAIL' if failed else 'PASS with FLAKY cases' if any(c.status == 'FLAKY' for c in checks) else 'PASS'))
    (logs / 'summary.json').write_text(json.dumps(dict(mode=mode, seconds=elapsed,
        ok=not failed, checks=[vars(check) for check in checks]), indent=2) + '\n')
    return int(failed)


def main():
    parser = argparse.ArgumentParser(description=__doc__, epilog='Full opt-ins: ' + ', '.join('FBT_TEST_' + f + '=1' for f in FLAGS) +
        '. FBT_TEST_SWAY selects private headless Sway; FBT_TEST_VERBOSE=1 mirrors logs; FBT_TEST_FORCE_FAIL=suite/case plants a failure. '
        'Hang guards: FBT_GATE_RUST_TIMEOUT_SECS=120 per test binary, FBT_GATE_CASE_TIMEOUT_SECS=180 per workflow; '
        'FBT_GATE_STEP_TIMEOUT_SECS overrides other stage deadlines. Hung tests retry by exact name, serially.')
    parser.add_argument('mode', choices=['fast', 'full'])
    parser.add_argument('--jobs', type=int, default=2, choices=range(1, 5))
    parser.add_argument('--list', action='store_true', help='show selected workflow cases without building or starting anything')
    args = parser.parse_args()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1')
    if args.mode == 'full': env.setdefault('RCH_BUILD_TIMEOUT_SEC', '1200')
    if args.mode == 'full' and not args.list and env.get('FBT_GATE_RELEASE_SCOPE') != '1':
        return subprocess.call(release_scope([PYTHON, '-B', str(Path(__file__).resolve()), *sys.argv[1:]]), env=env)
    registry = modules()
    if args.list:
        for check in cases(args.mode, Path('target/debug/fbittorrent'), env, registry): print(check.name)
        return 0
    (ROOT / 'target').mkdir(exist_ok=True)
    logs = Path(tempfile.mkdtemp(prefix='gate-' + args.mode + '-', dir=ROOT / 'target'))
    started = time.monotonic()
    checks = [Check('rust_tests', []), Check('build', [])]
    checks += cases(args.mode, Path('target/debug/fbittorrent'), env, registry)
    try:
        print('Running debug Rust tests (rch); logs: ' + str(logs), flush=True)
        rust = rust_tests(args.mode, logs, env); checks[0] = rust
        # A test failure remains red, but need not hide unrelated workflow
        # results. A compile/delivery failure cannot supply a validator and
        # stops below with all workflow cases still marked NOT RUN.
        validator = artifacts(Path(rust.log), test=True)
        env['FBT_API_CONTRACT_VALIDATOR'] = str(validator)
        release = args.mode == 'full'
        build = Check('build', ['cargo', 'build', '--locked', '--message-format=json'] + (['--release'] if release else []), timeout=1200 if release else 900)
        print('Building ' + ('release' if release else 'debug') + ' workflow binary (rch)', flush=True)
        checks[1] = build
        if not run_checks([build], logs, env): return summary(checks, time.monotonic()-started, logs, args.mode)
        binary = artifacts(Path(build.log), test=False)
        env['FBT_BINARY'] = env['FBT_BIN'] = str(binary)
        sway = headless_sway(env)
        if sway: env['FBT_TEST_SWAY'] = sway
        selected = cases(args.mode, binary, env, registry)
        checks[2:] = selected
        if args.mode == 'full':
            # Same preparation as daemon.py: concurrent upgrade cases must not
            # rebuild an executable while another case is launching it.
            prepare = Check('upgrade_fixtures', [PYTHON, '-B', str(HARNESS / 'daemon.py'), str(binary),
                            '--prepare-upgrade-fixtures', '--log-root', '{logs}'], timeout=1800)
            checks.append(prepare)
            if not run_checks([prepare], logs, env): return summary(checks, time.monotonic()-started, logs, args.mode)
        print(f'Running {len(selected)} isolated workflow cases ({args.jobs} workers)', flush=True)
        run_checks(selected, logs, env, args.jobs)
        if args.mode == 'full':
            other = full_checks(env, binary)
            checks += other; run_checks(other, logs, env)
            resources = resource_checks(env); checks += resources
            if resources and run_checks(resources[:1], logs, env):
                perf_binary = artifacts(Path(resources[0].log), test=False)
                for check in resources[1:]: check.command = [part.replace('{perf_binary}', str(perf_binary)) for part in check.command]
                run_checks(resources[1:], logs, env)
    except Exception as error:
        print('Gate setup failed: ' + str(error), file=sys.stderr)
        checks.append(Check('gate_setup', [], status='FAIL'))
    return summary(checks, time.monotonic()-started, logs, args.mode)


if __name__ == '__main__':
    if sys.argv[1:2] == ['--unit']: sys.exit(unit_environment(sys.argv[2:]))
    if sys.argv[1:2] == ['--rust-test']: sys.exit(unit_environment(sys.argv[2:], rust_test=True))
    sys.exit(main())
