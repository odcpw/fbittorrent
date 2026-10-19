#!/usr/bin/env python3
"""Native clipboard offers from the real daemon's opt-in bounded reader."""
import argparse
import datetime
import json
from pathlib import Path
import shlex
import shutil
import sys
import time

from dashboard import Session, ROOT
from visual import Capture
from popup import keys
import daemon
import fixtures
from fbtlog import Case, suite_status

HASH = 'ce' * 20
NAME = 'Owned clipboard selection'


def producer(case, box):
    inputs = box.root / 'selections.ndjson'
    inputs.touch()
    trace = case.path.with_suffix('.producer.jsonl')
    program = box.root / 'producer.py'
    program.write_text('''import json,os,subprocess,sys,time
selections,trace=sys.argv[1:3]
command=sys.argv[4:]
with open(trace,'a') as out:out.write(json.dumps(dict(pid=os.getpid(),argv=sys.argv[3:]))+'\\n')
with open(selections) as stream:
 while True:
  line=stream.readline()
  if not line:time.sleep(.02);continue
  value=json.loads(line)
  subprocess.run(command,input=value.encode(),check=True)
  with open(trace,'a') as out:out.write(json.dumps(dict(at=time.monotonic(),selection=value))+'\\n')
''')
    fixtures.path_shim('wl-paste', 'exec ' + shlex.join([sys.executable, str(program), str(inputs), str(trace)]) + ' "$@"', box.root)
    fixtures.path_shim('notify-send', 'exit 0', box.root)
    config = Path(box.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml'
    config.write_text(config.read_text().replace('notify = false', 'notify = true'))

    def send(hash, name=NAME):
        with inputs.open('a') as output:
            output.write(json.dumps('magnet:?xt=urn:btih:' + hash + '&dn=' + name) + '\n')
    return trace, send


def run(binary):
    previous = ROOT / 'target/test-logs/qs_clipboard'
    if previous.exists():
        shutil.copytree(previous, ROOT / 'target/test-logs/fbt-p6-13-dxe/runs' /
                        datetime.datetime.now().strftime('%H%M%S'))
    for surface in ['popup', 'dashboard']:
        with Case('qs_clipboard', surface + '_offer') as case, fixtures.Sandbox() as box, \
                daemon.Harness(case, binary, box, False) as harness:
            trace, send = producer(case, box)
            process = harness.start_serve(idle_exit_secs=0)
            if not (harness.receive_ready() and harness.wait_engine()):
                raise RuntimeError('owned clipboard daemon did not start')
            with daemon.ApiConn(harness) as client:
                client.hello()
                case.check(client.call('app.settings.get')['result']['clipboard_watch'] is False and not trace.exists(),
                           'clipboard reading defaults off; no reader exists', str(trace))
                q = Capture(case, box, 'owner') if surface == 'popup' else Session(case, box)
                try:
                    if surface == 'popup':
                        q.wait(lambda: json.loads(q.ipc('state'))['online'])
                        q.wait(lambda: json.loads(q.ipc('state'))['mode'] == 'subscribe')
                        offer = lambda: json.loads(q.ipc('offer'))
                    else:
                        q.wait(lambda: q.ipc('ready') == 'true')
                        q.shell('summon', '{"view":"torrents"}')
                        q.wait(lambda: q.state().get('view'))
                        offer = lambda: (q.state().get('view') or {}).get('clipboard')
                    client.call('app.settings.set', dict(clipboard_watch=True))
                    q.wait(lambda: trace.exists())
                    send(HASH)
                    q.wait(lambda: offer() and offer()['hash'] == HASH)
                    case.check(offer()['name'] == NAME, 'live daemon selection appears in the actual ' + surface + ' offer', json.dumps(offer()))
                    if surface == 'popup':
                        case.check(not json.loads(q.ipc('state'))['opened'], 'bar receives the live offer while both UIs are closed', json.dumps(offer()))
                        q.ipc('open')
                        q.wait(lambda: json.loads(q.ipc('state'))['opened'])
                    image = case.path.with_suffix('.png')
                    if surface == 'popup':
                        q.capture(image)
                    else:
                        q.ipc('grab', image)
                        q.wait(lambda: image.exists() and image.stat().st_size > 0)
                    notifications = box.root / 'shims/notify-send.log'
                    q.wait(lambda: notifications.exists())
                    case.check(sum('Magnet on clipboard:' in row for row in notifications.read_text().splitlines()) == 1,
                               'daemon emits one notification for the offered selection', notifications.read_text())
                    if surface == 'popup':
                        case.check(q.ipc('clickClipboard', 'add') == 'true', 'actual popup Add button is clickable', str(image))
                    else:
                        q.keys(['y'], lambda: not offer())
                    harness.wait_for(lambda: (any(row['hash'] == HASH and row.get('origin') == 'clipboard'
                                                 for row in client.call('torrents.list', {})['result']['torrents']), 'clipboard-origin ledger record'),
                                     'explicit Add creates a real clipboard-origin torrent')
                    q.wait(lambda: not offer())
                    send(HASH)
                    send(HASH)
                    q.wait(lambda: len(trace.read_text().splitlines()) >= 4)
                    case.check(not offer(), 'same selection is not re-offered', json.dumps(offer()))
                    send('df' * 20, 'Dismiss owned selection')
                    q.wait(lambda: offer() and offer()['hash'] == 'df' * 20)
                    if surface == 'popup':
                        case.check(q.ipc('clickClipboard', 'dismiss') == 'true', 'actual popup Dismiss button is clickable', str(image))
                        q.wait(lambda: not offer())
                    else:
                        q.keys(['-k', 'Escape'], lambda: not offer())
                    case.check(not any(row['hash'] == 'df' * 20 for row in client.call('torrents.list', {})['result']['torrents']),
                               'Dismiss sends no add for the second selection', json.dumps(offer()))
                    client.call('app.settings.set', dict(clipboard_watch=False))
                    pid = json.loads(trace.read_text().splitlines()[0])['pid']
                    q.wait(lambda: not daemon.process_alive(pid))
                    calls = [json.loads(line) for line in (box.root / 'shims/wl-paste.log').read_text().splitlines()]
                    case.check(len(calls) == 1 and calls[0][0] == '--watch',
                               'only the daemon owns a clipboard reader; UI spawns no clipboard process for events', json.dumps(calls))
                    text = harness.stderr() + json.dumps(client.call('app.log', dict(limit=1000))['result'])
                    case.check(NAME not in text and 'Dismiss owned selection' not in text and '&dn=' not in text,
                               'clipboard selection text is absent from daemon stderr and app.log', 'fixed-string selection scan returned no matches')
                finally:
                    q.close()
            harness.stop_serve(process)
            harness.lock_free()
    return suite_status('qs_clipboard')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
