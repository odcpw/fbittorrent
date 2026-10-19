"""P1-17: two real builds, atomic replacement, busy deferral and explicit restart."""
from contextlib import contextmanager
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import socket
import struct
import subprocess
import time
import tomllib

import daemon as d
import fixtures
from daemon_policy import fresh
from upgrade_artifacts import IDS, executable, valid_cache

_pair = None


def fingerprint():
    digest=hashlib.sha256()
    for helper in (Path(__file__),Path(__file__).with_name("upgrade_artifacts.py")):
        digest.update(helper.read_bytes())
    for path in sorted([*d.REPO.joinpath('src').rglob('*.rs'),*[d.REPO/name for name in ('Cargo.toml','Cargo.lock','build.rs','rust-toolchain.toml')]]):
        digest.update(str(path.relative_to(d.REPO)).encode());digest.update(path.read_bytes())
    # Vendor changes are rare; their git tree plus a local patch identifies the
    # runtime input without hashing hundreds of MB for every isolated worker.
    for command in (['git','rev-parse','HEAD:vendor'],['git','diff','HEAD','--','vendor']):
        digest.update(subprocess.check_output(command,cwd=d.REPO))
    return digest.hexdigest()


def binary_digest(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def binaries(case, box):
    lock=d.REPO/'target/test-logs/upgrade-fixtures/.compile.lock'
    lock.parent.mkdir(parents=True,exist_ok=True)
    with lock.open('a') as owned:
        fcntl.flock(owned,fcntl.LOCK_EX)
        return _binaries_unlocked(case,box)


def _binaries_unlocked(case, box):
    global _pair
    if _pair is not None:
        return _pair
    supplied = [os.environ.get('FBT_UPGRADE_OLD_BINARY'), os.environ.get('FBT_UPGRADE_NEW_BINARY')]
    if all(supplied):
        _pair = tuple(Path(value).resolve() for value in supplied)
        return _pair
    source=fingerprint()
    cache=d.REPO/'target/test-logs/upgrade-fixtures'/source
    manifest=cache/'manifest.json'
    if manifest.exists():
        saved=json.loads(manifest.read_text())
        paths=[cache/(tag+'.binary') for tag in ('old','new')]
        if valid_cache(saved,paths,binary_digest):
            _pair=tuple(paths)
            case.step('reuse real compiled upgrade fixtures','source fingerprint and both binary SHA256 values match').ok(source,[manifest])
            return _pair
    cache.mkdir(parents=True,exist_ok=True)
    d.private_environment(box)
    home = Path(pwd.getpwuid(os.geteuid()).pw_dir)
    paths = []
    for tag in ('old', 'new'):
        path = cache/(tag+'.binary')
        output = case.path.with_suffix('.build-' + tag + '.log')
        env = dict(box.env, CARGO_BUILD_JOBS='4',
                   RUSTUP_HOME=os.environ.get('RUSTUP_HOME', str(home / '.rustup')),
                   CARGO_HOME=os.environ.get('CARGO_HOME', str(home / '.cargo')))
        step = case.step('compile real upgrade fixture ' + tag, 'locked debug build succeeds in private XDG')
        with output.open('w') as stream:
            child = case.popen(['cargo', 'build', '--locked', '--bin', 'fbittorrent', '--message-format=json',
                                '--config', f'env.FBT_BUILD_ID_OVERRIDE.value="{IDS[tag]}"',
                                '--config', 'env.FBT_BUILD_ID_OVERRIDE.force=true'], env=env,
                               cwd=d.REPO, stdout=stream)
            code = child.wait()
        if not step.close(code == 0, f'exit={code}; build id=p1-17-{tag}', [output]):
            raise RuntimeError('upgrade fixture build failed')
        shutil.copy2(executable(output,env,d.REPO),path)
        # Compilation success alone cannot validate a stale delivery or cache.
        # Probe the copied artifact through its real private API before reuse.
        with d.Sandbox() as probe,d.Harness(case,path,probe,False) as owner:
            process=owner.start_serve(idle_exit_secs=0)
            try:
                if not(owner.receive_ready() and owner.wait_engine()):raise RuntimeError('compiled fixture did not start')
                with d.ApiConn(owner) as client:
                    build=client.hello()['result']['server']['build']
                    case.check(build==IDS[tag],'copied fixture reports its compiled build identity',build)
                    if build!=IDS[tag]:raise RuntimeError('stale or incorrectly identified fixture executable')
            finally:
                if process.child.poll() is None:owner.stop_serve(process)
        paths.append(path)
    if source!=fingerprint():raise RuntimeError('source changed while building upgrade fixtures')
    saved={tag:binary_digest(path) for tag,path in zip(IDS,paths)}
    saved['ids']=IDS
    if not valid_cache(saved,paths,binary_digest):raise RuntimeError('upgrade fixtures must be distinct validated executables')
    manifest.write_text(json.dumps(saved,indent=2)+'\n')
    _pair = tuple(paths)
    return _pair


def replace(case, box, new, current):
    temporary = box.root / 'replacement'
    shutil.copy2(new, temporary)
    before = current.stat().st_ino
    temporary.replace(current)
    case.check(before != current.stat().st_ino, 'copy and atomic rename replace only the private executable inode',
               f'path={current}; old inode={before}; new inode={current.stat().st_ino}')


@contextmanager
def owned(name, binary, systemd):
    with d.Case('daemon', name) as case, d.Sandbox() as box:
        old, new = binaries(case, box)
        current = box.root / 'fbittorrent'; shutil.copy2(old, current)
        box.env['FBT_TEST_UPGRADE_CHECK_MS'] = '100'; box.keys.append('FBT_TEST_UPGRADE_CHECK_MS')
        box.env.pop('FBT_BUILD_ID_OVERRIDE', None)
        with d.Harness(case, current, box, systemd=False) as h:
            with d.ExitStack() as resources:
                unit = resources.enter_context(d.InstalledUnit(h)) if systemd else None
                if unit:
                    h.cli('up', start=True); unit.verify_child(); process = None
                else:
                    process = h.start_serve(idle_exit_secs=0)
                    if not (h.receive_ready() and h.wait_engine()): raise RuntimeError('upgrade daemon not ready')
                with d.ApiConn(h) as client:
                    before = client.hello()['result']
                    case.check(before['server']['build'] == 'p1-17-old', 'hello identifies the real first build', json.dumps(before))
                    yield case, box, h, unit, process, client, before, new
                if not unit and h.engine_path.exists():
                    os.kill(h.daemon_pid(), d.signal.SIGTERM)
                    h.wait_for(lambda: (not h.engine_path.exists(), h.stderr()), 'foreground replacement drains')
            h.lock_free()


def replaced_owner(case, h, unit, process, before):
    if unit:
        def exit_status():
            result = unit.control('show', h.sandbox.unit, '--property=SubState,ExecMainStatus').output
            return 'SubState=auto-restart' in result and 'ExecMainStatus=75' in result, result
        h.wait_for(exit_status, 'manager records upgrade exit 75 before automatic replacement', timeout=5)
    else:
        process.wait(code=75)
        h.notify.close()
        replacement = h.start_serve(idle_exit_secs=0)
        if not (h.receive_ready() and h.wait_engine()): raise RuntimeError('manual foreground replacement not ready')
    def new_hello():
        with d.ApiConn(h) as client:
            hello = client.hello()['result']
            return hello['instance'] != before['instance'] and hello['server']['build'] == 'p1-17-new', json.dumps(hello)
    h.wait_for(new_hello, 'new owner hello has a new instance and compiled build id', timeout=12)
    if unit: unit.verify_child()


def upgrade_idle_exit_75(binary, systemd):
    with owned('upgrade_idle_exit_75', binary, systemd) as (case, box, h, unit, p, client, before, new):
        h.wait_for(lambda: (fresh(h)['busy_reasons'] == [], json.dumps(fresh(h))), 'empty daemon becomes safe to upgrade')
        replace(case, box, new, Path(h.binary))
        replaced_owner(case, h, unit, p, before)
        terminal = next((event for event in client.events() if event.get('ev') == 'bye'), None)
        case.check(terminal is not None and terminal['reason'] == 'upgrade', 'old connection receives bye upgrade', json.dumps(terminal))


@contextmanager
def downloading(case, box, h, client):
    with d.Sandbox() as source, d.Harness(case, _pair[0], source) as seed:
        cfg = tomllib.loads((Path(source.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml').read_text())
        peers = socket.inet_aton('127.0.0.1') + struct.pack('>H', cfg['peer_port'])
        tracker_body = fixtures.bencode(dict(interval=1, peers=peers))
        with fixtures.HttpFixture(source.root, {'/announce': dict(base64=base64.b64encode(tracker_body).decode())}) as tracker:
            payload = source.downloads / 'upgrade-download.bin'; payload.write_bytes(bytes(range(256)) * 32768)
            item = fixtures.torrent_fixture(source.root, [payload], tracker=tracker.base + '/announce')
            process = seed.start_serve(idle_exit_secs=0)
            if not (seed.receive_ready() and seed.wait_engine()): raise RuntimeError('private seed not ready')
            with d.ApiConn(seed) as peer:
                peer.hello()
                peer.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(source.downloads), seed=True))
                seed.wait_for(lambda: ((row := peer.call('torrents.get', dict(hash=item.info_hash))['result']['torrent'])['state'] == 'seeding', json.dumps(row)), 'seed verifies actual local bytes')
                peer.call('torrents.set_upload_limit', dict(hashes=[item.info_hash], bps=32768))
                client.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(box.downloads)))
                def received():
                    row = client.call('torrents.get', dict(hash=item.info_hash))['result']['torrent']
                    return row['state'] == 'downloading' and row.get('completed_bytes', 0) > 0 and row.get('dl_bps', 0) > 0, json.dumps(row)
                if not h.wait_for(received, 'replacement test has real capped loopback bytes in flight', timeout=15): raise RuntimeError('download did not start')
                try: yield item
                finally: seed.stop_serve(process)


