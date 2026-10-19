"""Remove / Undo / Remove while real upstream work delays engine dispatch."""
from contextlib import contextmanager
import hashlib
import http.server
import json
import threading
import time

import daemon as d


@contextmanager
def delayed_dispatch(h):
    release = threading.Event()
    entered = threading.Condition()
    requests = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            with entered:
                requests.append(self.path)
                entered.notify_all()
            release.wait()
            try:
                self.send_error(404)
            except OSError:
                pass

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, kwargs=dict(poll_interval=.05))
    thread.start()
    try:
        with d.ApiConn(h) as workers:
            workers.hello()
            for index in range(8):
                result = workers.call('torrents.add', dict(source=dict(url=f'http://127.0.0.1:{server.server_port}/{index}.torrent'), stopped=True))
                assert result['result']['state'] == 'pending'
            with entered:
                assert entered.wait_for(lambda: len(requests) > 0, timeout=10), requests
            # Each URL acknowledges from its worker immediately before engine
            # I/O. Eight acknowledgements occupy all eight workers, even if
            # the engine serializes some source admissions behind the first.
            h.case.check(len(requests) > 0, 'eight accepted URL adds hold every worker behind the owned loopback source', str(requests))
            yield release
    finally:
        release.set()
        server.shutdown()
        server.server_close()
        thread.join(2)
        h.case.check(not thread.is_alive(), 'owned delay fixture joins before XDG cleanup')


def remove_undo_delayed_engine(binary, systemd):
    with d.api_owned('remove_undo_delayed_engine', binary, systemd) as (case, box, h, process), d.ApiConn(h) as client:
        client.hello()
        fixture = d.recheck_local_fixture(box, size=16384)
        payload = fixture.files[0]
        digest = hashlib.sha256(payload.read_bytes()).hexdigest()
        client.call('torrents.add', dict(source=dict(path=str(fixture.torrent)), save_path=str(payload.parent), stopped=True))
        assert h.wait_for(lambda: ((row := client.call('torrents.get', dict(hash=fixture.info_hash))['result']['torrent'])['checked'], json.dumps(row)), 'private local payload finishes checking')
        calls = [('torrents.remove', dict(hashes=[fixture.info_hash], delete_files=False), 21),
                 ('torrents.add', dict(source=dict(restore=fixture.info_hash)), 2),
                 ('torrents.remove', dict(hashes=[fixture.info_hash], delete_files=False), 21)]
        for index, (op, args, delay) in enumerate(calls):
            reference = 'delayed-cleanup-' + str(index)
            with delayed_dispatch(h) as release:
                began = time.monotonic()
                reply = client.call(op, args, ref=reference, confirm=False)
                elapsed = time.monotonic()-began
                case.check(reply.get('result') == dict(state='pending', ref=reference) and elapsed < 2,
                           'Remove/Undo admits promptly while every engine dispatch worker is delayed', json.dumps(dict(reply=reply, elapsed_s=elapsed)))
                until = time.monotonic() + delay
                while time.monotonic() < until:
                    # Presence is served outside the engine queue. Keep real
                    # connection liveness while crossing the 20 s UI deadline.
                    client.call('app.client', dict(interactive=True))
                    time.sleep(min(1, max(0, until-time.monotonic())))
                case.check(not any(e.get('ref') == reference and e.get('ev') == 'mutation.result' for e in client.pushed),
                           'accepted intent stays pending rather than timing out before engine execution', reference)
                release.set()
                while True:
                    event = client.receive()
                    if event.get('ev') == 'mutation.result' and event.get('ref') == reference:
                        break
                case.check(event['outcome'] == 'done' and event['result']['state'] == 'done',
                           'engine completion pushes the confirmed result under the original ref', json.dumps(event))
                if op == 'torrents.remove':
                    case.check(8 <= event['result']['undo_until']-time.time() <= 10.1,
                               'Undo window begins at durable removal, not queue admission', json.dumps(event))
                else:
                    case.check(event['result']['hash'] == fixture.info_hash,
                               'Undo restores the same identity', json.dumps(event))
            client.call('torrents.get', dict(hash=fixture.info_hash), error='not_found' if op == 'torrents.remove' else None)
            case.check(payload.exists() and hashlib.sha256(payload.read_bytes()).hexdigest() == digest,
                       'Remove / Undo / Remove preserves the owned payload exactly', str(payload))


CASES = (remove_undo_delayed_engine,)
