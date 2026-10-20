"""Private, decodable faststart/tail MP4s over real loopback torrent engines."""
import json
import os
import struct
import subprocess

from daemon_watch import MIB, history, pair, row


def command(case, box, argv, success=True):
    child = case.popen(argv, env=box.env, cwd=box.root, stdout=subprocess.PIPE)
    out, _ = child.communicate(timeout=30)
    case.check((child.returncode == 0) == success,
               ('succeeds: ' if success else 'rejects missing movie metadata: ') + ' '.join(map(str, argv)),
               json.dumps(dict(code=child.returncode, stdout=out.decode(errors='replace'))))
    return out


def free(output, size):
    assert size >= 8
    output.write(struct.pack('>I4s', size, b'free'))
    left = size - 8
    while left:
        block = os.urandom(min(MIB, left)); output.write(block); left -= len(block)


def writer(faststart):
    def generate(case, box, path, size):
        source = path.with_suffix('.source.mp4')
        argv = ['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
                'testsrc2=size=160x90:rate=12', '-t', '4', '-c:v', 'mpeg4', '-threads', '1']
        if faststart: argv += ['-movflags', '+faststart']
        command(case, box, [*argv, str(source)])
        data = source.read_bytes(); at = 0; boxes = {}
        while at < len(data):
            length, kind = struct.unpack('>I4s', data[at:at+8]); assert length >= 8
            boxes[kind] = (at, length); at += length
        moov, length = boxes[b'moov']
        case.check((moov < boxes[b'mdat'][0]) == faststart, 'generated video has the requested real box layout', repr(boxes))
        with path.open('wb') as output:
            if faststart:
                output.write(data)
            else:
                # Keep the original mdat/chunk offsets. A legal top-level free
                # box moves moov behind a missing range. A free child makes
                # moov span >2 pieces, so its last piece alone cannot suffice.
                output.write(data[:moov]); free(output, 40*MIB-output.tell())
                output.write(struct.pack('>I4s', length+2*MIB, b'moov'))
                output.write(data[moov+8:moov+length]); free(output, 2*MIB)
                output.write(data[moov+length:])
            free(output, size-output.tell())
        command(case, box, ['ffmpeg', '-nostdin', '-v', 'error', '-i', str(path), '-frames:v', '1', '-f', 'null', '-'])
    return generate


def journey(name, binary, systemd, faststart):
    with pair(name, binary, systemd, extension='mp4', prepared_prefix=32*MIB,
              payload_writer=writer(faststart), stopped=True) as (case, box, h, p, client, peer, item, payload):
        target = box.downloads / payload.name
        captured = []
        def inspected():
            value = row(client, item.info_hash); captured[:] = [value]
            return 'playable' in value, json.dumps(value)
        assert h.wait_for(inspected, 'paused initial check exposes verified playback metadata', timeout=15)
        value = captured[0]
        case.check(value['state'] == 'paused' and value['completed_bytes'] == 33*MIB
                   and value['playable']['prefix_bytes'] == 32*MIB,
                   'real verification sees exactly the 32 MiB prefix and final 1 MiB piece', json.dumps(value))
        case.check(value['playable']['ready'] == faststart,
                   'front moov is ready; tail moov outside prefix/final piece stays unready', json.dumps(value))
        probe = ['ffprobe', '-v', 'error', '-show_entries', 'format=duration', '-of', 'json', str(target)]
        command(case, box, probe, success=faststart)
        if not faststart:
            case.check(not any(e['ev'] == 'torrent.playable' for e in history(client)),
                       'missing moov never produces a premature playable notification', json.dumps(history(client)))
            peer.call('transfer.set_limits', dict(upload_bps=2*MIB))
            client.call('torrents.start', dict(hashes=[item.info_hash]))
            def ready():
                value = row(client, item.info_hash); captured[:] = [value]
                return value.get('playable', {}).get('ready') is True, json.dumps(value)
            assert h.wait_for(ready, 'loopback transfer fills every piece of tail moov before declaring ready', timeout=30)
            value = captured[0]
        case.check(value['completed_bytes'] < value['size'] and value['playable'].get('buffered_s', 0) >= 2,
                   'both layouts are ready with a measured buffer before full download', json.dumps(value))
        files = client.call('torrents.files', dict(hash=item.info_hash))['result']['files']
        case.check(files[0]['ready'] is True, 'the shared Files readiness follows verified moov', json.dumps(files))
        command(case, box, ['ffmpeg', '-nostdin', '-v', 'error', '-i', str(target), '-frames:v', '1', '-f', 'null', '-'])
        events = history(client)
        case.check(sum(e['ev'] == 'torrent.playable' and e.get('hash') == item.info_hash for e in events) == 1,
                   'one truthful readiness notification for the incomplete decodable MP4', json.dumps(events))


def playable_mp4_faststart(binary, systemd):
    journey('playable_mp4_faststart', binary, systemd, True)


def playable_mp4_tail_moov(binary, systemd):
    journey('playable_mp4_tail_moov', binary, systemd, False)


CASES = (playable_mp4_faststart, playable_mp4_tail_moov)
