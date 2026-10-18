#!/usr/bin/env python3
"""Paired whole-app benchmarks with a real, private, loopback BitTorrent peer.

Measurement only: every process, config, download and socket is isolated. The
same supplied fbittorrent TUI is used with each backend. An optional saved
baseline adds a third native variant. qBittorrent is never launched through
systemd. No machine tuning or user config writes occur.
"""
import argparse
import fcntl
import hashlib
import http.server
import itertools
import json
import os
from pathlib import Path
import platform
import pty
import random
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import threading
import time
import urllib.parse
import urllib.request

from resources import process_stats


def bencode(value):
    if isinstance(value, int):
        return b'i' + str(value).encode() + b'e'
    if isinstance(value, str):
        value = value.encode()
    if isinstance(value, bytes):
        return str(len(value)).encode() + b':' + value
    if isinstance(value, list):
        return b'l' + b''.join(map(bencode, value)) + b'e'
    return b'd' + b''.join(bencode(k) + bencode(v) for k, v in sorted(value.items())) + b'e'


def recv_exact(connection, count):
    chunks = bytearray()
    while len(chunks) < count:
        chunk = connection.recv(count - len(chunks))
        if not chunk:
            raise EOFError()
        chunks.extend(chunk)
    return bytes(chunks)


def free_port():
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        return probe.getsockname()[1]


