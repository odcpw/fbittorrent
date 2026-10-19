#!/usr/bin/env python3
"""Native Settings/Log/Help acceptance over the real daemon API."""
import argparse
import datetime
import json
from pathlib import Path
import shutil
import socket

from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status


def view(q):
    return q.state().get('view') or {}


def settings(q):
    return view(q).get('settings') or {}


def set_control(case, q, client, key, value, expected):
    client.pushed.clear()
    result = q.ipc('setting', key, json.dumps(value))
    if result != 'ok':
        raise RuntimeError('setting control refused: ' + result)
    q.wait(lambda: settings(q).get('values', {}).get(key) == expected
           and key not in settings(q).get('pending', {}), timeout=15)
    q.wait(lambda: any(frame.get('topic') == 'settings' and frame.get('data', {}).get(key) == expected
                      for frame in (client.call('app.info'), client.pushed)[1]), timeout=5)
    wire = next(frame for frame in reversed(client.pushed)
                if frame.get('topic') == 'settings' and frame.get('data', {}).get(key) == expected)
    trace = json.loads(q.ipc('settingsTrace'))
    reference, action = next((reference, action) for reference, action in reversed(list(trace['actions'].items()))
                             if action['op'] == 'app.settings.set' and key in action['args'])
    request = dict(action, ref=reference)
    response = trace['results'][reference]
    actual = client.call('app.settings.get')['result']
    case.check(request['args'] == {key: expected} and response['status'] == 'done'
               and actual.get(key) == expected,
               'control round-trips ' + key + ' through a real request, acknowledgement and pushed settings',
               json.dumps(dict(input=value, request=request, reply=response, frame=wire, readback=actual.get(key))))


