"""A large file count must not hide the whole engine behind payload fsync."""
import json
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
