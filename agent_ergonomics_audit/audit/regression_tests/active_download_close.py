#!/usr/bin/env python3
"""Quit the real native TUI during a private download, then resume exact bytes.

Uses the benchmark's App and Seed with disposable XDG state and loopback sockets.
This is a lifecycle check, separate from measured performance runs.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import socket
import tempfile
import time

from engine_benchmark import App, Seed


def listener_refuses(port):
    try:
        connection = socket.create_connection(('127.0.0.1', port), timeout=1)
    except ConnectionRefusedError:
        return
    connection.close()
    raise AssertionError(f'closed app still accepts peers on its owned port {port}')


def file_snapshot(path):
    return {'size': path.stat().st_size,
            'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--ratings', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--mib', type=int, default=32)
    parser.add_argument('--rate-mib', type=float, default=4)
    args = parser.parse_args()
    assert args.mib > 0 and args.rate_mib > 0, 'a bounded positive transfer is required'
    args.binary = args.binary.resolve()
    result = {'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'binary': str(args.binary),
              'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'payload_mib': args.mib, 'seed_limit_mib_s': args.rate_mib}
    with tempfile.TemporaryDirectory(prefix='fbittorrent-active-download-close-') as temporary:
        root = Path(temporary)
        ratings = root/'title.ratings.tsv'
        shutil.copyfile(args.ratings, ratings)
        run_root = root/'native'
        run_root.mkdir()
        seed = Seed(root, args.mib, args.rate_mib)
        app = App(run_root, 'native', args, ratings)
        try:
            result['opening_ms'] = app.start()
            # Prove the owned port accepts peers before testing its removal.
            with socket.create_connection(('127.0.0.1', app.peer_port), timeout=1):
                pass
            seed.label = 'partial'
            app.cli('add', str(seed.torrent), '--quiet')
            partial = None

            def has_partial_payload():
                nonlocal partial
                torrents = app.request('torrents/info')
                if len(torrents) == 1 and torrents[0]['progress'] > 0:
                    partial = torrents[0]
                    return True
                return False

            app.wait_for(has_partial_payload)
            assert 0 < partial['progress'] < 1, ('fixture completed before close', partial)
            result['progress_at_close'] = partial['progress']
            pid = app.process.pid
            result['close'] = app.close()
            assert app.process.returncode == 0
            assert not Path(f'/proc/{pid}').exists(), 'downloader process survived quit'
            listener_refuses(app.peer_port)
            # A joined peer receipt while Seed remains live proves app-side closure.
            receipts = seed.wait_for_receipts('partial')
            sent = sum(receipt['bytes_sent'] for receipt in receipts)
            assert 0 < sent < len(seed.data), ('transfer was not active when closed', receipts)
            assert not seed.stop.is_set()
            result['partial_seed_receipts'] = receipts
            target = run_root/'downloads/benchmark.bin'
            before = file_snapshot(target)
            assert before['sha256'] != seed.sha256, 'closed partial file already contains the full payload'
            time.sleep(.5)
            after = file_snapshot(target)
            assert after == before, ('closed downloader continued changing its payload', before, after)
            result['closed_file_snapshot'] = after
            result['closed_stability_seconds'] = .5

            seed.label = 'resume'
            result['reopen_ms'] = app.start()
            app.wait_for(lambda: (items := app.request('torrents/info')) and
                         items[0]['progress'] >= 1 and not items[0]['state'].startswith('checking'))
            completed = file_snapshot(target)
            assert completed == {'size': len(seed.data), 'sha256': seed.sha256}, completed
            result['completed_file_snapshot'] = completed
            result['reclose'] = app.close()
            assert app.process.returncode == 0
            listener_refuses(app.peer_port)
            result['resume_seed_receipts'] = seed.wait_for_receipts('resume')
            result['stderr'] = (run_root/'tui-stderr').read_text()
            assert not result['stderr'], result['stderr']
            result['payload_sha256'] = seed.sha256
            result['passed'] = True
        finally:
            try:
                app.cleanup()
            finally:
                seed.close()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
