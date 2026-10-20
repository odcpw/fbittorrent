#!/usr/bin/env python3
"""Magnet -> real review/Download -> partial video -> real mpv to EOF.

Requires ffmpeg, mpv and the private headless Qt harness. Generates two ~265 MB
H.264/AAC movies; all swarm traffic stays in a loopback-only network namespace.
"""
import argparse
import base64
from contextlib import contextmanager
import errno
import hashlib
import http.client
import json
import os
from pathlib import Path
import shlex
import shutil
import socket
import subprocess
import sys
import time
import tomllib
import urllib.parse

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2] / 'plugin/tests'))
import daemon as d
import fixtures
import fbtlog
import private_net
from dashboard import Session
from add_sheet import wait_sheet, state
from watch_media import generate, keyframe, moov_range, Player
from tui_smoke import PaneUi

MIB = fixtures.MIB


def row(client, info_hash): return client.call('torrents.get', dict(hash=info_hash))['result']['torrent']

def have(snapshot, piece):
    data = base64.b64decode(snapshot['have'])
    return bool(data[piece // 8] & (0x80 >> (piece % 8)))


@contextmanager
def swarm(case, binary, source, box, player, piece_length=MIB, seed_rate=4*MIB):
    config = Path(box.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml'
    config.write_text(config.read_text() + '\nopen_command = ' + json.dumps(shlex.join(player.argv)) + '\n')
    with fixtures.Sandbox() as seedbox, d.Harness(case, binary, box, False) as h, d.Harness(case, binary, seedbox, False) as seed:
        process = h.start_serve(idle_exit_secs=0); sp = seed.start_serve(idle_exit_secs=0)
        assert h.receive_ready() and h.wait_engine() and seed.receive_ready() and seed.wait_engine()
        with d.ApiConn(h) as client, d.ApiConn(seed) as peer:
            client.hello(); peer.hello()
            port = tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml').read_text())['peer_port']
            peers = socket.inet_aton('127.0.0.1') + port.to_bytes(2, 'big')
            response = lambda data: dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1, peers=data))).decode())
            with fixtures.HttpFixture(seedbox.root, {'/announce': response(peers), '/seed': response(b'')}) as tracker:
                payload = seedbox.downloads / source.name
                try: os.link(source, payload)
                except OSError as error:
                    if error.errno != errno.EXDEV: raise
                    shutil.copyfile(source, payload)
                item = fixtures.torrent_fixture(seedbox.root, [payload], tracker=tracker.base + '/announce', piece_length=piece_length)
                item.info['private'] = 0; item.info_hash = hashlib.sha1(fixtures.bencode(item.info)).hexdigest()
                item.torrent.write_bytes(fixtures.bencode(dict(info=item.info, announce=tracker.base + '/seed')))
                peer.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(seedbox.downloads), seed=True))
                assert seed.wait_for(lambda: ((r := row(peer, item.info_hash))['state'] == 'seeding', json.dumps(r)),
                                     'real seed verifies every video piece', timeout=30)
                peer.call('transfer.set_limits', dict(upload_bps=seed_rate))
                magnet = 'magnet:?' + urllib.parse.urlencode(dict(xt='urn:btih:' + item.info_hash, dn=payload.name, tr=tracker.base + '/announce'))
                yield h, process, client, peer, item, magnet


def wait_player(player, predicate, timeout, message):
    deadline = time.monotonic() + timeout
    while True:
        if predicate(): return
        assert not player.diagnostics(), player.diagnostics()[-10:]
        assert time.monotonic() < deadline, message
        time.sleep(.1)