class Seed:
    """Common TCP seed; bounded speed measures efficiency, not max throughput."""
    def __init__(self, root, mib, rate, filename='benchmark.bin'):
        assert Path(filename).name == filename and filename not in ('', '.', '..')
        self.filename = filename
        self.data = random.Random(20261010).randbytes(mib * 1024 * 1024)
        self.sha256 = hashlib.sha256(self.data).hexdigest()
        self.piece_size = 256 * 1024
        pieces = b''.join(hashlib.sha1(self.data[i:i+self.piece_size]).digest()
                          for i in range(0, len(self.data), self.piece_size))
        self.info = {'name': filename, 'length': len(self.data),
                     'piece length': self.piece_size, 'pieces': pieces, 'private': 1}
        self.info_hash = hashlib.sha1(bencode(self.info)).digest()
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.listener.settimeout(.2)
        self.port = self.listener.getsockname()[1]
        self.rate = rate * 1024 * 1024
        self.stop = threading.Event()
        self.connections = set()
        self.lock = threading.Lock()
        self.receipts = []
        self.workers = []
        self.workers_by_run = {}
        self.label = None
        seed = self

        class Tracker(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                body = bencode({'interval': 30, 'complete': 1, 'incomplete': 0,
                                'peers': socket.inet_aton('127.0.0.1') + struct.pack('!H', seed.port)})
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.tracker = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Tracker)
        self.tracker.daemon_threads = True
        threading.Thread(target=self.tracker.serve_forever, daemon=True).start()
        self.accept_thread = threading.Thread(target=self.accept, daemon=True)
        self.accept_thread.start()
        self.torrent = root / (Path(filename).stem + '.torrent')
        self.torrent.write_bytes(bencode({'announce': f'http://127.0.0.1:{self.tracker.server_port}/announce',
                                         'info': self.info}))

    def accept(self):
        while not self.stop.is_set():
            try:
                conn, _ = self.listener.accept()
            except (TimeoutError, OSError):
                continue
            with self.lock:
                self.connections.add(conn)
                label = self.label
                worker = threading.Thread(target=self.serve, args=(conn,label), daemon=True)
                self.workers.append(worker)
                self.workers_by_run.setdefault(label, []).append(worker)
                worker.start()

    def serve(self, conn, label):
        sent = 0
        first = last = None
        try:
            conn.settimeout(10)
            conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            hello = recv_exact(conn, 68)
            assert hello[:20] == b'\x13BitTorrent protocol' and hello[28:48] == self.info_hash
            conn.sendall(b'\x13BitTorrent protocol' + bytes(8) + self.info_hash + b'-FB0001-012345678901')
            count = len(self.info['pieces']) // 20
            bitfield = bytes([255]) * (count // 8)
            if count % 8:
                bitfield += bytes([255 << (8-count % 8) & 255])
            conn.sendall(struct.pack('!I', len(bitfield)+1) + b'\x05' + bitfield + b'\x00\x00\x00\x01\x01')
            while not self.stop.is_set():
                length = struct.unpack('!I', recv_exact(conn, 4))[0]
                if not length:
                    continue
                message = recv_exact(conn, length)
                if message[0] != 6:
                    continue
                index, begin, size = struct.unpack('!III', message[1:])
                offset = index*self.piece_size + begin
                assert 0 < size <= 16*1024 and offset+size <= len(self.data)
                if first is None:
                    first = time.monotonic()
                if self.rate:
                    delay = first + sent/self.rate - time.monotonic()
                    if delay > 0:
                        time.sleep(delay)
                block = self.data[offset:offset+size]
                conn.sendall(struct.pack('!I', size+9) + b'\x07' + struct.pack('!II', index, begin) + block)
                sent += size
                last = time.monotonic()
        except (EOFError, ConnectionError, OSError):
            pass
        finally:
            conn.close()
            with self.lock:
                self.connections.discard(conn)
                self.receipts.append({'run': label, 'bytes_sent': sent,
                                      'first_payload_monotonic': first, 'last_payload_monotonic': last,
                                      'closed_monotonic': time.monotonic(),
                                      'transfer_seconds': last-first if last else 0})

    def wait_for_receipts(self, label, timeout=2):
        """Require the peer workers to finish after their owning app has stopped."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                workers = list(self.workers_by_run.get(label, []))
            for worker in workers:
                worker.join(timeout=max(0, deadline-time.monotonic()))
            with self.lock:
                current = self.workers_by_run.get(label, [])
                receipts = [item.copy() for item in self.receipts if item['run'] == label]
                if current and all(not worker.is_alive() for worker in current) and any(
                        item['bytes_sent'] > 0 for item in receipts):
                    return receipts
            time.sleep(.01)
        raise TimeoutError(f'seed peer did not close with the app: {label}')

    def close(self):
        self.stop.set()
        self.listener.close()
        with self.lock:
            for conn in self.connections:
                try:
                    conn.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
        self.tracker.shutdown()
        self.tracker.server_close()
        self.accept_thread.join(timeout=2)
        for worker in self.workers:
            worker.join(timeout=2)
            assert not worker.is_alive(), 'seed worker did not close'


class App:
    def __init__(self, root, engine, args, ratings, binary=None, capture_output=False):
        self.root, self.engine, self.args = root, engine, args
        self.binary = binary if binary is not None else args.binary
        self.env = dict(os.environ, TERM='xterm-256color')
        for kind in ('CONFIG', 'DATA', 'RUNTIME', 'CACHE'):
            key = f'XDG_{kind}_HOME' if kind != 'RUNTIME' else 'XDG_RUNTIME_DIR'
            directory = root / kind.lower()
            directory.mkdir(mode=0o700)
            self.env[key] = str(directory)
        cache = root / 'cache/fbittorrent'
        cache.mkdir()
        (cache / 'title.ratings.tsv').symlink_to(ratings)
        # Prevent unrelated title suggestion/poster traffic for the fixture row.
        negative = cache / 'imdb/by-name-v2'
        negative.mkdir(parents=True)
        (negative / 'benchmark_bin.json').write_text('{"missing":true}')
        self.web_port = free_port()
        self.peer_port = free_port()
        config = root / 'config/fbittorrent'
        config.mkdir()
        (config / 'config.toml').write_text(
            f'engine = "{engine}"\nautostart = false\nnotify = false\n'
            f'url = "http://127.0.0.1:{self.web_port}"\npeer_port = {self.peer_port}\n'
            f'network_interface = "lo"\npeer_encryption = "off"\n'
            f'save_path = {json.dumps(str(root / "downloads"))}\n'
            'backend_unit = "fbittorrent-benchmark-never.service"\n')
        self.process = self.daemon = None
        self.reader = self.master = None
        self.output_bytes = 0
        self.terminal_output = bytearray() if capture_output else None
        self.terminal_writes = [] if capture_output else None
        self.errors = (root / 'tui-stderr').open('ab')
        self.daemon_out = (root / 'qbt-output').open('ab')
        if engine == 'qbittorrent':
            qconfig = root / 'profile/qBittorrent/config'
            qconfig.mkdir(parents=True)
            (qconfig / 'qBittorrent.conf').write_text(
                '[LegalNotice]\nAccepted=true\n[Preferences]\n'
                'WebUI\\Address=127.0.0.1\nWebUI\\LocalHostAuth=false\n'
                'WebUI\\HostHeaderValidation=false\nConnection\\UPnP=false\n'
                '[BitTorrent]\nSession\\Interface=lo\nSession\\PeXEnabled=false\n'
                'Session\\LSDEnabled=false\nSession\\Encryption=2\n')

    def cli(self, *args):
        command = [str(self.binary), *args, '--json']
        result = subprocess.run(command, env=self.env, capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError(f'{command}: {result.stdout.decode()} {result.stderr.decode()}')
        return json.loads(result.stdout)

    def request(self, path, form=None):
        if self.engine == 'native':
            request = json.dumps({'method': 'POST' if form else 'GET', 'path': path,
                                  'args': list((form or {}).items()), 'file': None}).encode()
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(2)
                connection.connect(str(self.root / 'runtime/fbittorrent/engine.sock'))
                connection.sendall(struct.pack('!I', len(request)) + request)
                length = struct.unpack('!I', recv_exact(connection, 4))[0]
                result = json.loads(recv_exact(connection, length))
                if result.get('error'):
                    raise RuntimeError(result['error'])
                body = result['value']
        else:
            url = f'http://127.0.0.1:{self.web_port}/api/v2/{path}'
            body_data = urllib.parse.urlencode(form).encode() if form else None
            with urllib.request.urlopen(urllib.request.Request(url, data=body_data,
                      headers={'Origin': f'http://127.0.0.1:{self.web_port}'}), timeout=2) as response:
                body = response.read().decode()
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            return body

    def start(self):
        began = time.monotonic()
        self.process = None
        if self.engine == 'qbittorrent':
            self.daemon = subprocess.Popen([self.args.qbt, '--confirm-legal-notice',
                 f'--profile={self.root / "profile"}', f'--webui-port={self.web_port}',
                 f'--torrenting-port={self.peer_port}'], env=self.env, stdout=self.daemon_out,
                 stderr=self.daemon_out, start_new_session=True)
            self.wait_for(lambda: self.request('app/version'), began)
            self.request('app/setPreferences', {'json': json.dumps({'upnp': False,
                     'pex': False, 'lsd': False, 'encryption': 2, 'bittorrent_protocol': 1})})
            prefs = self.request('app/preferences')
            assert prefs['current_network_interface'] == 'lo' and not prefs['upnp']
            self.qbt_settings = {k: prefs.get(k) for k in ('dht','pex','lsd','encryption',
                                    'current_network_interface','bittorrent_protocol')}
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 120, 0, 0))
        self.output_bytes = 0
        if self.terminal_output is not None:
            self.terminal_output.clear()
            self.terminal_writes.clear()
        self.process = subprocess.Popen([str(self.binary), 'tui'], env=self.env,
                stdin=slave, stdout=slave, stderr=self.errors, start_new_session=True,
                preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0))
        os.close(slave)

        def drain():
            try:
                while chunk := os.read(self.master, 65536):
                    self.output_bytes += len(chunk)
                    if self.terminal_output is not None:
                        self.terminal_output.extend(chunk)
                        self.terminal_writes.append({'monotonic': time.monotonic(),
                                                     'bytes': len(chunk),
                                                     'total_output_bytes': self.output_bytes})
            except OSError:
                pass

        self.reader = threading.Thread(target=drain, daemon=True)
        self.reader.start()
        self.wait_for(lambda: self.output_bytes > 1000 and self.request('app/version'), began)
        result = self.cli('status')
        assert result['ok'] and result['engine'] == self.engine
        return (time.monotonic()-began)*1000

    def wait_for(self, predicate, began=None):
        began = began if began is not None else time.monotonic()
        last = None
        while time.monotonic()-began < 25:
            try:
                if predicate():
                    return
            except (OSError, RuntimeError, ValueError) as error:
                last = error
            if self.process and self.process.poll() is not None:
                raise RuntimeError((self.root/'tui-stderr').read_text())
            if self.daemon and self.daemon.poll() is not None:
                raise RuntimeError((self.root/'qbt-output').read_text())
            time.sleep(.01)
        raise TimeoutError(f'{self.engine} did not become ready: {last}')

    def stats(self):
        observations = []
        for process in (self.process,self.daemon):
            if not process or process.poll() is not None:
                continue
            observation = process_stats(process.pid)
            fields = Path(f'/proc/{process.pid}/stat').read_text().rsplit(')',1)[1].split()
            observation['user_cpu_ticks'] = int(fields[11])
            observation['system_cpu_ticks'] = int(fields[12])
            observations.append(observation)
        return {k: sum(item.get(k, 0) for item in observations)
                for k in ('cpu_ticks', 'user_cpu_ticks', 'system_cpu_ticks',
                          'VmRSS', 'VmHWM', 'Pss', 'Threads')}

    def idle(self, seconds):
        time.sleep(1)
        before, began = self.stats(), time.monotonic()
        time.sleep(seconds)
        after, elapsed = self.stats(), time.monotonic()-began
        return {'seconds': elapsed, 'pss_mib': after['Pss']/1024,
                'rss_mib': after['VmRSS']/1024, 'threads': after['Threads'],
                'cpu_percent_one_core': (after['cpu_ticks']-before['cpu_ticks']) /
                      os.sysconf('SC_CLK_TCK') / elapsed * 100}

    def transfer(self, seed):
        before = self.stats()
        began = time.monotonic()
        observer_cpu = time.process_time()
        self.cli('add', str(seed.torrent), '--quiet')
        peaks = self.stats()
        deadline = began+60
        while time.monotonic() < deadline:
            torrents = self.request('torrents/info')
            now = self.stats()
            for key in ('Pss','VmRSS','Threads'):
                peaks[key] = max(peaks[key], now[key])
            if len(torrents) == 1 and torrents[0]['progress'] >= 1:
                break
            time.sleep(.1)
        else:
            raise TimeoutError(f'transfer stuck: {torrents}')
        elapsed = time.monotonic()-began
        observer_cpu = time.process_time()-observer_cpu
        after = self.stats()
        target = self.root/'downloads/benchmark.bin'
        actual = hashlib.sha256(target.read_bytes()).hexdigest()
        assert actual == seed.sha256, (actual,seed.sha256)
        return {'seconds': elapsed, 'mib_per_second': len(seed.data)/1048576/elapsed,
                'cpu_seconds': (after['cpu_ticks']-before['cpu_ticks']) / os.sysconf('SC_CLK_TCK'),
                'user_cpu_seconds': (after['user_cpu_ticks']-before['user_cpu_ticks']) / os.sysconf('SC_CLK_TCK'),
                'system_cpu_seconds': (after['system_cpu_ticks']-before['system_cpu_ticks']) / os.sysconf('SC_CLK_TCK'),
                'common_seed_and_observer_cpu_seconds': observer_cpu,
                'add_started_monotonic': began, 'complete_monotonic': began+elapsed,
                'peak_pss_mib': peaks['Pss']/1024, 'peak_rss_mib': peaks['VmRSS']/1024,
                'peak_threads': peaks['Threads'], 'verified_sha256': actual}

    def close(self):
        began = time.monotonic()
        if self.process and self.process.poll() is None:
            os.write(self.master, b'q')
            self.process.wait(timeout=8)
            assert self.process.returncode == 0
        tui_ms = (time.monotonic()-began)*1000
        if self.reader:
            self.reader.join(timeout=1)
            assert not self.reader.is_alive()
            os.close(self.master)
            self.reader = self.master = None
        remaining = self.stats()
        if self.daemon and self.daemon.poll() is None:
            self.daemon.terminate()
            self.daemon.wait(timeout=10)
        assert not (self.root/'runtime/fbittorrent/engine.sock').exists()
        return {'tui_close_ms': tui_ms, 'total_stop_ms': (time.monotonic()-began)*1000,
                'after_tui_close': remaining}

    def cleanup(self):
        for process in (self.process, self.daemon):
            if process and process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        if self.reader:
            self.reader.join(timeout=1)
            os.close(self.master)
        self.errors.close()
        self.daemon_out.close()


def verify_completed_receipt(seed, label, download):
    receipts = seed.wait_for_receipts(label)
    positive = [receipt for receipt in receipts if receipt['bytes_sent'] > 0]
    assert len(positive) == 1, (label, receipts)
    receipt = positive[0]
    assert receipt['bytes_sent'] == len(seed.data), (label, receipt)
    assert download['add_started_monotonic'] <= receipt['first_payload_monotonic'] <= (
        receipt['last_payload_monotonic']) <= download['complete_monotonic'], (label, receipt, download)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--baseline-binary', type=Path,
                        help='also measure this native binary; prewarm each variant once and rotate six orders')
    parser.add_argument('--qbt', default='/usr/bin/qbittorrent-nox')
    parser.add_argument('--runs', type=int, default=20)
    parser.add_argument('--mib', type=int, default=128)
    parser.add_argument('--rate-mib', type=float, default=32)
    parser.add_argument('--idle-seconds', type=float, default=10)
    parser.add_argument('--ratings', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    if args.baseline_binary:
        args.baseline_binary = args.baseline_binary.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    result = {'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'binary': str(args.binary), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'qbt': args.qbt, 'qbt_version': subprocess.check_output([args.qbt,'--version'],text=True).strip(),
              'platform': platform.platform(), 'cpu_affinity': sorted(os.sched_getaffinity(0)),
              'load_before': Path('/proc/loadavg').read_text().strip(),
              'cpu_pressure_before': Path('/proc/pressure/cpu').read_text().strip(),
              'scenario': 'whole app, same 120x30 TUI, real cached IMDb ratings, private loopback TCP torrent, '
                          'cached missing-title lookup, encryption off, sequential/first-last, fresh destination, '
                          'warm executable and payload caches',
              'payload_mib': args.mib, 'seed_limit_mib_s': args.rate_mib,
              'runs': [], 'limits': 'Background host activity; representative paired measurements, not a quiet-host max-throughput result. '
                                   'Active CPU excludes short-lived add CLI and common Python seed. Progress polling every 100ms. '
                                   'Download wall time is CLI add to verified engine completion, including its fixed 400ms sleep. '
                                   'p95 is descriptive at 20 samples, not a robust extreme-tail estimate.'}
    variants = [('native', 'native', args.binary), ('qbittorrent', 'qbittorrent', args.binary)]
    if args.baseline_binary:
        variants.insert(0, ('baseline_native', 'native', args.baseline_binary))
        orders = list(itertools.permutations(variants))
        result['baseline_binary'] = str(args.baseline_binary)
        result['baseline_binary_sha256'] = hashlib.sha256(args.baseline_binary.read_bytes()).hexdigest()
        result['comparison_orders'] = [[variant[0] for variant in order] for order in orders]
        result['warmups'] = []
        result['prewarm_per_variant'] = 1
        batches = [(-1, variants)]
        batches.extend((pair, orders[pair % len(orders)]) for pair in range(args.runs))
    else:
        batches = [(pair, variants if pair % 2 == 0 else list(reversed(variants)))
                   for pair in range(args.runs)]
    with tempfile.TemporaryDirectory(prefix='fbittorrent-engine-benchmark-') as tmp:
        root = Path(tmp)
        ratings = root/'title.ratings.tsv'
        shutil.copyfile(args.ratings,ratings)
        result['ratings_sha256'] = hashlib.sha256(ratings.read_bytes()).hexdigest()
        seed = Seed(root,args.mib,args.rate_mib)
        result['payload_sha256'] = seed.sha256
        result['torrent_sha256'] = hashlib.sha256(seed.torrent.read_bytes()).hexdigest()
        try:
            for pair, order in batches:
                warmup = pair == -1
                for variant, engine, binary in order:
                    label = f'warmup:{variant}' if warmup else f'{pair+1}:{variant}'
                    run_root = root/f'{pair}-{variant}'
                    run_root.mkdir()
                    app = App(run_root,engine,args,ratings,binary=binary)
                    try:
                        row = {'pair': pair+1, 'engine': variant, 'opening_ms': app.start()}
                        if args.baseline_binary:
                            row['binary'] = str(binary)
                        if warmup:
                            row['warmup'] = True
                        # Equal number of actual widget CLI polls in every run.
                        latencies = []
                        for _ in range(3):
                            queried = time.monotonic()
                            assert app.cli('status')['ok']
                            latencies.append((time.monotonic()-queried)*1000)
                        row['status_cli_ms'] = latencies
                        if not warmup and pair in (0,args.runs//2,args.runs-1):
                            row['idle_empty'] = app.idle(args.idle_seconds)
                        else:
                            time.sleep(1)
                        seed.label = label
                        row['download'] = app.transfer(seed)
                        if not warmup and pair in (0,args.runs//2,args.runs-1):
                            row['idle_completed'] = app.idle(args.idle_seconds)
                        row['close'] = app.close()
                        row['seed_receipt'] = verify_completed_receipt(seed, label, row['download'])
                        row['reopen_ms'] = app.start()
                        restored = time.monotonic()
                        app.wait_for(lambda: (items := app.request('torrents/info')) and
                             items[0]['progress'] >= 1 and not items[0]['state'].startswith('checking'))
                        row['reopen_to_completed_ms'] = row['reopen_ms']+(time.monotonic()-restored)*1000
                        row['reclose'] = app.close()
                        if engine == 'qbittorrent':
                            row['qbt_preferences'] = app.qbt_settings
                        row['stderr'] = (run_root/'tui-stderr').read_text()
                        assert not row['stderr'], row['stderr']
                        result['warmups' if warmup else 'runs'].append(row)
                        args.output.write_text(json.dumps(result,indent=2)+'\n')
                        print(json.dumps({'pair':pair+1,'engine':variant,
                               'open_ms':round(row['opening_ms'],1),
                               'download_s':round(row['download']['seconds'],3),
                               'cpu_s':row['download']['cpu_seconds'],
                               'peak_pss_mib':round(row['download']['peak_pss_mib'],1),
                               'reopen_ms':round(row['reopen_to_completed_ms'],1)}),flush=True)
                    finally:
                        app.cleanup()
                    shutil.rmtree(run_root)
        finally:
            seed.close()
        result['seed_receipts'] = seed.receipts
        for row in result['runs'] + result.get('warmups', []):
            label = (f"warmup:{row['engine']}" if row.get('warmup')
                     else f"{row['pair']}:{row['engine']}")
            verify_completed_receipt(seed, label, row['download'])
    result['load_after'] = Path('/proc/loadavg').read_text().strip()
    result['cpu_pressure_after'] = Path('/proc/pressure/cpu').read_text().strip()
    args.output.write_text(json.dumps(result,indent=2)+'\n')


if __name__ == '__main__':
    main()
