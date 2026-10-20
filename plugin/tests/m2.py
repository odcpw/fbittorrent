#!/usr/bin/env python3
"""Run the M2 behaviour matrix serially with private fixtures and shells.

No builds, installed shell reloads, or owner-data operations. Each existing test
keeps its own assertions; this runner preserves its output and generated evidence.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'agent_ergonomics_audit/audit/regression_tests'))


def matrix(binary):
    python = sys.executable
    dashboard = ['bash', 'plugin/tests/qs-dashboard.sh']
    p2 = [('search_flow' if name == 'search' else name, ['bash', 'plugin/tests/qs-' + name + '.sh'])
          for name in ('client', 'store', 'glyphs', 'format', 'icons', 'search', 'smoke')]
    p2 += [('contrast', [python, 'plugin/tests/contrast.py', '--self-test']),
           ('theme_lint', ['bash', 'plugin/tests/lint.sh']),
           ('service', [python, 'plugin/tests/service-tests.py', binary]),
           ('popup', [python, 'plugin/tests/popup.py', '--binary', binary])]
    p2 += [('qml_' + name, [python, 'plugin/tests/run-qs.py', name.replace('-', '_'), 'plugin/tests/' + name])
           for name in ('popup-state', 'table', 'add-flow', 'clipboard-flow', 'details-flow',
                        'folder-picker', 'settings-flow', 'shows-flow', 'shows-quality',
                        'rss-flow', 'cleanup-flow', 'pieces', 'piece-completion', 'review-rows', 'row-refresh', 'stop-confirm')]
    ui = [('window', [*dashboard, '--binary', binary])]
    ui += [(name, [*dashboard, '--' + name, '--binary', binary])
           for name in ('add-sheet', 'destination-picker', 'details', 'search',
                        'preferences', 'grid', 'pieces', 'rss', 'recheck', 'journeys')]
    ui += [('owner_' + case, [*dashboard, '--owner-review', '--binary', binary, '--case', case])
           for case in ('search', 'rows', 'pieces', 'popup', 'review', 'stop')]
    ui += [('table', [python, 'plugin/tests/dashboard-table.py', '--binary', binary]),
           ('table_fixture', [python, 'plugin/tests/dashboard-table.py', '--binary', binary, '--fixture-cli']),
           ('table_profile', [python, 'plugin/tests/dashboard-table.py', '--binary', binary, '--profile'])]
    ui += [('magnet_' + name, [python, 'plugin/tests/handler_magnet.py', '--binary', binary, *flags])
           for name, flags in [('review', []), ('early', ['--early'])]]
    import daemon
    names = [name for name in daemon.CASES if (
        name.startswith(('search_', 'shows_', 'rss_', 'imdb_', 'settings_', 'staged_', 'cli_search_', 'cli_rss_', 'cli_shows_', 'cli_imdb_', 'cli_catalogue_'))
        or name in ('commit_non_staged_invalid_state', 'handler_confirm_stages', 'tui_open_one_refresh'))]
    # The real user manager creates its child in the host network namespace.
    # Keep that one existing test with its loopback HTTP fixture, disposable
    # named unit and verified private XDG. Other cases get separate namespaces.
    backend = [(name, [python, *(['agent_ergonomics_audit/audit/regression_tests/daemon.py']
                                if name == 'cli_catalogue_grabs' else ['plugin/tests/m2.py', '--daemon-worker']), binary,
                       '--jobs', '1', '--case', name]) for name in names]
    return dict(p2=p2, dashboard=ui, daemon=backend)


def run(name, command, output, timeout):
    destination = output / name
    destination.mkdir()
    began = time.time()
    env = dict(os.environ, FBT_PIECES_CAPTURE=str(destination / 'pieces.png'))
    with (destination / 'output.log').open('w') as log:
        child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                                 start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            # Give test finally blocks a chance to reap their own shell/daemon.
            os.killpg(child.pid, signal.SIGINT)
            try: child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)
            code = 124
    logs = ROOT / 'target/test-logs'
    for suite in logs.iterdir():
        if not suite.is_dir() or not (suite / 'summary.json').is_file():
            continue
        if any(p.is_file() and p.stat().st_mtime >= began for p in suite.rglob('*')):
            shutil.copytree(suite, destination / suite.name)
    result = dict(name=name, command=command, exit=code,
                  seconds=round(time.time() - began, 2), evidence=str(destination))
    print(('PASS ' if code == 0 else 'FAIL ') + name + ' ' + str(result['seconds']) + 's ' + str(destination), flush=True)
    return result


def main():
    if sys.argv[1:2] == ['--daemon-worker']:
        import private_net
        private_net.enter()
        sys.argv.pop(1)
        import daemon
        return daemon.main()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--group', choices=('all', 'p2', 'dashboard', 'daemon'), default='all')
    parser.add_argument('--case', action='append', help='run only these named matrix entries')
    parser.add_argument('--timeout', type=int, default=600)
    args = parser.parse_args()
    os.environ['FBT_BIN'] = str(args.binary.resolve())
    groups = matrix(str(args.binary.resolve()))
    selected = [item for group, items in groups.items() if args.group in ('all', group) for item in items]
    if args.case:
        unknown = set(args.case) - {name for name, _ in selected}
        if unknown: parser.error('unknown cases: ' + ', '.join(sorted(unknown)))
        selected = [item for item in selected if item[0] in args.case]
    output = ROOT / 'target' / ('m2-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    output.mkdir()
    print('M2 evidence: ' + str(output), flush=True)
    results = []
    for name, command in selected:
        results.append(run(name, command, output, args.timeout))
        (output / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')
    return int(any(result['exit'] for result in results))


if __name__ == '__main__':
    sys.exit(main())