def run(binary, source, name):
    with fbtlog.Case('watch_player', name) as case, fixtures.Sandbox() as box:
        player = Player(case, box)
        tail = name == 'mp4_tail'
        original = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries',
            'format=duration:stream=codec_name', '-of', 'json', str(source)], env=box.env))
        duration = float(original['format']['duration'])
        if tail: player.argv.append('--speed=4')  # Decode every frame of the 360 s audit file in 90 s.
        with swarm(case, binary, source, box, player, 16_384 if tail else MIB, (2 if tail else 4)*MIB) as (h, process, client, peer, item, magnet):
            q = Session(case, box)
            ui = None; opener = None
            try:
                q.wait(lambda: q.ipc('ready') == 'true')
                q.shell('summon', json.dumps(dict(view='torrents', add=dict(source=dict(magnet=magnet)))))
                preview = wait_sheet(q, 'ready'); staged = row(client, item.info_hash)
                assert case.check(preview['hash'] == item.info_hash and staged['staged'] and not list(box.downloads.iterdir()),
                                  'magnet review resolves real peer metadata without downloading payload', json.dumps(preview))
                q.keys(['-k', 'Return'], lambda: not state(q).get('visible'))
                current = q.wait(lambda: r if (r := row(client, item.info_hash)).get('playable', {}).get('ready') else None, timeout=90)
                assert case.check(current['completed_bytes'] < current['size'] and current['state'] == 'downloading'
                    and not current['upload_enabled'] and current['uploaded_bytes'] == 0,
                    'Ready to watch comes from verified pieces while the video is still downloading', json.dumps(current))
                if tail:
                    start, end = moov_range(source)
                    pieces = client.call('torrents.pieces', dict(hash=item.info_hash))['result']
                    metadata = range(start//pieces['piece_size'], (end-1)//pieces['piece_size']+1)
                    assert case.check(len(metadata) >= 2 and all(have(pieces, i) for i in metadata)
                        and current['completed_bytes'] < current['size']*.75
                        and not have(pieces, (start//pieces['piece_size'])//2),
                        'the entire multi-piece tail moov is verified early, ahead of missing media',
                        json.dumps(dict(moov=[start, end], pieces=list(metadata), row=current)))
                target = client.call('torrents.open_target', dict(hash=item.info_hash))['result']
                assert case.check(bool(target.get('stream_url')) and Path(target['path']).name == source.name,
                                  'incomplete video has a verified streaming target and its real file path', json.dumps(target))
                url = urllib.parse.urlsplit(target['stream_url'])
                assert url.hostname == '127.0.0.1' and url.scheme == 'http'
                if tail:
                    probe = subprocess.run(['ffprobe', '-v', 'error', '-show_entries',
                        'format=duration:stream=codec_name', '-of', 'json', target['path']],
                        env=box.env, capture_output=True, text=True, timeout=10)
                    partial = json.loads(probe.stdout) if probe.returncode == 0 else {}
                    assert case.check(probe.returncode == 0 and not probe.stderr
                        and partial.get('streams') == original['streams']
                        and partial.get('format', {}).get('duration') == original['format']['duration'],
                        'ffprobe reads valid metadata from the still-incomplete payload', probe.stdout+probe.stderr)
                # Negative privacy controls use the real listener, not a parser substitute.
                for path, headers, status in [(url.path[:-1] + ('0' if url.path[-1] != '0' else '1'), {}, 404),
                    (url.path, {'Origin': 'https://outside.invalid'}, 403), (url.path, {'Host': 'outside.invalid'}, 403)]:
                    connection = http.client.HTTPConnection(url.hostname, url.port, timeout=3)
                    try:
                        connection.request('HEAD', path, headers=headers)
                        response = connection.getresponse()
                        assert case.check(response.status == status, 'media rejects forged capability or cross-origin access', str(response.status))
                    finally: connection.close()
                if name.endswith('stop_reader'):
                    client.call('torrents.stop', dict(hashes=[item.info_hash]))
                    connection = http.client.HTTPConnection(url.hostname, url.port, timeout=.5)
                    try:
                        connection.request('GET', url.path, headers={'Range': 'bytes=190000000-'})
                        response = connection.getresponse()
                        assert response.status == 206
                        try:
                            response.read(1)
                            raise AssertionError('an unfetched paused piece was returned')
                        except TimeoutError: pass
                        began = time.monotonic()
                        client.call('app.shutdown', dict(reason='user'))
                        assert h.wait_for(lambda: (process.child.poll() == 0, str(process.child.poll())),
                                          'shutdown drains a real blocked media reader', timeout=4)
                        assert case.check(time.monotonic()-began < 4, 'blocked playback cannot overrun the daemon stop budget',
                                          json.dumps(dict(seconds=time.monotonic()-began, stderr=h.stderr())))
                        probe = socket.socket(); probe.settimeout(1)
                        try: assert probe.connect_ex((url.hostname, url.port)) != 0
                        finally: probe.close()
                    finally: connection.close()
                    return 0
                # Production dashboard Open uses the owner's configured command.
                q.shell('summon', json.dumps(dict(view='torrents', hash=item.info_hash)))
                q.keys(['-k', 'Return'], lambda: 'Ready to watch' in q.ipc('detailTexts'))
                image = case.path.with_suffix('.ready.png'); q.ipc('grab', image)
                q.wait(lambda: image.exists() and image.stat().st_size > 0)
                assert case.check('Ready to watch' in q.ipc('detailTexts'),
                                  'the real details pane visibly offers Ready to watch before opening', str(image))
                q.keys(['-k', 'Escape'], lambda: q.state().get('view', {}).get('focus') == 'table')
                if name.endswith('seek'):
                    ui = PaneUi(case, str(binary), box, h, session='fbt-watch-seek')
                    ui.start_owned()
                    assert ui.observe('wait for the real video row', 'the TUI selects the downloading video',
                                      lambda pane: source.name in pane)
                    assert ui.press('o', 'TUI Open launches the configured real mpv', lambda pane: player.ipc.exists())
                elif name.endswith('cli'):
                    opener = case.popen([str(binary), '--no-start', 'open', item.info_hash, '--json'],
                        env=box.env, stdout=subprocess.PIPE, text=True)
                    assert opener.wait(timeout=5) == 0
                else:
                    q.keys(['o'], lambda: player.ipc.exists())
                player.attach()
                if opener:
                    output, _ = opener.communicate(timeout=1)
                    assert case.check(json.loads(output)['ok'], 'CLI Open returns its JSON without waiting for the player', output)
                wait_player(player, lambda: (player.get('time-pos') or 0) > .2, 12, 'mpv did not decode/start playback')
                opened = row(client, item.info_hash)
                assert case.check(opened['completed_bytes'] < opened['size'],
                    'the real player decodes while payload pieces are still missing', json.dumps(opened))
                if ui:
                    assert ui.press('q', 'closing the TUI leaves daemon-owned playback running', lambda pane: ui.done.exists())
                    assert process.child.poll() is None
                samples = []; began = time.monotonic(); longest_gap = 0; last_time = player.get('time-pos'); last_advance = began
                if name.endswith('seek'):
                    seconds, offset = keyframe(source, 110, box.env)
                    pieces = client.call('torrents.pieces', dict(hash=item.info_hash))['result']
                    sought = offset // pieces['piece_size']; gap = (sought + current['playable']['prefix_bytes']//pieces['piece_size'])//2
                    assert case.check(not have(pieces, sought) and not have(pieces, gap), 'seek targets real unfetched pieces',
                                      json.dumps(dict(seconds=seconds, piece=sought, gap=gap, pieces=pieces)))
                    player.command('seek', 110, 'absolute+exact')
                    wait_player(player, lambda: (player.get('time-pos') or 0) > 111, 20, 'seek never resumed valid playback')
                    after = client.call('torrents.pieces', dict(hash=item.info_hash))['result']
                    assert case.check(have(after, sought) and not have(after, gap),
                        'seek pieces arrive ahead of the intervening sequential gap', json.dumps(after))
                    last_time = player.get('time-pos'); last_advance = time.monotonic()
                if name.endswith('pause_slow'):
                    client.call('torrents.stop', dict(hashes=[item.info_hash]))
                    stopped = row(client, item.info_hash)
                    assert stopped['state'] == 'paused', stopped
                    wait_player(player, lambda: player.get('paused-for-cache') is True, 50,
                                'paused download did not eventually exhaust the real player buffer')
                    assert case.check(not player.eof() and not player.diagnostics(),
                        'pause buffers at missing pieces rather than decoding sparse zeros or ending early', json.dumps(player.events[-10:]))
                    peer.call('transfer.set_limits', dict(upload_bps=MIB))
                    client.call('torrents.start', dict(hashes=[item.info_hash]))
                    assert h.wait_for(lambda: ((r := row(client, item.info_hash)).get('playable', {}).get('stall_free') is False, json.dumps(r)),
                                      'slow peer is truthfully marked may-stall', timeout=15)
                    frozen = player.get('time-pos') or 0
                    slow_began = time.monotonic(); slow_before = row(client, item.info_hash)['completed_bytes']
                    wait_player(player, lambda: (player.get('time-pos') or 0) > frozen + 2, 18, 'resume did not wake the buffered reader')
                    while time.monotonic()-slow_began < 12:
                        player.get('time-pos')
                        assert not player.diagnostics(), player.diagnostics()[-10:]
                        time.sleep(.2)
                    slow_seconds = time.monotonic()-slow_began
                    received = row(client, item.info_hash)['completed_bytes']-slow_before
                    assert case.check(2*MIB <= received <= slow_seconds*MIB+2*MIB,
                        'the slow peer supplies actual verified bytes within its measured cap',
                        json.dumps(dict(bytes=received, seconds=slow_seconds, bps=received/slow_seconds)))
                    peer.call('transfer.set_limits', dict(upload_bps=4*MIB))
                    last_time = player.get('time-pos'); last_advance = time.monotonic()
                while not player.eof():
                    position = player.get('time-pos'); buffering = player.get('paused-for-cache')
                    now = time.monotonic()
                    assert q.compositor.child.poll() is None, f'private compositor exited {q.compositor.child.returncode}'
                    assert q.child.poll() is None, f'private dashboard exited {q.child.returncode}'
                    if position is not None and (last_time is None or position > last_time + .01):
                        longest_gap = max(longest_gap, now-last_advance); last_advance = now; last_time = position
                    samples.append(dict(wall=now-began, position=position, buffering=buffering))
                    assert not player.diagnostics(), player.diagnostics()[-10:]
                    assert now-began < duration+90, 'real playback did not reach EOF within its rate-aware fuse'
                    if not name.endswith('pause_slow'):
                        assert now-last_advance < 3, 'playback stalled despite the faster-than-media peer and verified buffer'
                    time.sleep(.2)
                playing_wall = time.monotonic()-began
                assert h.wait_for(lambda: ((r := row(client, item.info_hash))['completed_bytes'] == r['size'], json.dumps(r)),
                                  'remaining unplayed pieces finish and can be compared to the original video', timeout=90)
                final = row(client, item.info_hash)
                def digest(path):
                    with path.open('rb') as file: return hashlib.file_digest(file, 'sha256').hexdigest()
                assert case.check(not player.diagnostics() and last_time is not None and last_time >= duration-1
                    and final['completed_bytes'] == final['size']
                    and final['uploaded_bytes'] == 0 and not final['upload_enabled']
                    and digest(source) == digest(Path(target['path'])),
                    'real player reaches EOF without decode errors; complete payload matches and upload remains off',
                    json.dumps(dict(final=final, final_video_time=last_time, events=player.events[-10:], longest_gap=longest_gap,
                                    playing_wall=playing_wall, wall=time.monotonic()-began)))
                case.path.with_suffix('.player.json').write_text(json.dumps(dict(samples=samples, events=player.events), indent=2))
            finally:
                if player.connection is None and player.ipc.exists(): player.attach()
                player.close()
                if opener: opener.communicate(timeout=2)
                if ui: ui.close()
                q.close()
    return fbtlog.suite_status('watch_player')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--case', choices=['mp4_play', 'mp4_seek', 'mp4_cli', 'mkv_pause_slow', 'mp4_stop_reader', 'mp4_tail'], action='append')
    parser.add_argument('--log-root', type=Path)
    parser.add_argument('--generated-media', type=Path, help='reuse only a private /tmp/fbt-watch-media-* generation')
    parser.add_argument('--tail-source', type=Path, help='read-only real audit MP4 for the 16 KiB tail-metadata case')
    args = parser.parse_args(); private_net.enter()
    if args.log_root: fbtlog.LOG_ROOT = args.log_root.resolve()
    with fixtures.Sandbox() as media_box:
        if args.generated_media:
            root = args.generated_media.resolve()
            assert root.parent == Path('/tmp') and root.name.startswith('fbt-watch-media-')
            media = {'mp4': root/'Reality.H264.end-moov.mp4', 'mkv': root/'Reality.H264.mkv'}
        else: media = generate(media_box)
        for name in args.case or ['mp4_play', 'mp4_seek', 'mkv_pause_slow', 'mp4_stop_reader']:
            source = args.tail_source if name == 'mp4_tail' and args.tail_source else media[name.split('_')[0]]
            if run(args.binary.resolve(), source, name): return 1
    return fbtlog.suite_status('watch_player')


if __name__ == '__main__': sys.exit(main())