def upgrade_pending_while_busy(binary, systemd):
    with owned('upgrade_pending_while_busy', binary, systemd) as (case, box, h, unit, p, client, before, new):
        with downloading(case, box, h, client) as item:
            replace(case, box, new, Path(h.binary))
            h.wait_for(lambda: ((info := client.call('app.info')['result'])['upgrade_pending'], json.dumps(info)), 'replaced binary marks upgrade pending while a download is running')
            summary = fresh(h)
            case.check(summary['upgrade_pending'] and 'downloading' in summary['busy_reasons'], 'summary pushes pending upgrade and actual download busy reason', json.dumps(summary))
            time.sleep(.6)
            after = client.call('app.info')['result']
            case.check(after['instance'] == before['instance'] and after['server']['build'] == 'p1-17-old', 'three accelerated replacement checks do not restart active work', json.dumps(after))
            client.call('torrents.stop', dict(hashes=[item.info_hash]))
            replaced_owner(case, h, unit, p, before)


def restart_now(binary, systemd):
    with owned('restart_now', binary, systemd) as (case, box, h, unit, p, client, before, new):
        client.call('app.client', dict(interactive=True))
        replace(case, box, new, Path(h.binary))
        h.wait_for(lambda: ((info := client.call('app.info')['result'])['upgrade_pending'], json.dumps(info)), 'interactive old owner holds pending replacement')
        if unit:
            result = h.cli('restart', timeout=35)
            case.check(result.get('scope') == 'daemon' and result.get('instance') != before['instance'] and result.get('server', {}).get('build') == 'p1-17-new',
                       'explicit CLI restart switches immediately despite interactive work and returns replacement hello', json.dumps(result))
            unit.verify_child()
        else:
            result = h.cli('restart', code=8, timeout=35)
            p.wait(code=75)
            case.check(result.get('code') == 8 and not h.engine_path.exists(), 'foreground restart exits 8 after 30 s without inventing an owner', json.dumps(result))
        case.check('reason=Upgrade code=75' in h.stderr(), 'explicit restart drains with unit-only exit 75', h.stderr())


def restart_closed_never_starts(binary, systemd):
    with d.Case('daemon','restart_closed_never_starts') as case, d.Sandbox() as box, d.Harness(case,binary,box) as h:
        result = h.cli('restart',code=4)
        case.check(not d.DaemonContract.socket_path(box).exists(), 'restart on closed daemon exits 4 without startup', json.dumps(result))
