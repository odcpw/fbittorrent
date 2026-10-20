#!/usr/bin/env python3
"""One private daemon, real CLI, attached tmux TUI and native dashboard/popup."""
import argparse
from contextlib import ExitStack, contextmanager
import json
import os
import re
import subprocess
from pathlib import Path
import time

from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status
from tui_smoke import PaneUi


@contextmanager
def evidence(case, q, ui):
    try:
        yield
    except BaseException:
        try:
            case.path.with_suffix('.state.json').write_text(json.dumps(q.state(), indent=2))
            case.path.with_suffix('.failure.pane.txt').write_text(ui.capture(ui.target))
        except Exception as error:
            case.path.with_suffix('.capture-error.txt').write_text(str(error))
        raise


def settings(binary, focused=False, race=False):
    with Case('cross_surface', 'settings_race' if race else 'settings_focused' if focused else 'settings') as case, ExitStack() as stack:
        box = stack.enter_context(fixtures.Sandbox())
        h = stack.enter_context(daemon.Harness(case, binary, box, False))
        process = h.start_serve(idle_exit_secs=0)
        assert h.receive_ready() and h.wait_engine()
        q = Session(case, box, recipe='bar-host')
        stack.callback(q.close)
        keyboard = subprocess.Popen(['wtype', '-s', '600000'], env=q.env)
        stack.callback(lambda: (keyboard.terminate(), keyboard.wait(timeout=3)))
        ui = PaneUi(case, str(binary), box, h, session='fbt-cross-surface-settings')
        stack.callback(ui.close)
        ui.start_owned()
        stack.enter_context(evidence(case, q, ui))
        q.wait(lambda: q.ipc('ready') == 'true')
        q.shell('summon', '{"view":"settings"}')
        q.wait(lambda: ((q.state().get('view') or {}).get('settings') or {}).get('loaded'))
        if race:
            for index in range(4):
                base = 10+index*3
                h.cli('settings', '--max-downloads', str(base))
                q.wait(lambda: q.state()['settings']['max_active_downloads'] == base)
                refs = q.state()['mutationRefs']
                assert q.ipc('applySetting', 'max_active_downloads', str(base+1)) == 'true'
                h.cli('settings', '--max-downloads', str(base+2))
                q.wait(lambda: all(a['status'] in ('confirmed', 'done') for a in q.state()['actions'].values()), timeout=2)
                confirmed = h.cli('settings')['max_active_downloads']
                observed = q.wait(lambda: s if not (s := q.state())['view']['settings']['pending']
                                  and s['settingFields']['max_active_downloads']['text'] == str(confirmed) else None,
                                  timeout=2)
                case.check(observed['mutationRefs'] == refs+1,
                           'rapid same-field dashboard/CLI writes converge without a stuck pending edit or duplicate action',
                           json.dumps(dict(confirmed=confirmed, state=observed)))
            return
        if focused:
            value = h.cli('settings')['max_active_downloads']+1
            q.compositor.ipc('[title="^fbittorrent$"] focus')
            assert q.ipc('focusSetting', 'max_active_downloads') == 'true'
            q.keys(['-M', 'ctrl', 'a', '-m', 'ctrl', str(value), '-k', 'Return'],
                   lambda: q.state()['settings']['max_active_downloads'] == value
                   and not q.state()['settingFields']['max_active_downloads']['pending'])
            assert q.state()['settingFields']['max_active_downloads']['focused']
            h.cli('settings', '--max-downloads', str(value+1))
            after = q.wait(lambda: s if (s := q.state())['settingFields']['max_active_downloads']['text'] == str(value+1) else None,
                           timeout=2)
            case.check(after['settingFields']['max_active_downloads']['focused'],
                       'focused, unchanged dashboard setting follows an external CLI update', json.dumps(after))
            q.keys(['-M', 'ctrl', 'a', '-m', 'ctrl', str(value+3)],
                   lambda: q.state()['settingFields']['max_active_downloads']['text'] == str(value+3))
            h.cli('settings', '--max-downloads', str(value+2))
            current = q.wait(lambda: s if (s := q.state())['settings']['max_active_downloads'] == value+2 else None)
            case.check(current['settingFields']['max_active_downloads']['text'] == str(value+3),
                       'a deliberate dashboard draft survives a simultaneous CLI edit', json.dumps(current))
            q.keys(['-k', 'Return'], lambda: q.state()['settings']['max_active_downloads'] == value+3)
            case.check(h.cli('settings')['max_active_downloads'] == value+3,
                       'only deliberate Apply commits the preserved dashboard draft', str(value+3))
            return
        assert q.ipc('popup') == 'true'
        q.wait(lambda: q.state().get('opened') and q.state().get('popup'))
        assert ui.contains('F6', 'Simultaneous downloads', literal=False)
        before = h.cli('settings')['max_active_downloads']
        def matches(pane, value):
            return bool(re.search(r'Simultaneous downloads\s+'+str(value)+r'\b', pane))
        assert ui.observe('read initial TUI settings', 'the selected field matches the daemon before an external edit',
                          lambda pane: matches(pane, before))
        value = before+1
        started = time.monotonic()
        written = h.cli('settings', '--max-downloads', str(value))
        assert written['ok'] and written['max_active_downloads'] == value
        observed = q.wait(lambda: s if (s := q.state())['settings']['max_active_downloads'] == value
                          and s['view']['settings']['values']['max_active_downloads'] == value else None, timeout=2)
        case.check(True, 'CLI setting reaches the live dashboard while popup and TUI remain attached',
                   json.dumps(dict(elapsed_ms=round((time.monotonic()-started)*1000), state=observed)))
        assert ui.observe('observe external CLI update in open TUI Settings',
                          'untouched active-downloads field follows the confirmed daemon value',
                          lambda pane: matches(pane, value), timeout=2)
        local = value+1
        assert ui.press('Right', 'edit the TUI draft without saving', lambda pane: matches(pane, local), literal=False)
        h.cli('settings', '--upload', 'on')
        q.wait(lambda: q.state()['settings']['upload'] == 'on')
        assert ui.observe('follow external upload permission while a local field is dirty',
                          'Upload is on and the local active-downloads draft remains unchanged',
                          lambda pane: 'Upload: on' in pane and matches(pane, local), timeout=2)
        q.native_hook('close')
        q.compositor.ipc('[title="^fbittorrent$"] focus')
        assert q.ipc('focusSetting', 'alt_download_limit_bps') == 'true'
        refs = q.state()['mutationRefs']
        q.keys(['-M', 'ctrl', 'a', '-m', 'ctrl', '128K', '-k', 'Return'],
               lambda: q.state()['settings'].get('alt_download_limit_bps') == 131072)
        case.check(q.state()['mutationRefs'] == refs+1, 'one dashboard Apply sends one mutation', str(q.state()['mutationRefs']-refs))
        assert ui.observe('dashboard changes turtle cap beneath the TUI draft',
                          'the confirmed cap updates while the local queue edit survives',
                          lambda pane: any('Turtle download limit' in line and '131072' in line for line in pane.splitlines())
                          and matches(pane, local), timeout=2)
        assert q.ipc('popup') == 'true'
        q.wait(lambda: q.state()['opened'] and q.state()['popup'])
        refs = q.state()['mutationRefs']
        assert q.ipc('turtle') == 'true'
        q.wait(lambda: q.state()['popup']['turtle'] is True and q.state()['appearance']['turtle'] is True, timeout=2)
        case.check(q.state()['mutationRefs'] == refs+1, 'one popup turtle click sends one mutation', str(q.state()['mutationRefs']-refs))
        assert ui.observe('popup turtle change reaches the TUI header', 'the same daemon reports Turtle on',
                          lambda pane: 'Turtle' in pane.splitlines()[0], timeout=2)
        assert ui.press('Enter', 'save only the TUI local field', lambda pane: '[Settings]' not in pane, literal=False)
        observed = q.wait(lambda: s if (s := q.state())['settings']['max_active_downloads'] == local else None, timeout=2)
        current = h.cli('settings')
        case.check(current['max_active_downloads'] == local and current['upload'] == 'on'
                   and current['alt_download_limit_bps'] == 131072 and current['alt_limits_on'] is True,
                   'TUI save preserves simultaneous CLI upload, dashboard cap and popup turtle changes', json.dumps(current))
        h.cli('settings', '--upload', 'off')
        q.wait(lambda: q.state()['settings']['upload'] == 'off')
        case.check(process.child.poll() is None, 'all surfaces still share the original daemon', str(h.daemon_pid()))


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--case', choices=['settings', 'settings_focused', 'settings_race'], default='settings')
    parser.add_argument('--log-root', type=Path)
    args = parser.parse_args()
    if args.log_root:
        import fbtlog
        fbtlog.LOG_ROOT = args.log_root.resolve()
    settings(args.binary.resolve(), focused=args.case == 'settings_focused', race=args.case == 'settings_race')
    raise SystemExit(suite_status('cross_surface'))
