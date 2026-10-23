"""Generated, genuinely decodable media and a real headless mpv process."""
import json
import os
from pathlib import Path
import socket
import shutil
import struct
import subprocess
import time


def moov_range(path):
    with path.open('rb') as file:
        size = path.stat().st_size
        while file.tell() < size:
            at = file.tell(); length, kind = struct.unpack('>I4s', file.read(8))
            if length == 1: length = struct.unpack('>Q', file.read(8))[0]
            if length == 0: length = size-at
            assert 8 <= length <= size-at
            if kind == b'moov': return at, at+length
            file.seek(at+length)
    raise AssertionError('fixture has no moov')


def generate(box):
    mp4 = box.root / 'Reality.H264.end-moov.mp4'
    mkv = box.root / 'Reality.H264.mkv'
    subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
        'testsrc2=size=640x360:rate=24', '-f', 'lavfi', '-i',
        'sine=frequency=440:sample_rate=48000', '-t', '150', '-c:v', 'libx264',
        '-preset', 'ultrafast', '-threads', '2', '-b:v', '14M', '-minrate', '14M',
        '-maxrate', '14M', '-bufsize', '28M', '-x264-params',
        'nal-hrd=cbr:force-cfr=1:keyint=48', '-pix_fmt', 'yuv420p', '-c:a', 'aac',
        '-b:a', '128k', str(mp4)], env=box.env, check=True, timeout=90)
    subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(mp4), '-map', '0',
        '-c', 'copy', str(mkv)], env=box.env, check=True, timeout=30)
    boxes = {}
    with mp4.open('rb') as f:
        while f.tell() < mp4.stat().st_size:
            at = f.tell(); size, kind = struct.unpack('>I4s', f.read(8))
            if size == 1: size = struct.unpack('>Q', f.read(8))[0]
            assert size >= 8
            boxes[kind] = at; f.seek(at + size)
    assert boxes[b'moov'] > boxes[b'mdat'], boxes
    for path in (mp4, mkv):
        probe = json.loads(subprocess.check_output(['ffprobe', '-v', 'error',
            '-show_entries', 'format=duration,size:stream=codec_name', '-of', 'json', str(path)], env=box.env))
        assert {'h264', 'aac'} <= {s['codec_name'] for s in probe['streams']}, probe
        assert 200_000_000 < path.stat().st_size < 350_000_000
        assert 149 <= float(probe['format']['duration']) <= 151
    return {'mp4': mp4, 'mkv': mkv}


def keyframe(path, seconds, env):
    probe = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-select_streams',
        'v:0', '-show_packets', '-show_entries', 'packet=pts_time,pos,flags', '-of', 'json', str(path)], env=env))
    return max((float(p['pts_time']), int(p['pos'])) for p in probe['packets']
               if 'K' in p['flags'] and float(p['pts_time']) <= seconds)


class Player:
    def __init__(self, case, box):
        self.case, self.box = case, box
        self.ipc = box.root / 'mpv.sock'
        self.log = case.path.with_suffix('.mpv.log')
        self.errors = case.path.with_suffix('.mpv.stderr')
        self.stream = self.errors.open('w')
        self.child = None; self.connection = None; self.file = None
        self.pid = None
        self.events = []; self.request = 0
        player = shutil.which('mpv')
        if not player:
            raise RuntimeError('real headless playback requires mpv')
        self.argv = [str(Path(player).resolve()), '--no-config', '--hwdec=no', '--vo=null', '--ao=null',
            '--idle=yes', '--keep-open=no', '--cache=yes', '--cache-secs=4',
            '--demuxer-max-bytes=8MiB', '--terminal=no', '--log-file=' + str(self.log),
            '--input-ipc-server=' + str(self.ipc)]

    def attach(self, owner=None):
        deadline = time.monotonic() + 12
        while not self.ipc.exists():
            if owner: assert owner.poll() is None, self.errors.read_text()
            assert time.monotonic() < deadline, 'the real open action never started mpv'
            time.sleep(.02)
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.settimeout(3); self.connection.connect(str(self.ipc))
        self.file = self.connection.makefile('rwb', buffering=0)
        self.pid = self.get('pid')
        assert self.pid and Path(f'/proc/{self.pid}/exe').resolve().name == 'mpv'

    def command(self, *args):
        self.request += 1
        self.file.write((json.dumps(dict(command=list(args), request_id=self.request)) + '\n').encode())
        while True:
            line = self.file.readline()
            if not line: raise EOFError('real mpv IPC closed')
            value = json.loads(line)
            if 'event' in value:
                assert len(self.events) < 4096, 'unbounded mpv event stream'
                self.events.append(value)
            if value.get('request_id') == self.request: return value

    def get(self, name): return self.command('get_property', name).get('data')

    def diagnostics(self):
        body = self.log.read_text(errors='replace') if self.log.exists() else ''
        return [line for line in body.splitlines() if '][e]' in line
                or any(word in line.lower() for word in ('error while decoding', 'invalid nal', 'corrupt', 'partial file'))]

    def eof(self): return any(e.get('event') == 'end-file' and e.get('reason') == 'eof' for e in self.events)

    def close(self):
        if self.file:
            try: self.file.write(b'{"command":["quit"]}\n')
            except OSError: pass
            self.file.close(); self.connection.close()
            self.file = None; self.connection = None
        if self.child:
            try: self.child.wait(timeout=5)
            except subprocess.TimeoutExpired: self.child.kill(); self.child.wait()
        elif self.pid:
            deadline = time.monotonic() + 5
            while Path(f'/proc/{self.pid}').exists():
                # A shell-owned player can briefly await its own parent's reap.
                status = Path(f'/proc/{self.pid}/stat')
                try:
                    if status.read_text().rsplit(')', 1)[1].split()[0] == 'Z': break
                except FileNotFoundError: break
                assert time.monotonic() < deadline, 'the real player did not exit after quit'
                time.sleep(.02)
        self.stream.close()
