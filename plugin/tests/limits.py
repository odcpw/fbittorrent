#!/usr/bin/env python3
"""Turtle controls and confirmed push state on real isolated shell surfaces."""
import argparse
import datetime
import json
from pathlib import Path
import shutil
import subprocess
import time

from dashboard import Session, ROOT
from visual import Capture
import daemon
import fixtures
from fbtlog import Case, suite_status


def popup(q):
    return json.loads(q.ipc('state'))


def press(q, key, predicate):
    child = subprocess.Popen(['wtype', '-s', '80', key, '-s', '700'], env=q.env)
    try:
        return q.wait(predicate)
    finally:
        child.wait(timeout=3)


def start(harness):
    process = harness.start_serve()
    if not (harness.receive_ready() and harness.wait_engine()):
        raise RuntimeError('owned limits daemon did not become ready')
    return process


def run(binary):
    previous = ROOT / 'target/test-logs/qs_limits'
    if previous.exists():
        shutil.copytree(previous, ROOT / 'target/test-logs/fbt-p5-04-ipg/runs' /
                        datetime.datetime.now().strftime('%H%M%S'))
    with Case('qs_limits', 'popup_turtle_toggle') as case, fixtures.Sandbox() as sandbox, \
            daemon.Harness(case, binary, sandbox, False) as harness:
        process = start(harness)
        with daemon.ApiConn(harness) as client:
            hello = client.hello()['result']
            case.check({'limits', 'subscribe'}.issubset(hello['caps']),
                       'real daemon provides live limits and pushed subscriptions', json.dumps(hello))
            q = Capture(case, sandbox, 'owner')
            try:
                q.wait(lambda: popup(q)['online'])
                q.ipc('open')
                q.wait(lambda: popup(q)['opened'])
                press(q, 'z', lambda: popup(q)['turtle'] and popup(q)['popupTurtle'])
                state = popup(q)
                settings = client.call('app.settings.get')['result']
                case.check(state['bar']['turtle'] and 'turtle limits on' in state['bar']['tooltip']
                           and settings['alt_limits_on'],
                           'popup z persists turtle mode and the bar glyph and hero show its confirmed state',
                           json.dumps(dict(ui=state, settings=settings)))
                image = case.path.with_suffix('.png')
                q.capture(image)
                case.check(q.ipc('clickTurtle') == 'true', 'actual hero turtle button is clickable', str(image))
                q.wait(lambda: not popup(q)['turtle'] and not popup(q)['popupTurtle']
                       and not popup(q)['bar']['turtle'])
                case.check(not client.call('app.settings.get')['result']['alt_limits_on'],
                           'hero button clears the persisted limit mode', json.dumps(popup(q)))
                client.call('transfer.set_alt', dict(on=True))
                began = time.monotonic()
                q.wait(lambda: popup(q)['turtle'] and popup(q)['bar']['turtle'])
                elapsed = round((time.monotonic() - began) * 1000)
                case.check(elapsed <= 1000, 'external turtle change reaches the bar and popup within one 1000 ms push interval',
                           json.dumps(dict(ms=elapsed, ui=popup(q))))
                harness.stop_serve(process)
                harness.notify.close()
                process = start(harness)
                q.wait(lambda: popup(q)['online'] and popup(q)['turtle'] and popup(q)['bar']['turtle'])
                case.check(True, 'bar and popup recover the persisted turtle mode after daemon restart', json.dumps(popup(q)))
            finally:
                q.close()
        harness.stop_serve(process)
        harness.lock_free()
    with Case('qs_limits', 'dashboard_turtle_toggle') as case, fixtures.Sandbox() as sandbox, \
            daemon.Harness(case, binary, sandbox, False) as harness:
        process = start(harness)
        with daemon.ApiConn(harness) as client:
            client.hello()
            q = Session(case, sandbox)
            try:
                q.wait(lambda: q.ipc('ready') == 'true')
                q.shell('summon', json.dumps(dict(view='torrents')))
                q.wait(lambda: q.state().get('view'))
                q.wait(lambda: q.compositor.ipc('[title="^fbittorrent$"] floating enable, border none'))
                q.compositor.ipc('seat seat0 cursor set 1 1')
                q.keys(['z'], lambda: q.state()['view']['turtle'])
                case.check(client.call('app.settings.get')['result']['alt_limits_on'],
                           'dashboard z updates the real daemon and global top-bar indicator', json.dumps(q.state()))
                image = case.path.with_suffix('.png')
                q.ipc('grab', image)
                q.wait(lambda: image.exists() and image.stat().st_size > 0)
                client.call('transfer.set_alt', dict(on=False))
                began = time.monotonic()
                q.wait(lambda: not q.state()['view']['turtle'])
                elapsed = round((time.monotonic() - began) * 1000)
                case.check(elapsed <= 1000, 'external change clears the dashboard indicator within one 1000 ms push interval',
                           json.dumps(dict(ms=elapsed, state=q.state())))
            finally:
                q.close()
        harness.stop_serve(process)
        harness.lock_free()
    return suite_status('qs_limits')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
