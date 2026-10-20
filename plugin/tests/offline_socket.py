#!/usr/bin/env python3
"""Measure the real private bar while stopped, then exercise socket appearance."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import time
from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status


def cpu_seconds(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return sum(int(fields[i]) for i in (11, 12)) / os.sysconf('SC_CLK_TCK')


def socket_warnings(text):
    return [line for line in text.splitlines() if 'QLocalSocket::' in line]


def run(args):
    previous = ROOT / 'target/test-logs/qs_offline_socket'
    if previous.exists():
        shutil.copytree(previous, ROOT / 'target/offline-socket-runs' / str(time.time_ns()))
    with Case('qs_offline_socket', args.label) as case, fixtures.Sandbox() as box:
        fixtures.path_shim('stat', 'exec /usr/bin/stat "$@"', box.root)
        q = Session(case, box, recipe='bar-host', journal_tag='fbt-offline-'+box.root.name)
        measurement = dict(label=args.label, revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                           kernel=platform.release(), cpus=os.cpu_count(), clock_ticks=os.sysconf('SC_CLK_TCK'), load=os.getloadavg(),
                           source=str(Path(os.environ.get('FBT_PLUGIN_SOURCE', ROOT / 'plugin/oliver.torrents')).resolve()))
        offline_text = ''
        try:
            q.wait(lambda: q.state().get('widgets') == 1)
            case.check(Path(f'/proc/{q.child.pid}/exe').resolve().name in ('qs', 'quickshell'),
                       'CPU samples belong to the actual Quickshell PID', str(Path(f'/proc/{q.child.pid}/exe').resolve()))
            assert not q.state()['online']
            time.sleep(1)
            offset = len(q.read_log())
            probe_log = box.root / 'shims/stat.log'
            def probes():
                return len(probe_log.read_text().splitlines()) if probe_log.exists() else 0
            before_probes = probes()
            before = cpu_seconds(q.child.pid)
            began = time.monotonic()
            time.sleep(args.seconds)
            elapsed = time.monotonic() - began
            offline_text = q.read_log()[offset:]
            measurement['offline'] = dict(seconds=elapsed, cpu_pct=100 * (cpu_seconds(q.child.pid) - before) / elapsed,
                                          socket_warning_lines=len(socket_warnings(offline_text)), probes=probes()-before_probes)
            case.check(not q.state()['online'] and not (box.root / 'shims/fbittorrent.log').exists(),
                       'stopped bar never launches the daemon', json.dumps(measurement['offline']))
            if not args.baseline:
                case.check(not socket_warnings(offline_text) and probes()==before_probes,
                           'stopped socket has no polling or connection failures', json.dumps(measurement['offline']))
            if not args.baseline:
                runtime = Path(box.env['XDG_RUNTIME_DIR']) / 'fbittorrent'
                runtime.mkdir(mode=0o700, exist_ok=True)
                api_socket = runtime / 'api.sock'
                # A stale socket must not be retried for unrelated directory
                # events. The real daemon then replaces it with a new inode.
                stale = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                stale.bind(str(api_socket))
                try:
                    q.wait(lambda: 'QLocalSocket::ConnectionRefusedError' in q.read_log())
                    failures = len(socket_warnings(q.read_log()))
                    for index in range(3):
                        (runtime / ('unrelated-' + str(index))).touch()
                        time.sleep(.15)
                    case.check(len(socket_warnings(q.read_log())) == failures == 1,
                               'one connection failure per unchanged stale socket despite directory activity', q.read_log())
                finally:
                    stale.close()
                with daemon.Harness(case, args.binary, box, False) as harness:
                    measurement['online_ms'] = []
                    for iteration in range(2):
                        process = harness.start_serve(idle_exit_secs=0)
                        if not harness.receive_ready():
                            raise RuntimeError('private daemon did not start')
                        appeared_ns = api_socket.stat().st_ctime_ns
                        q.wait(lambda: q.state()['online'], timeout=1)
                        elapsed_ms = (time.time_ns()-appeared_ns)/1e6
                        measurement['online_ms'].append(round(elapsed_ms))
                        case.check(elapsed_ms < 1000, 'socket appearance connects the bar within one second', json.dumps(measurement))
                        harness.stop_serve(process)
                        harness.notify.close()
                        q.wait(lambda: not q.state()['online'])
                        time.sleep(1)
                        offset = len(q.read_log())
                        time.sleep(3)
                        after = q.read_log()[offset:]
                        case.check(not socket_warnings(after), 'daemon stop returns the bar to quiet', after)
                        if iteration == 0:
                            # Only this fixture's stopped runtime is removed.
                            assert runtime.is_relative_to(box.root)
                            shutil.rmtree(runtime)
        finally:
            q.close()
            # The measured shell writes directly to its unique journal stream.
            journal = q.read_log()
            journal_path = case.path.with_suffix('.journal.log')
            journal_path.write_text(offline_text)
            measurement['journal'] = dict(socket_warning_lines=len(socket_warnings(offline_text)), path=str(journal_path), identifier=q.journal_tag)
            case.check('Configuration Loaded' in journal,
                       'measurement reads the owned shell directly from the journal', json.dumps(measurement['journal']))
            artifact = case.path.with_suffix('.measurement.json')
            artifact.write_text(json.dumps(measurement, indent=2) + '\n')
            print(json.dumps(measurement), flush=True)
    return suite_status('qs_offline_socket')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--seconds', type=float, default=60)
    parser.add_argument('--label', default='quiet')
    parser.add_argument('--baseline', action='store_true')
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    raise SystemExit(run(args))
