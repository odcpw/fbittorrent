"""P1-12 real lifetime tests. Every child belongs to a retaining private Sandbox."""
from contextlib import contextmanager
import base64
import json
from pathlib import Path
import socket
import struct
import time
import tomllib

import daemon as d
from daemon_bus import RevisionStream, fixture
import fixtures


@contextmanager
def owned(name, binary, systemd, seconds=2, setup=None):
    with d.Case('daemon', name) as case, d.Sandbox() as sandbox:
        if setup: setup(sandbox)
        with d.Harness(case, binary, sandbox, systemd) as harness:
            process = harness.start_serve(idle_exit_secs=seconds)
            if not (harness.receive_ready() and harness.wait_engine()):
                raise RuntimeError('private policy fixture did not become ready')
            try:
                yield case, sandbox, harness, process
            finally:
                if process.child.poll() is None: harness.stop_serve(process)
                harness.lock_free()


def hello(client, kind='test', interactive=False):
    return client.call('hello', dict(proto=1, client=dict(name='idle-policy-test', version='0.2.0', kind=kind), interactive=interactive))


def subscribe(client):
    sub = client.call('subscribe', dict(topics=['summary'], interval_ms=250))['result']['sub']
    stream = RevisionStream(client)
    frames = stream.snapshot(sub, ['summary'])
    return stream, next(frame['data']['daemon'] for frame in frames if frame.get('topic') == 'summary')


def fresh(harness):
    with d.ApiConn(harness) as client:
        hello(client, 'bar')
        _, summary = subscribe(client)
        return summary


def hold(case, process, client, seconds, expected):
    step = case.step(f'hold client for {seconds} seconds', expected)
    began = time.monotonic()
    client.socket.settimeout(.2)
    frames = []
    try:
        while time.monotonic()-began < seconds and process.child.poll() is None:
            try:
                frame = client.receive()
                if frame is None: break
                if frame.get('topic') == 'summary': frames.append(frame['data']['daemon'])
            except socket.timeout: pass
    finally:
        client.socket.settimeout(8)
    step.close(process.child.poll() is None and time.monotonic()-began >= seconds,
               json.dumps(dict(elapsed=time.monotonic()-began, exit=process.child.poll(), summaries=frames)))


def idle_exit(case, harness, process, client, timeout=6):
    step = case.step('wait for idle shutdown', 'bye idle, exit 0, both owned sockets removed')
    began = time.monotonic(); terminal = None; summaries = []
    client.socket.settimeout(timeout)
    while (frame := client.receive()) is not None:
        if frame.get('topic') == 'summary': summaries.append(frame['data']['daemon'])
        if frame.get('ev') == 'bye': terminal = frame; break
    process.wait(timeout=timeout, code=0)
    sockets = [harness.engine_path, d.DaemonContract.socket_path(harness.sandbox)]
    step.close(terminal is not None and terminal.get('reason') == 'idle' and not any(p.exists() for p in sockets),
               json.dumps(dict(elapsed=time.monotonic()-began, bye=terminal, summaries=summaries, sockets=[p.exists() for p in sockets])))
    return time.monotonic()-began


def shutdown(case, harness, process, client):
    reply = client.call('app.shutdown', dict(reason='user'))
    case.check(reply.get('ok') is True, 'owner shutdown is accepted independently of busy or disabled idle state', str(reply))
    process.wait(timeout=10, code=0)
    case.check(not harness.engine_path.exists(), 'owner shutdown drains the engine and removes its socket', str(harness.engine_path))


def idle_exit_no_torrents(binary, systemd):
    with owned('idle_exit_no_torrents', binary, systemd) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client); _, summary = subscribe(client)
        case.check(summary['busy_reasons'] == [] and summary['idle_exit_in_s'] <= 2,
                   'an empty daemon exposes its idle countdown', str(summary))
        idle_exit(case, h, p, client)


def idle_exit_bar_does_not(binary, systemd):
    with owned('idle_exit_bar_does_not', binary, systemd) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client, 'bar', interactive=True); _, summary = subscribe(client)
        case.check(summary['busy_reasons'] == [], 'a bar cannot claim interactive ownership even if it sends interactive true', str(summary))
        idle_exit(case, h, p, client)


def idle_exit_interactive_keeps_alive(binary, systemd):
    with owned('idle_exit_interactive_keeps_alive', binary, systemd) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client, 'tui', interactive=True); _, summary = subscribe(client)
        case.check('interactive' in summary['busy_reasons'] and 'idle_exit_in_s' not in summary,
                   'interactive presence cancels the countdown and names its reason', str(summary))
        hold(case, p, client, 4, 'interactive client keeps daemon alive beyond its two-second idle delay')
        shutdown(case, h, p, client)


