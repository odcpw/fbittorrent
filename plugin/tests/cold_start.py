#!/usr/bin/env python3
"""Clean private installation and real xdg-open cold magnet review, twice."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import shlex
import subprocess
import time

from dashboard import Session
import daemon
import desktop_setup
import fixtures
from fbtlog import Case, suite_status


def run(binary):
    with Case('cross_surface', 'cold_start') as case, ExitStack() as stack:
        box = stack.enter_context(daemon.Sandbox())
        config = desktop_setup.prepare(box)
        app_config = config/'fbittorrent/config.toml'
        app_config.unlink()  # Only the generated private fixture's defaults.
        case.check(not list(Path(box.env['HOME']).iterdir()), 'setup starts with an empty private HOME', str(box.root))
        desktop_setup.call(case, binary, box, '--magnet', '--plugin=copy')
        installed = config/'omarchy/plugins/oliver.torrents'
        layout = json.loads((config/'omarchy/shell.json').read_text())['bar']
        case.check(installed.is_dir(), 'setup copied the plugin into the private installation', str(installed))
        # No daemon has run. Bind all future engine traffic to lo and keep every
        # path explicit before the real unit is even linked into its runtime.
        app_config.parent.mkdir(parents=True, exist_ok=True)
        app_config.write_text('autostart = true\nnotify = false\nnetwork_interface = "lo"\npeer_port = 0\n'
                              'save_path = '+json.dumps(str(box.downloads))+'\n')
        box.env['FBT_TEST_BAR_CONFIG'] = json.dumps(layout)
        h = stack.enter_context(daemon.Harness(case, binary, box, False))
        unit = stack.enter_context(daemon.InstalledUnit(h))
        q = Session(case, box, recipe='bar-host', source_link=installed)
        stack.callback(q.close)
        startup_log = case.path.with_suffix('.startup.log')
        fixtures.path_shim('fbittorrent', 'exec '+shlex.quote(str(binary))+' "$@" >>'+shlex.quote(str(startup_log))+' 2>&1', box.root)
        fixtures.path_shim('omarchy-shell', 'exec /usr/share/omarchy/bin/omarchy-shell "$@"', box.root)
        q.wait(lambda: q.state()['widgets'] == 1)
        case.check(unit.pid() == 0 and not q.state()['online'], 'the installed bar loads without starting the closed daemon', json.dumps(q.state()))
        measurements = []
        previous_pid = 0
        for index in range(2):
            name = 'Private cold review '+str(index+1)
            magnet = 'magnet:?xt=urn:btih:'+str(index+1)*40+'&dn='+name.replace(' ', '%20')
            started = time.monotonic()
            child = subprocess.Popen(['/usr/bin/xdg-open', magnet], env=dict(q.env, DE='generic', XDG_CURRENT_DESKTOP='generic'),
                                     cwd=box.root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                def sheet():
                    state = q.state()
                    review = (state.get('view') or {}).get('add') or {}
                    return review if review.get('visible') and review.get('name') == name else None
                review = q.wait(sheet, timeout=15)
                elapsed = round((time.monotonic()-started)*1000)
                q.wait(lambda: q.state()['online'], timeout=15)
                ready_ms = round((time.monotonic()-started)*1000)
                unit.verify_child()
                pid = unit.pid()
                out, err = child.communicate(timeout=5)
                case.check(child.returncode == 0 and pid > 1 and pid != previous_pid,
                           'real xdg-open starts a new isolated daemon and opens the installed review sheet',
                           json.dumps(dict(round=index+1, click_to_sheet_ms=elapsed, click_to_online_ms=ready_ms, pid=pid, previous_pid=previous_pid,
                                           review=review, stdout=out, stderr=err)))
                case.check(not list(box.downloads.iterdir()) and not review.get('seed'),
                           'cold review neither commits payload nor enables upload', json.dumps(review))
                measurements.append(dict(sheet_ms=elapsed, online_ms=ready_ms))
                q.compositor.ipc('[title="^fbittorrent$"] focus')
                q.keys(['-k', 'Escape'], lambda: not ((q.state().get('view') or {}).get('add') or {}).get('visible'))
                h.cli('down')
                q.wait(lambda: not q.state()['online'])
                case.check(unit.pid() == 0 and h.lock_free(), 'down fully stops the owned unit before the next cold click', unit.state())
                previous_pid = pid
            except BaseException:
                case.path.with_suffix('.state.json').write_text(json.dumps(q.state(), indent=2))
                raise
            finally:
                if child.poll() is None:
                    child.terminate()
                    child.communicate(timeout=3)
        print(json.dumps(dict(cold_click_to_sheet_ms=measurements)), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--log-root', type=Path)
    args = parser.parse_args()
    if args.log_root:
        import fbtlog
        fbtlog.LOG_ROOT = args.log_root.resolve()
    run(args.binary.resolve())
    raise SystemExit(suite_status('cross_surface'))
