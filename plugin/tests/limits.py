#!/usr/bin/env python3
"""Retired controls stay absent while legacy limit API state remains readable."""
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
    with Case('qs_limits', 'popup_retired_limits') as case, fixtures.Sandbox() as sandbox, \
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
                serial=json.loads(q.ipc('cursor'))['keySerial']
                press(q, 'z', lambda: json.loads(q.ipc('cursor'))['keySerial']>serial)
                state = popup(q)
                settings = client.call('app.settings.get')['result']
                case.check('turtle' not in state['bar'] and 'turtle' not in state['bar']['tooltip'].lower()
                           and not settings['alt_limits_on'],
                           'retired popup z does not mutate limits or add a bar indicator',
                           json.dumps(dict(ui=state, settings=settings)))
                image = case.path.with_suffix('.png')
                q.capture(image)
                case.check(q.ipc('clickTurtle') == 'false', 'retired hero button is absent', str(image))
                client.call('transfer.set_alt', dict(on=True))
                began = time.monotonic()
                q.wait(lambda: json.loads(q.ipc('activity'))['summary'].get('alt') is True)
                elapsed = round((time.monotonic() - began) * 1000)
                case.check(elapsed <= 1000 and 'turtle' not in popup(q)['bar'], 'legacy API changes still arrive without restoring retired controls',
                           json.dumps(dict(ms=elapsed, ui=popup(q))))
                harness.stop_serve(process)
                harness.notify.close()
                process = start(harness)
                q.wait(lambda: popup(q)['online'] and json.loads(q.ipc('activity'))['summary'].get('alt') is True)
                case.check(q.ipc('clickTurtle')=='false' and 'turtle' not in popup(q)['bar'],
                           'persisted legacy limits do not restore controls after reconnect',json.dumps(popup(q)))
            finally:
                q.close()
        harness.stop_serve(process)
        harness.lock_free()
    with Case('qs_limits', 'dashboard_retired_limits') as case, fixtures.Sandbox() as sandbox, \
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
                q.keys(['z'], lambda: q.state().get('view'))
                case.check(not client.call('app.settings.get')['result']['alt_limits_on']
                           and 'turtle' not in q.state()['view'],
                           'dashboard z is inert and no mode indicator is published', json.dumps(q.state()))
                image = case.path.with_suffix('.png')
                q.ipc('grab', image)
                q.wait(lambda: image.exists() and image.stat().st_size > 0)
                q.keys([','], lambda:q.state()['view']['window']['view']=='settings')
                q.wait(lambda:q.state()['view'].get('settings',{}).get('loaded'))
                case.check(q.ipc('focusSetting','alt_limits_on')=='false'
                           and q.ipc('focusSetting','alt_download_limit_bps')=='false'
                           and q.ipc('focusSetting','alt_upload_limit_bps')=='false',
                           'Settings exposes ordinary caps and no retired alternate controls',json.dumps(q.state()))
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
