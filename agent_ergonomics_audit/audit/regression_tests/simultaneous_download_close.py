#!/usr/bin/env python3
"""Exercise three real native downloads, status, timed refresh, quit and resume.

Three private seeds supply distinct 32/48/64 MiB files at 4 MiB/s each. This
checks forward progress and a real maintenance timer side effect under load;
it does not measure scheduler equity, maximum throughput or response tails.
All application state and connections belong to this disposable fixture.
"""
import argparse
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import shutil
import socket
import tempfile
import time

from active_download_close import file_snapshot, listener_refuses
from engine_benchmark import App, Seed


NAMES = ('fairness-alpha.bin', 'fairness-beta.bin', 'fairness-gamma.bin')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--ratings', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sizes-mib', type=int, nargs=3, default=(32, 48, 64))
    parser.add_argument('--rate-mib', type=float, default=4)
    parser.add_argument('--observe-seconds', type=float, default=3,
                        help='observe all three partial transfers through more than one 2s TUI poll period')
    parser.add_argument('--timeout-seconds', type=float, default=60)
    args = parser.parse_args()
    assert all(size > 0 for size in args.sizes_mib) and args.rate_mib > 0
    assert args.observe_seconds > 2 and args.timeout_seconds > args.observe_seconds
    assert args.sizes_mib[0] < min(args.sizes_mib[1:]), 'first file must finish while others remain partial'
    args.binary = args.binary.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    result = {'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'binary': str(args.binary),
              'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'seed_limit_mib_s_each': args.rate_mib, 'partial_samples': [], 'resume_samples': [],
              'resume_startup_samples': [], 'passed': False,
              'timer_observers': {
                  'tui': {'source': 'src/tui/mod.rs:1672', 'callback': 'src/tui/mod.rs:353',
                          'observable': 'periodic real PTY output changes during three advancing partial downloads',
                          'limit': 'output activity also includes the 300ms Hover subscription; bytes alone cannot attribute each draw to Poll'},
                  'native': {'source': 'src/engine.rs:473', 'side_effect': 'src/engine.rs:301',
                             'observable': 'first file has persisted completion_on and stoppedUP while another is still partial',
                             'read_only_observation_routes': ['torrents/info', 'app/queue', 'status CLI']}},
              'limits': 'Three distinct actual TCP seeds and torrent hashes; checks collective forward progress, '
                        'responsive status and timer completion handling. No claim of equal scheduling shares, '
                        'strict response latency, encrypted peers, Internet throughput or physical-disk behavior.'}
    with tempfile.TemporaryDirectory(prefix='fbittorrent-simultaneous-close-') as temporary, ExitStack() as cleanup:
        root = Path(temporary)
        ratings = root/'title.ratings.tsv'
        shutil.copyfile(args.ratings, ratings)
        seeds = []
        for filename, size in zip(NAMES, args.sizes_mib):
            seed = Seed(root, size, args.rate_mib, filename=filename)
            cleanup.callback(seed.close)
            seed.label = 'partial'
            seeds.append(seed)
        hashes = [seed.info_hash.hex() for seed in seeds]
        assert len(set(hashes)) == 3
        result['fixtures'] = [{'filename': seed.filename, 'bytes': len(seed.data),
                               'info_hash': seed.info_hash.hex(), 'payload_sha256': seed.sha256,
                               'torrent_sha256': hashlib.sha256(seed.torrent.read_bytes()).hexdigest()}
                              for seed in seeds]
        run_root = root/'native'
        run_root.mkdir()
        app = App(run_root, 'native', args, ratings, capture_output=True)
        cleanup.callback(app.cleanup)
        negative = run_root/'cache/fbittorrent/imdb/by-name-v2'
        for filename in NAMES:
            key = ''.join(char.lower() if char.isascii() and char.isalnum() else '_'
                          for char in filename)[:120]
            (negative/(key+'.json')).write_text('{"missing":true}')
        ledger_path = run_root/'data/fbittorrent/engine/torrents.json'

        def observe(phase):
            began = time.monotonic()
            torrents = app.request('torrents/info')
            assert {torrent['hash'] for torrent in torrents} == set(hashes), torrents
            queue = app.request('app/queue')
            status_started = time.monotonic()
            status = app.cli('status')
            ended = time.monotonic()
            assert status['ok'] and status['engine'] == 'native' and status['total'] == 3, status
            assert queue['max_active_downloads'] == 3 and not queue['network_blocked'], queue
            row = {'started_monotonic': began, 'observed_monotonic': ended,
                   'status_started_monotonic': status_started,
                   'status_cli_ms': (ended-status_started)*1000,
                   'status': status, 'queue': queue, 'torrents': torrents,
                   'tty_output_bytes': app.output_bytes}
            result[phase+'_samples'].append(row)
            return row, {torrent['hash']: torrent for torrent in torrents}

        def all_partial(torrents):
            return all(0 < torrent['completed'] < torrent['size'] and 0 < torrent['progress'] < 1
                       for torrent in torrents.values())

        def save_terminal(phase):
            content = bytes(app.terminal_output)
            path = args.output.with_name(args.output.stem+f'-{phase}-terminal.ansi.txt')
            path.write_bytes(content)
            result[phase+'_terminal'] = {'path': str(path.resolve()), 'bytes': len(content),
                                         'sha256': hashlib.sha256(content).hexdigest(),
                                         'writes': list(app.terminal_writes),
                                         'literal_name_markers_seen': {name: name.encode() in content for name in NAMES}}

        result['opening_ms'] = app.start()
        with socket.create_connection(('127.0.0.1', app.peer_port), timeout=1):
            pass
        for seed in seeds:
            started = time.monotonic()
            app.cli('add', str(seed.torrent), '--quiet')
            result.setdefault('add_receipts', []).append({'info_hash': seed.info_hash.hex(),
                                                         'started_monotonic': started,
                                                         'finished_monotonic': time.monotonic()})
        deadline = time.monotonic()+args.timeout_seconds
        first = None
        tty_changes = 0
        previous_tty_bytes = app.output_bytes
        while time.monotonic() < deadline:
            row, torrents = observe('partial')
            if first is None:
                if all_partial(torrents) and row['queue']['active_downloads'] == 3:
                    first = row
                    previous_tty_bytes = row['tty_output_bytes']
            else:
                assert all_partial(torrents), 'a file finished before the simultaneous partial observer completed'
                assert row['queue']['active_downloads'] == 3
                if row['tty_output_bytes'] > previous_tty_bytes:
                    tty_changes += 1
                previous_tty_bytes = row['tty_output_bytes']
                if row['observed_monotonic']-first['observed_monotonic'] >= args.observe_seconds:
                    initial = {torrent['hash']: torrent for torrent in first['torrents']}
                    for digest in hashes:
                        assert torrents[digest]['completed'] > initial[digest]['completed'], (
                            'one of three active downloads did not advance', digest, initial[digest], torrents[digest])
                    assert tty_changes >= 2, ('real TUI produced no repeated timed updates during active transfers', tty_changes)
                    result['partial_observer'] = {'all_three_advanced': True, 'tty_changes_observed': tty_changes,
                                                   'elapsed_seconds': row['observed_monotonic']-first['observed_monotonic']}
                    break
            time.sleep(.25)
        else:
            raise TimeoutError('three simultaneous progressing partial downloads were not observed')

        result['close_started_monotonic'] = time.monotonic()
        pid = app.process.pid
        result['close'] = app.close()
        assert app.process.returncode == 0 and not Path(f'/proc/{pid}').exists()
        listener_refuses(app.peer_port)
        save_terminal('partial')
        result['partial_seed_receipts'] = {}
        for seed in seeds:
            receipts = seed.wait_for_receipts('partial')
            sent = sum(receipt['bytes_sent'] for receipt in receipts)
            assert not seed.stop.is_set() and 0 < sent < len(seed.data), (seed.filename, receipts)
            result['partial_seed_receipts'][seed.info_hash.hex()] = receipts
        saved = json.loads(ledger_path.read_text())['records']
        assert {record['hash'] for record in saved} == set(hashes)
        assert all(not record['paused'] and record['completion_on'] == 0 for record in saved), saved
        result['closed_running_intent'] = saved
        snapshots = {seed.filename: file_snapshot(run_root/'downloads'/seed.filename) for seed in seeds}
        for seed in seeds:
            assert snapshots[seed.filename]['sha256'] != seed.sha256, ('partial file is already complete', seed.filename)
        time.sleep(.5)
        after = {seed.filename: file_snapshot(run_root/'downloads'/seed.filename) for seed in seeds}
        assert after == snapshots, ('closed downloads continued changing files', snapshots, after)
        result['closed_file_snapshots'] = after
        result['closed_stability_seconds'] = .5

        for seed in seeds:
            seed.label = 'resume'
        result['reopen_ms'] = app.start()
        # API readiness can precede restoration/checking of existing payloads.
        # Arm the timer observer only once all three are restored and partial;
        # retain the real initial states and reject completion before this point.
        restored_started = time.monotonic()
        deadline = restored_started+args.timeout_seconds
        while True:
            row, torrents = observe('resume_startup')
            result.setdefault('resume_initial_snapshot', row)
            completed_before_observer = [torrent for torrent in torrents.values()
                                         if torrent['completion_on'] != 0 or (
                                             torrent['size'] > 0 and (
                                                 torrent['completed'] >= torrent['size'] or
                                                 torrent['progress'] >= 1))]
            if completed_before_observer:
                result['failure'] = 'a resumed file completed before all three restored partial states were visible'
                args.output.write_text(json.dumps(result, indent=2)+'\n')
                raise AssertionError((result['failure'],
                                      'initial_snapshot', result['resume_initial_snapshot'],
                                      'current_snapshot', row))
            if all_partial(torrents) and row['queue']['active_downloads'] == 3:
                result['resume_timer_observer_start'] = {
                    'snapshot': row, 'restoration_wait_seconds': row['observed_monotonic']-restored_started}
                args.output.write_text(json.dumps(result, indent=2)+'\n')
                break
            if time.monotonic() >= deadline:
                result['failure'] = 'three restored partial downloads were not visible before the timeout'
                args.output.write_text(json.dumps(result, indent=2)+'\n')
                raise TimeoutError((result['failure'],
                                    'initial_snapshot', result['resume_initial_snapshot'],
                                    'current_snapshot', row))
            time.sleep(.25)
        deadline = time.monotonic()+args.timeout_seconds
        timer = None
        while time.monotonic() < deadline:
            row, torrents = observe('resume')
            first_torrent = torrents[hashes[0]]
            others_partial = [digest for digest in hashes[1:]
                              if 0 < torrents[digest]['completed'] < torrents[digest]['size']]
            if timer is None and first_torrent['progress'] >= 1 and first_torrent['state'] == 'stoppedUP' and (
                    first_torrent['completion_on'] > 0 and others_partial):
                persisted = {record['hash']: record for record in json.loads(ledger_path.read_text())['records']}
                assert persisted[hashes[0]]['completion_on'] == first_torrent['completion_on']
                assert not persisted[hashes[0]]['paused'], 'automatic completion rewrote running intent'
                timer = {'observed_monotonic': row['observed_monotonic'], 'finished_hash': hashes[0],
                         'other_partial_hashes': others_partial, 'finished_snapshot': first_torrent,
                         'persisted_record': persisted[hashes[0]]}
                result['native_timer_during_payload'] = timer
            if all(torrent['progress'] >= 1 and torrent['completion_on'] > 0 and torrent['state'] == 'stoppedUP'
                   for torrent in torrents.values()):
                break
            time.sleep(.25)
        else:
            raise TimeoutError('resumed transfers or their native completion maintenance did not finish')
        assert timer is not None, 'native completion timer was not observed while another transfer was active'
        result['completed_file_snapshots'] = {}
        for seed in seeds:
            snapshot = file_snapshot(run_root/'downloads'/seed.filename)
            assert snapshot == {'size': len(seed.data), 'sha256': seed.sha256}, (seed.filename, snapshot)
            result['completed_file_snapshots'][seed.filename] = snapshot
        result['reclose'] = app.close()
        assert app.process.returncode == 0 and not Path(f'/proc/{app.process.pid}').exists()
        listener_refuses(app.peer_port)
        save_terminal('resume')
        result['resume_seed_receipts'] = {seed.info_hash.hex(): seed.wait_for_receipts('resume') for seed in seeds}
        assert all(not seed.stop.is_set() for seed in seeds)
        result['stderr'] = (run_root/'tui-stderr').read_text()
        assert not result['stderr'], result['stderr']
        result['passed'] = True
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'passed': True, 'output': str(args.output.resolve()),
                      'fixtures': result['fixtures'], 'partial_observer': result['partial_observer'],
                      'native_timer_observed_monotonic': result['native_timer_during_payload']['observed_monotonic']}))


if __name__ == '__main__':
    main()