def parked(box):
    digest, _ = d.saved_hash_fixture(box, 4 * fixtures.MIB)
    (box.downloads / 'daemon-hash.bin').unlink()
    path = Path(box.env['XDG_DATA_HOME']) / 'fbittorrent/engine/torrents.json'
    saved = json.loads(path.read_text()); saved['records'][0]['paused'] = False
    path.write_text(json.dumps(saved))
    cfg = Path(box.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml'
    cfg.write_text(cfg.read_text().replace('network_interface = "lo"', 'network_interface = "fbmissing0"'))


def idle_exit_parked_keeps_alive(binary, systemd):
    with owned('idle_exit_parked_keeps_alive', binary, systemd, setup=parked) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client); _, summary = subscribe(client)
        case.check(summary['state'] == 'parked' and 'parked_waiting' in summary['busy_reasons'],
                   'a durable waiting download keeps the parked daemon busy', str(summary))
        hold(case, p, client, 4, 'waiting download keeps parked daemon alive beyond its two-second idle delay')
        shutdown(case, h, p, client)


def idle_exit_disabled_never_exits(binary, systemd):
    with owned('idle_exit_disabled_never_exits', binary, systemd, seconds=0) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client); _, summary = subscribe(client)
        case.check('idle_exit_in_s' not in summary and summary['busy_reasons'] == [], 'zero disables the countdown without inventing a busy reason', str(summary))
        hold(case, p, client, 4, 'disabled idle net leaves an empty daemon alive')


def idle_exit_disabled_shutdown_drains(binary, systemd):
    with owned('idle_exit_disabled_shutdown_drains', binary, systemd, seconds=0) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client); item = fixture(client, box)
        summary = fresh(h)
        case.check('downloading' in summary['busy_reasons'], 'a running incomplete fixture names its busy reason even with idle exit disabled', str(summary))
        shutdown(case, h, p, client)
        ledger = json.loads((Path(box.env['XDG_DATA_HOME']) / 'fbittorrent/engine/torrents.json').read_text())
        record = next(row for row in ledger['records'] if row['hash'] == item.info_hash)
        case.check(record['paused'] is False, 'shutdown preserves the durable run intent', str(record))


def idle_exit_glance_pauses(binary, systemd):
    with owned('idle_exit_glance_pauses', binary, systemd, seconds=30) as (case, box, h, p), d.ApiConn(h) as client:
        hello(client, 'bar'); stream, initial = subscribe(client)
        frame = stream.until(lambda f: f.get('data', {}).get('daemon', {}).get('idle_exit_in_s') == 20, seconds=15)
        client.call('app.client', dict(glance=True))
        hold(case, p, client, 5, 'open glance pauses a twenty-second countdown for five seconds')
        paused = fresh(h)
        case.check(paused['idle_exit_in_s'] == 20 and paused['busy_reasons'] == [], 'fresh subscription observes frozen twenty seconds; glance is not busy', str(paused))
        client.call('app.client', dict(glance=False))
        resumed = fresh(h)
        case.check(resumed['idle_exit_in_s'] == 20, 'closing glance resumes twenty seconds rather than resetting or subtracting the pause', str(resumed))
        elapsed = idle_exit(case, h, p, client, timeout=25)
        case.check(19 <= elapsed <= 23, 'exit follows twenty seconds after close, not fifteen', f'elapsed after close={elapsed:.3f}s')


def idle_exit_glance_drop_resumes(binary, systemd):
    with owned('idle_exit_glance_drop_resumes', binary, systemd) as (case, box, h, p), d.ApiConn(h) as glance, d.ApiConn(h) as observer:
        hello(glance, 'bar'); glance.call('app.client', dict(glance=True))
        hello(observer); subscribe(observer)
        hold(case, p, observer, 3, 'an open glance pauses the idle net beyond its original deadline')
        paused = fresh(h)
        case.check(paused['idle_exit_in_s'] == 2, 'the glance freezes two remaining seconds', str(paused))
        glance.close()
        elapsed = idle_exit(case, h, p, observer)
        case.check(1 <= elapsed <= 5, 'disconnect releases glance presence and resumes the remaining delay', f'exit {elapsed:.3f}s after socket drop')


