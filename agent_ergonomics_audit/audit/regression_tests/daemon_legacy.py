"""A fake v0.1 socket owner exercises the new CLI's upgrade routing, not a legacy binary."""
from contextlib import contextmanager
import fcntl
import json
from pathlib import Path
import socket
import struct
import threading

import daemon as d


@contextmanager
def old_engine(sandbox):
    directory = Path(sandbox.env['XDG_RUNTIME_DIR']) / 'fbittorrent'
    directory.mkdir(mode=0o700)
    path = directory / 'engine.sock'
    calls, errors = [], []
    row = dict(hash='a' * 40, name='old-owner-only', state='downloading',
               progress=0.25, size=4096, completed=1024, dlspeed=12345, upspeed=678,
               save_path=str(sandbox.downloads))
    replies = {'app/version': '0.1.0', 'torrents/info': json.dumps([row]),
               'transfer/info': json.dumps(dict(dl_info_speed=12345, up_info_speed=678,
                                               connection_status='connected'))}
    done = threading.Event()

    def exact(connection, count):
        data = bytearray()
        while len(data) < count:
            part = connection.recv(count - len(data))
            if not part:
                raise EOFError('old engine request ended early')
            data.extend(part)
        return bytes(data)

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener, (directory / 'engine.lock').open('w+') as lock:
        (directory / 'engine.lock').chmod(0o600)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        listener.bind(str(path)); path.chmod(0o600)
        listener.listen(8); listener.settimeout(0.05)

        def serve():
            while not done.is_set():
                try:
                    connection, _ = listener.accept()
                except socket.timeout:
                    continue
                try:
                    with connection:
                        connection.settimeout(1)
                        size = struct.unpack('!I', exact(connection, 4))[0]
                        if size > 65536 or len(calls) >= 100:
                            raise ValueError('old-owner fixture request budget exceeded')
                        request = json.loads(exact(connection, size)); calls.append(request)
                        value = replies.get(request['path']) if request['method'] == 'GET' else None
                        response = dict(value=value, error=None if value is not None else 'unexpected legacy request')
                        wire = json.dumps(response).encode()
                        connection.sendall(struct.pack('!I', len(wire)) + wire)
                except Exception as error:
                    errors.append(repr(error))

        thread = threading.Thread(target=serve, name='fake-old-engine')
        thread.start()
        try:
            yield lock, calls, errors
        finally:
            done.set(); thread.join(timeout=2)
            if thread.is_alive():
                raise RuntimeError('old engine fixture thread did not stop')
            path.unlink()


def old_engine_compat(binary, systemd):
    with d.Case('daemon', 'old_engine_compat') as case, d.Sandbox() as sandbox, d.Harness(case, binary, sandbox, False) as h:
        # Autostart is allowed: the refusal must precede any manager/launcher attempt.
        config = Path(sandbox.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('autostart = false', 'autostart = true'))
        ledger = Path(sandbox.env['XDG_DATA_HOME']) / 'fbittorrent/engine/torrents.json'
        ledger.parent.mkdir(parents=True)
        ledger.write_text('{"migrated_legacy":true,"records":[]}\n')
        before = (config.read_bytes(), ledger.read_bytes())
        with old_engine(sandbox) as (lock, calls, errors):
            api = h.engine_path.with_name('api.sock')
            case.check(h.engine_path.is_socket() and not api.exists(),
                       'the fake v0.1 owner exposes engine.sock only, with the real owner flock held', str(h.engine_path))
            rows = h.cli('list', start=True)
            case.check(rows.get('ok') is True and rows.get('count') == 1 and
                       [row['name'] for row in rows.get('torrents', [])] == ['old-owner-only'],
                       'list reads the old socket row, which is absent from the offline ledger', json.dumps(rows))
            status = h.cli('status', start=True)
            case.check(status.get('ok') is True and status.get('total') == 1 and
                       status.get('dl') == 12345 and status.get('up') == 678 and
                       status.get('connection') == 'connected' and status.get('app_open') is True,
                       'status reads the old owner transfer rates and counts successfully', json.dumps(status))
            refusal = 'an older fbittorrent app is running; quit it with Q and retry'
            for args in [('stop', 'a' * 40), ('add', 'magnet:?xt=urn:btih:' + 'b' * 40),
                         ('start', 'a' * 40), ('delete', 'a' * 40)]:
                result = h.cli(*args, code=4, start=True)
                case.check(result == dict(ok=False, code=4, code_name='backend', error=refusal),
                           'write refuses the older app with the exact message and dictionary exit 4', json.dumps(dict(args=args, result=result)))
            # Older engines may not hold this lock. A readable socket still forbids offline writes.
            fcntl.flock(lock, fcntl.LOCK_UN)
            result = h.cli('stop', 'a' * 40, code=4, start=True)
            case.check(result == dict(ok=False, code=4, code_name='backend', error=refusal),
                       'an available ledger lock does not bypass the old-owner refusal', json.dumps(result))
            artifact = case.path.with_suffix('.requests.json')
            artifact.write_text(json.dumps(calls, indent=2) + '\n')
            case.check(not errors and all(call['method'] == 'GET' and call['path'] in
                       ('app/version', 'torrents/info', 'transfer/info') for call in calls),
                       'no mutation is sent to the fake old engine', json.dumps(dict(calls=calls, errors=errors)))
            launched = [path.name for path in (sandbox.root / 'shims').glob('*.log')]
            case.check(not launched and not api.exists() and before == (config.read_bytes(), ledger.read_bytes()),
                       'neither reads nor refused writes start another owner or change config/ledger', json.dumps(dict(shim_calls=launched, api_exists=api.exists())))


CASES = (old_engine_compat,)
