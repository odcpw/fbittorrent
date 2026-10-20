"""A large file count must not hide the whole engine behind payload fsync."""
import json
import hashlib
import socket
from pathlib import Path
import time

import daemon as d
import fixtures


def many_files_completion_keeps_reads_live(binary, systemd):
    # /tmp is tmpfs on the development host. Keep the whole sandbox private,
    # but use a real filesystem so payload durability actually exercises I/O.
    with d.api_owned('many_files_completion_keeps_reads_live', binary, systemd, directory='/var/tmp') as (case, box, h, process):
        folder = box.downloads / 'many-files'
        folder.mkdir()
        payload = b'private many-file fixture\n' * 160
        paths = []
        for index in range(10000):
            path = folder / f'{index:05}.bin'
            path.write_bytes(payload + index.to_bytes(8, 'big'))
            paths.append(path)
        item = fixtures.torrent_fixture(box.root, paths, name='many-files', piece_length=262144)
        latencies = []
        with d.ApiConn(h) as add, d.ApiConn(h) as reader:
            add.hello(); reader.hello()
            identifier = add.send('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(folder)))
            deadline = time.monotonic() + 120
            row = None
            while time.monotonic() < deadline:
                began = time.monotonic()
                rows = reader.call('torrents.list', {})['result']['torrents']
                latencies.append(time.monotonic() - began)
                row = next((value for value in rows if value['hash'] == item.info_hash), None)
                if row:
                    began = time.monotonic()
                    row = reader.call('torrents.get', dict(hash=item.info_hash))['result']['torrent']
                    latencies.append(time.monotonic() - began)
                    case.check(row['state'] != 'error' and not row['upload_enabled'],
                               'reads answer during initial checking and durable completion; upload stays off', json.dumps(row))
                    if row.get('completed_at') and row['state'] == 'done':
                        break
                time.sleep(.1)
            response = add.response(identifier)
            case.check(response.get('ok') is True and response['result'].get('hash') == item.info_hash,
                       'the asynchronous observation did not disturb add admission', json.dumps(response))
            case.check(bool(row and row.get('completed_at') and row['state'] == 'done'),
                       'all 10000 files reach durably recorded completion', json.dumps(row))
        case.check(all(path.read_bytes() == payload + index.to_bytes(8, 'big') for index, path in enumerate(paths)),
                   'every completed payload file retains its exact generated contents', str(folder))
        case.step('observe read latency while files are certified', 'no read timed out or lost the engine').ok(
            json.dumps(dict(samples=len(latencies), max_seconds=max(latencies))))


def _discarded_metadata(binary, systemd, parked):
    # Metadata-only review churn isolates retained memory from payload I/O.
    # A warm-up covers dispatcher arenas before measuring further unique hashes.
    name = 'parked_discarded_many_file_metadata_is_released' if parked else 'discarded_many_file_metadata_is_released'
    with d.api_owned(name, binary, systemd, directory='/tmp') as (case, box, h, process), d.ApiConn(h) as client:
        hello = client.hello()
        pid = hello['result']['server']['pid']
        if parked:
            interface = f'fbtz{pid:x}'
            assert interface not in {name for _, name in socket.if_nameindex()}
            client.call('app.settings.set', dict(network_interface=interface))
            assert h.wait_for(lambda: (client.call('app.info', {})['result']['state'] == 'parked', interface),
                              'the private daemon parks on its nonexistent fixture interface')
        files = [dict(length=1, path=[f'{index:05}.bin']) for index in range(10000)]
        path = box.root / 'review.torrent'
        samples = []
        for index in range(48):
            path.write_bytes(fixtures.bencode(dict(info={
                'files': files, 'name': f'review-{index:03}', 'piece length': 16384,
                'pieces': hashlib.sha1(b'\0' * 10000).digest(), 'private': 1,
            })))
            reply = client.call('torrents.add', dict(source=dict(path=str(path)), stage=True))
            digest = reply['result']['hash']
            row = client.call('torrents.get', dict(hash=digest))['result']['torrent']
            case.check(row['staged'] and row['size'] == 10000,
                       'review parses each unique 10000-file metainfo without committing it', digest)
            client.call('torrents.discard', dict(hash=digest))
            rows = client.call('torrents.list', {})['result']['torrents']
            case.check(not rows and not list(box.downloads.iterdir()),
                       'discard releases the row and never creates payload files', digest)
            status = dict(line.split(':', 1) for line in Path(f'/proc/{pid}/status').read_text().splitlines())
            samples.append(int(status['VmRSS'].split()[0]))
        warm = max(samples[:16])
        growth = max(samples[16:]) - warm
        case.check(growth <= 16 * 1024,
                   'discarded metainfo does not accumulate with every review after allocator warm-up',
                   json.dumps(dict(warm_peak_kib=warm, later_peak_kib=max(samples[16:]), growth_kib=growth, samples_kib=samples)))


def discarded_many_file_metadata_is_released(binary, systemd):
    _discarded_metadata(binary, systemd, False)


def parked_discarded_many_file_metadata_is_released(binary, systemd):
    _discarded_metadata(binary, systemd, True)