def idle_exit_mutation_keeps_alive(binary, systemd):
    with owned('idle_exit_mutation_keeps_alive', binary, systemd) as (case, box, h, p), d.ApiConn(h) as client, d.ApiConn(h) as observer, d.stalled_api_source(case) as (url, entered):
        hello(client); hello(observer)
        result = client.call('torrents.add', dict(source=dict(url=url)))
        case.check(entered.wait(2) and result['result']['state'] == 'pending', 'real loopback URL resolution remains pending', str(result))
        _, summary = subscribe(observer)
        case.check('mutation' in summary['busy_reasons'], 'pending URL resolution names its mutation busy reason', str(summary))
        hold(case, p, observer, 4, 'pending mutation keeps daemon alive beyond its two-second idle delay')
        shutdown(case, h, p, observer)


def idle_exit_download_keeps_alive(binary, systemd):
    with d.Case('daemon', 'idle_exit_download_keeps_alive') as case, d.Sandbox() as seedbox, d.Sandbox() as box, d.Harness(case, binary, seedbox, systemd) as seedh, d.Harness(case, binary, box, systemd) as h:
        cfg = tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml').read_text())
        peer = socket.inet_aton('127.0.0.1') + struct.pack('>H', cfg['peer_port'])
        body = fixtures.bencode(dict(interval=1, peers=peer))
        with fixtures.HttpFixture(seedbox.root, {'/announce': dict(base64=base64.b64encode(body).decode())}) as tracker:
            payload = seedbox.downloads / 'idle-download.bin'
            payload.write_bytes(bytes(range(256)) * 32768)
            item = fixtures.torrent_fixture(seedbox.root, [payload], tracker=tracker.base + '/announce')
            seed = seedh.start_serve(idle_exit_secs=0)
            if not (seedh.receive_ready() and seedh.wait_engine()): raise RuntimeError('private seed not ready')
            with d.ApiConn(seedh) as seedclient:
                hello(seedclient)
                seedclient.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(seedbox.downloads), seed=True))
                def seeding():
                    frame = seedclient.call('torrents.get', dict(hash=item.info_hash))
                    return frame.get('result', {}).get('torrent', {}).get('state') == 'seeding', str(frame)
                if not seedh.wait_for(seeding, 'private local seed finishes verifying'): raise RuntimeError('seed did not verify')
                seedclient.call('torrents.set_upload_limit', dict(hashes=[item.info_hash], bps=32768))
                case.check('seeding' in fresh(seedh)['busy_reasons'], 'explicit live seed names its busy reason', str(fresh(seedh)))
                p = h.start_serve(idle_exit_secs=2)
                if not (h.receive_ready() and h.wait_engine()): raise RuntimeError('private leecher not ready')
                with d.ApiConn(h) as client:
                    hello(client)
                    client.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(box.downloads)))
                    def receiving():
                        frame = client.call('torrents.get', dict(hash=item.info_hash))
                        row = frame.get('result', {}).get('torrent', {})
                        return row.get('state') == 'downloading' and row.get('completed_bytes', 0) > 0 and row.get('dl_bps', 0) > 0, str(frame)
                    if not h.wait_for(receiving, 'real loopback payload is downloading', timeout=15): raise RuntimeError('leecher never received')
                    _, summary = subscribe(client)
                    case.check('downloading' in summary['busy_reasons'], 'active local download names its busy reason', str(summary))
                    hold(case, p, client, 4, 'real capped loopback transfer stays alive beyond two seconds')
                    shutdown(case, h, p, client)
                    h.lock_free()
                shutdown(case, seedh, seed, seedclient)
                seedh.lock_free()


def dashboard_bar_presence(binary, systemd):
    with owned('dashboard_bar_presence', binary, systemd) as (case, box, h, p), d.ApiConn(h) as client:
        response=hello(client,'bar',interactive=True)
        case.check('dashboard_presence' in response['result']['caps'],'server negotiates explicit shared-dashboard presence',json.dumps(response))
        client.call('app.client',dict(interactive=True,dashboard_visible=True,glance=False))
        _, summary=subscribe(client)
        case.check('interactive' in summary['busy_reasons'] and 'idle_exit_in_s' not in summary,
                   'visible dashboard on the shared bar socket cancels the idle countdown',json.dumps(summary))
        hold(case,p,client,4,'visible dashboard keeps daemon alive beyond its two-second idle deadline')
        # Clear the visibility marker while leaving the old interactive flag true:
        # even a stale bar interactive claim must not keep a hidden dashboard alive.
        client.call('app.client',dict(dashboard_visible=False))
        summary=fresh(h)
        case.check(summary['busy_reasons']==[] and 'idle_exit_in_s' in summary,
                   'hiding dashboard clears interactive presence despite a stale interactive flag',json.dumps(summary))
        client.call('app.client',dict(interactive=False,dashboard_visible=False,glance=False))
        idle_exit(case,h,p,client)