def run(binary):
    previous = ROOT / 'target/test-logs/qs_preferences'
    if previous.exists():
        shutil.copytree(previous, ROOT / 'target/test-logs/fbt-p3-15-tsg/runs' /
                        datetime.datetime.now().strftime('%H%M%S'))
    with Case('qs_preferences', 'fixture') as setup, fixtures.Sandbox() as box:
        payload = box.downloads / 'log.bin'
        payload.write_bytes(b'owned engine log fixture' * 1024)
        item = fixtures.torrent_fixture(box.root, [payload])
        fixtures.corrupt_byte(payload, 0)
        box.env['FBT_TEST_INJECT_ERROR'] = item.info_hash
        box.keys.append('FBT_TEST_INJECT_ERROR')
        fixtures.path_shim('wl-paste', 'exec sleep infinity', box.root)
        fixtures.path_shim('notify-send', 'exit 0', box.root)
        with daemon.Harness(setup, binary, box, False) as harness:
            process = harness.start_serve(idle_exit_secs=0)
            if not (harness.receive_ready() and harness.wait_engine()):
                raise RuntimeError('owned preferences daemon did not start')
            with daemon.ApiConn(harness) as client:
                hello = client.hello()['result']
                client.call('subscribe', dict(topics=['settings'], interval_ms=250))
                q = Session(setup, box)
                try:
                    q.wait(lambda: q.ipc('ready') == 'true')
                    q.shell('summon', '{"view":"settings"}')
                    q.wait(lambda: settings(q).get('loaded') and 'max_active_downloads' in settings(q).get('values', {}))
                    with Case('qs_preferences', 'settings_roundtrip_each') as case:
                        save = box.root / 'new downloads'
                        save.mkdir()
                        values = [
                            ('max_active_downloads', '4', 4), ('save_path', str(save), str(save)),
                            ('seed_by_default', True, True), ('idle_exit_secs', '600', 600),
                            ('peer_encryption', 'prefer', 'prefer'), ('network_interface', '', None),
                            ('peer_port', '0', 0), ('download_limit_bps', '64K', 65536),
                            ('upload_limit_bps', '64K', 65536), ('alt_download_limit_bps', '32K', 32768),
                            ('alt_upload_limit_bps', '16K', 16384), ('alt_limits_on', True, True),
                            ('open_command', 'xdg-open --', 'xdg-open --'), ('handler', 'add', 'add'),
                            ('show_quality', '720p', '720p'), ('notify', True, True), ('autostart', True, True),
                            ('clipboard_watch', True, True), ('reduce_motion', True, True), ('timeout_secs', '12', 12),
                        ]
                        for key, value, expected in values:
                            set_control(case, q, client, key, value, expected)
                        set_control(case, q, client, 'clipboard_watch', False, False)
                        set_control(case, q, client, 'download_limit_bps', '', None)
                        set_control(case, q, client, 'network_interface', 'lo', 'lo')
                        case.check('lo' in settings(q)['interfaces'], 'binding options come from real app.network interfaces', json.dumps(settings(q)['interfaces']))
                        q.ipc('focusSetting', 'max_active_downloads')
                        q.keys(['-M', 'ctrl', 'a', '-m', 'ctrl', '5', '-k', 'Return'],
                               lambda: settings(q).get('values', {}).get('max_active_downloads') == 5)
                        case.check(client.call('app.settings.get')['result']['max_active_downloads'] == 5,
                                   'native keyboard editing and Enter use the same setting path', json.dumps(settings(q)))
                        image = case.path.with_suffix('.png')
                        q.ipc('grab', image)
                        q.wait(lambda: image.exists() and image.stat().st_size > 0)
                    with Case('qs_preferences', 'settings_restart_class_banner') as case:
                        before = client.call('app.info')['result']
                        with socket.socket() as probe:
                            probe.bind(('127.0.0.1', 0))
                            port = probe.getsockname()[1]
                        q.ipc('setting', 'peer_port', json.dumps(str(port)))
                        q.wait(lambda: settings(q).get('banner') == 'applies after restart; restarting now')
                        case.check(q.state()['instance'] == hello['instance'],
                                   'restart-class field shows the exact banner on the surviving connection', json.dumps(dict(banner=settings(q)['banner'], instance=q.state()['instance'])))
                        q.wait(lambda: (info := client.call('app.info')['result'])['state'] == 'running'
                               and info['engine_epoch'] > before['engine_epoch'], timeout=15)
                        after = client.call('app.info')['result']
                        case.check(after['instance'] == before['instance'] and after['clients']['bar'] == 1,
                                   'engine-slot restart retains daemon instance and the single shared bar client', json.dumps(dict(before=before, after=after)))
                    with Case('qs_preferences', 'log_view_filter') as case:
                        q.keys(['-k', 'Escape'], lambda: view(q).get('window', {}).get('view') == 'torrents')
                        q.keys(['L'], lambda: view(q).get('window', {}).get('view') == 'log')
                        client.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(box.downloads), stopped=True))
                        q.wait(lambda: any('injected engine error' in row['text'] for row in json.loads(q.ipc('logRows'))), timeout=8)
                        rows = json.loads(q.ipc('logRows'))
                        case.check(any('error · engine' in row['text'] and 'injected engine error' in row['text'] for row in rows),
                                   'Log shows an actual injected engine error with its source', json.dumps(rows))
                        q.ipc('logFilter', 'level', 'error')
                        q.ipc('logFilter', 'source', 'engine')
                        q.wait(lambda: view(q).get('log', {}).get('source') == 'engine')
                        rows = json.loads(q.ipc('logRows'))
                        case.check(bool(rows) and all('error · engine' in row['text'] for row in rows),
                                   'actual source/level filter chips retain only matching rendered rows', json.dumps(rows))
                        image = case.path.with_suffix('.png')
                        q.ipc('grab', image)
                        q.wait(lambda: image.exists() and image.stat().st_size > 0)
                    with Case('qs_preferences', 'help_lists_every_key') as case:
                        q.keys(['?'], lambda: view(q).get('help') is True)
                        shown = json.loads(q.ipc('helpKeys'))
                        source = Path(__import__('os').environ.get('FBT_PLUGIN_SOURCE', ROOT / 'plugin/oliver.torrents')) / 'keymap.json'
                        expected = [row for row in json.loads(source.read_text())['actions'] if isinstance(row['keys'].get('dashboard'), list)]
                        actual = {row['id']: row for row in shown}
                        for row in expected:
                            found = actual.get(row['id'])
                            case.check(found is not None and found['keys'] == ' / '.join(row['keys']['dashboard'])
                                       and found['scope'] == row['scope'] and row['label'] in found['label'],
                                       'Help renders every generated dashboard key for ' + row['id'], json.dumps(found))
                        case.check(len(shown) == len(expected), 'Help renders each dashboard row exactly once', str(len(shown)))
                        required={"L",",","?","z","q","Q","Ctrl+Q","Ctrl+V","1","2","3","4","Tab","Shift+Tab","[","]","y","Esc"}
                        tokens={key for row in expected for key in row['keys']['dashboard']}
                        case.check(required <= tokens, 'current global, details and offer keys are explicitly present', json.dumps(sorted(required-tokens)))
                        image = case.path.with_suffix('.png')
                        q.ipc('grab', image)
                        q.wait(lambda: image.exists() and image.stat().st_size > 0)
                        q.keys(['-k', 'Escape'], lambda: view(q).get('help') is False)
                        case.check(q.state()['online'], 'Esc closes Help while the dashboard and daemon remain running', json.dumps(view(q)))
                        q.keys(['-M','ctrl','-k','c','-m','ctrl'],lambda:q.state().get('view') is None)
                        case.check(client.call('app.info')['ok'], 'documented Ctrl+C closes only the window', json.dumps(q.state()))
                finally:
                    q.close()
            harness.stop_serve(process)
            harness.lock_free()
    return suite_status('qs_preferences')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
