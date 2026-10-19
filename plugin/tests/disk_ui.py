#!/usr/bin/env python3
"""Real 64 MiB destination: native add refusal, disk banners and Errors count."""
import argparse
import base64
from contextlib import ExitStack
import datetime
import json
import os
from pathlib import Path
import shutil
import socket
import time

from dashboard import Session, ROOT
from visual import Capture
import daemon
import fixtures
from daemon_disk import mounted
from fbtlog import Case, suite_status

MIB = 1024 * 1024


def view(q):
    return q.state().get('view') or {}


def start(harness):
    process = harness.start_serve(idle_exit_secs=0)
    if not (harness.receive_ready() and harness.wait_engine()):
        raise RuntimeError('owned disk daemon did not start')
    return process


def run(binary):
    previous = ROOT / 'target/test-logs/qs_disk'
    if previous.exists():
        shutil.copytree(previous, ROOT / 'target/test-logs/fbt-p5-07-gei/runs' /
                        datetime.datetime.now().strftime('%H%M%S'))
    with Case('qs_disk', 'dashboard_disk_and_add_refusal') as case, fixtures.Sandbox() as box, \
            fixtures.Sandbox() as seedbox, mounted(case, box) as available, ExitStack() as stack:
        if not available:
            raise RuntimeError('native disk acceptance requires the private tmpfs')
        with daemon.Harness(case, binary, box, False) as harness, \
                daemon.Harness(case, binary, seedbox, False) as seed:
            payload = seedbox.downloads / 'disk-100MiB.bin'
            with payload.open('wb') as stream:
                block = os.urandom(MIB)
                for _ in range(100):
                    stream.write(block)
            port = daemon.tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME']) /
                                         'fbittorrent/config.toml').read_text())['peer_port']
            body = fixtures.bencode(dict(interval=1, peers=socket.inet_aton('127.0.0.1') + port.to_bytes(2, 'big')))
            http = stack.enter_context(fixtures.HttpFixture(seedbox.root, {'/announce': dict(base64=base64.b64encode(body).decode())}))
            item = fixtures.torrent_fixture(seedbox.root, [payload], tracker=http.base + '/announce')
            runtime = Path(box.env['XDG_DATA_HOME']) / 'fbittorrent/nova3'
            engines = runtime / 'engines'
            engines.mkdir(parents=True)
            row = dict(link='magnet:?xt=urn:btih:' + item.info_hash, name=payload.name,
                       size=100 * MIB, seeds=1, leech=0, engine_url='https://disk.invalid')
            (engines / 'disk.py').write_text('class disk:\n name="Local Disk"\n url="https://disk.invalid"\n def search(self,query):\n  from novaprinter import prettyPrinter\n  prettyPrinter(' + repr(row) + ')\n')
            filler = box.downloads / 'filler'
            filler.write_bytes(b'F' * (32 * MIB))
            process, seedprocess = start(harness), start(seed)
            client = stack.enter_context(daemon.ApiConn(harness))
            peer = stack.enter_context(daemon.ApiConn(seed))
            client.hello()
            peer.hello()
            peer.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(seedbox.downloads), seed=True))
            seed.wait_for(lambda: ((value := peer.call('torrents.get', dict(hash=item.info_hash))['result']['torrent'])['state'] == 'seeding', json.dumps(value)),
                          'real seed completes its hash check before the client can connect', timeout=20)
            peer.call('transfer.set_limits', dict(upload_bps=128 * 1024))
            q = Session(case, box)
            try:
                q.wait(lambda: q.ipc('ready') == 'true')
                q.shell('summon', json.dumps(dict(view='search', query='Disk')))
                q.wait(lambda: len(view(q).get('rows', [])) == 1, timeout=12)
                q.keys(['-k', 'Down'], lambda: view(q).get('focus') == 'results')
                q.keys(['-k', 'Return'], lambda: view(q).get('add', {}).get('insufficientSpace'))
                review = view(q)['add']
                case.check(review['neededBytes'] == 100 * MIB and review['space']['free_bytes'] <= 64 * MIB
                           and not review['destinationReady'],
                           'actual 100 MiB provider result is refused against the real 64 MiB destination', json.dumps(review))
                q.keys(['-k', 'Return'], lambda: view(q).get('add', {}).get('visible'))
                case.check(client.call('torrents.list', {})['result']['torrents'] == [],
                           'Enter cannot bypass the disabled Download control; no torrent was added', json.dumps(view(q)['add']))
                image = case.path.with_suffix('.add.png')
                q.ipc('grab', image)
                q.wait(lambda: image.exists() and image.stat().st_size > 0)
                q.keys(['-k', 'Escape'], lambda: not view(q).get('add', {}).get('visible'))
                q.shell('summon', json.dumps(dict(view='torrents')))
                q.wait(lambda: view(q).get('window', {}).get('view') == 'torrents')
                client.call('torrents.add', dict(source=dict(path=str(item.torrent))))
                q.wait(lambda: view(q).get('diskBanner', '').startswith('Not enough space for the running downloads:'), timeout=8)
                before = client.call('torrents.get', dict(hash=item.info_hash))['result']['torrent']
                case.check(before['state'] != 'error' and 'MiB needed' in view(q)['diskBanner'] and 'MiB free' in view(q)['diskBanner'],
                           'native dashboard shows binary-unit warning before the first ENOSPC', json.dumps(dict(view=view(q), torrent=before)))
                image = case.path.with_suffix('.banner.png')
                q.ipc('grab', image)
                q.wait(lambda: image.exists() and image.stat().st_size > 0)
                peer.call('transfer.set_limits', dict(upload_bps=2 * MIB))
                q.wait(lambda: view(q).get('errors') == 1, timeout=40)
                failed = client.call('torrents.get', dict(hash=item.info_hash))['result']['torrent']
                case.check(failed['state'] == 'error' and failed.get('reason') == 'disk_full',
                           'real storage ENOSPC appears as Disk full and increments the rendered Errors filter', json.dumps(dict(view=view(q), torrent=failed)))
                filler.write_bytes(b'')
                previous_bytes=failed.get('completed_bytes',0)
                q.wait(lambda: view(q).get('errors') == 0, timeout=15)
                harness.wait_for(lambda: ((value := client.call('torrents.get', dict(hash=item.info_hash))['result']['torrent']).get('completed_bytes',0) > previous_bytes,
                                         json.dumps(value)), 'freeing the owned filler resumes bytes without any start request', timeout=10)
                case.check(True, 'automatic recovery clears the rendered Errors count without a start action', json.dumps(view(q)))
            finally:
                q.close()
                seed.stop_serve(seedprocess)
                harness.stop_serve(process)
    with Case('qs_disk', 'popup_disk_banner') as case, fixtures.Sandbox() as box, \
            mounted(case, box) as available, daemon.Harness(case, binary, box, False) as harness:
        if not available:
            raise RuntimeError('native disk acceptance requires the private tmpfs')
        payload = box.root / 'popup-100MiB.bin'
        with payload.open('wb') as stream:
            stream.truncate(100 * MIB)
        item = fixtures.torrent_fixture(box.root, [payload])
        process = start(harness)
        with daemon.ApiConn(harness) as client:
            client.hello()
            client.call('torrents.add', dict(source=dict(path=str(item.torrent))))
            q = Capture(case, box, 'owner')
            try:
                q.wait(lambda: json.loads(q.ipc('state'))['online'])
                q.ipc('open')
                state = q.wait(lambda: value if (value := json.loads(q.ipc('state')))['bar']['state'] == 'disk_low' else None)
                case.check(state['bar']['tone'] == 'urgent' and '100.0 MiB needed' in state['banner']
                           and '64.0 MiB free' in state['banner'],
                           'native popup and urgent bar show actual needed and free bytes', json.dumps(state))
                image = case.path.with_suffix('.png')
                q.capture(image)
            finally:
                q.close()
        harness.stop_serve(process)
    return suite_status('qs_disk')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
