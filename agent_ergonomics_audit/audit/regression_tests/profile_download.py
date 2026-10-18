#!/usr/bin/env python3
"""Isolated stack/CPU diagnostic. Sampling timings are not benchmark results."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from types import SimpleNamespace

from engine_benchmark import App, Seed


def threads(pid):
    result = {}
    for task in Path(f'/proc/{pid}/task').iterdir():
        try:
            fields = (task / 'stat').read_text().rsplit(')', 1)[1].split()
            switches = {line.split(':')[0]: int(line.split(':')[1])
                        for line in (task / 'status').read_text().splitlines()
                        if 'ctxt_switches:' in line}
            result[task.name] = {'name': (task / 'comm').read_text().strip(),
                                'user_ticks': int(fields[11]), 'system_ticks': int(fields[12]),
                                'wchan': (task / 'wchan').read_text().strip(), **switches}
        except FileNotFoundError:
            pass
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--ratings', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--mib', type=int, default=256)
    parser.add_argument('--rate-mib', type=int, default=32)
    parser.add_argument('--samples', type=int, default=40)
    parser.add_argument('--perf-callgraph', choices=('dwarf,16384', 'fp'), default='dwarf,16384',
                        help='CPU diagnostic call graph; fp needs a separate frame-pointer build')
    parser.add_argument('--settle-seconds', type=float, default=1,
                        help='settle after three actual status CLI calls, matching the benchmark (default: 1)')
    profiler_options = parser.add_mutually_exclusive_group()
    profiler_options.add_argument('--no-sampler', action='store_true',
                                  help='collect process/source diagnostics without stopping or sampling threads')
    profiler_options.add_argument('--perf', type=Path,
                                  help='local perf executable; sample CPU without ptrace stops')
    profiler_options.add_argument('--strace', type=Path,
                                  help='local strace executable; retain an all-thread syscall summary')
    parser.add_argument('--chunk-diagnostics', action='store_true',
                        help='accept explicit feature-only chunk counter output; diagnostic evidence only')
    args = parser.parse_args()
    if args.settle_seconds < 0:
        parser.error('--settle-seconds must be nonnegative')
    args.binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    original_popen = subprocess.Popen
    libc = ctypes.CDLL(None, use_errno=True)

    def own_profile_child(command, *positional, **kwargs):
        if command == [str(args.binary), 'tui']:
            original_preexec = kwargs.get('preexec_fn')
            def preexec():
                if original_preexec:
                    original_preexec()
                # Permit inspection of this isolated child only. No host sysctl change.
                if libc.prctl(0x59616d61, ctypes.c_ulong(-1).value, 0, 0, 0) != 0:
                    raise OSError(ctypes.get_errno(), 'PR_SET_PTRACER failed')
            kwargs['preexec_fn'] = preexec
        return original_popen(command, *positional, **kwargs)
    subprocess.Popen = own_profile_child
    mode = 'counts' if args.no_sampler else 'perf' if args.perf else 'strace' if args.strace else 'eu-stack'
    limits = {
        'counts': 'No stack sampler. Opt-in source counters and any dispatch ablation perturb execution; '
                  'timings establish diagnostic attribution, not a production saving.',
        'perf': 'perf samples user CPU only and adds sampling overhead; diagnostic timings. '
                'Self samples identify executing symbols. Caller attribution requires verified stack quality; '
                'frame pointers join synchronous calls only, not asynchronous ownership chains.',
        'strace': 'strace -f -c counts syscalls across the isolated app threads and adds ptrace overhead; '
                  'syscall times and transfer timings are diagnostic, not final CPU or throughput evidence.',
        'eu-stack': 'eu-stack stops threads while sampling; diagnostic timings',
    }
    result = {'binary': str(args.binary),
              'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'profiler_mode': mode, 'perf_callgraph': args.perf_callgraph,
              'settle_seconds': args.settle_seconds,
              'limits': limits[mode],
              'samples': []}
    with tempfile.TemporaryDirectory(prefix='fbittorrent-cpu-profile-') as temporary:
        root = Path(temporary)
        ratings = root / 'ratings.tsv'
        shutil.copyfile(args.ratings, ratings)
        seed = Seed(root, args.mib, args.rate_mib)
        app_root = root / 'native'
        app_root.mkdir()
        app = App(app_root, 'native', SimpleNamespace(binary=args.binary, qbt='qbittorrent-nox'), ratings)
        original_stats = app.stats
        result['cpu_series'] = []
        def sampled_stats():
            stats = original_stats()
            result['cpu_series'].append({'monotonic': time.monotonic(), **stats})
            return stats
        app.stats = sampled_stats
        errors = []
        profiler = None
        worker = None

        def stop_profiler():
            nonlocal profiler
            if profiler is None:
                return
            current, profiler = profiler, None
            if current.poll() is None:
                current.send_signal(signal.SIGINT)
            try:
                stdout, stderr = current.communicate(timeout=15)
            except subprocess.TimeoutExpired:
                current.kill()
                stdout, stderr = current.communicate(timeout=5)
                result['profiler_forced_cleanup'] = True
            result['profiler_exit_code'] = current.returncode
            result['profiler_output'] = stderr.decode(errors='replace')
            if stdout:
                result['profiler_stdout'] = stdout.decode(errors='replace')

        try:
            result['opening_ms'] = app.start()
            result['status_cli_ms'] = []
            for _ in range(3):
                began = time.monotonic()
                status = app.cli('status')
                assert status['ok'] and status['engine'] == 'native', status
                result['status_cli_ms'].append((time.monotonic()-began)*1000)
            time.sleep(args.settle_seconds)
            seed.label = 'profile'
            result['threads_before'] = threads(app.process.pid)
            profiler_command = None
            if args.perf:
                profiler_command = [str(args.perf.resolve()), 'record', '-e', 'cpu-clock:u',
                    '-F', '499', '--call-graph', args.perf_callgraph, '-p', str(app.process.pid),
                    '-o', str(args.output / 'profile.perf.data')]
            elif args.strace:
                summary = args.output / 'strace-summary.txt'
                result['strace_summary'] = str(summary.resolve())
                profiler_command = [str(args.strace.resolve()), '-f', '-c', '-p', str(app.process.pid),
                                    '-o', str(summary)]
            if profiler_command:
                result['profiler_command'] = profiler_command
                profiler = subprocess.Popen(profiler_command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                time.sleep(.1)
                assert profiler.poll() is None, profiler.communicate()
            def transfer():
                try:
                    result['transfer'] = app.transfer(seed)
                except BaseException as error:
                    errors.append(error)
            worker = threading.Thread(target=transfer)
            worker.start()
            app.wait_for(lambda: bool(app.request('torrents/info')))
            for index in range(args.samples):
                if profiler_command or args.no_sampler:
                    break
                if not worker.is_alive():
                    break
                began = time.monotonic()
                before = threads(app.process.pid)
                sampled = subprocess.run(['eu-stack', '-p', str(app.process.pid), '-n', '24', '-d'],
                                         capture_output=True, text=True, timeout=20)
                (args.output / f'stack-{index:03d}.txt').write_text(sampled.stdout + sampled.stderr)
                result['samples'].append({'index': index, 'monotonic': began,
                                          'duration': time.monotonic()-began,
                                          'returncode': sampled.returncode, 'threads': before})
                if sampled.returncode == 2:
                    raise RuntimeError(sampled.stderr)
                time.sleep(.08)
            worker.join(timeout=80)
            assert not worker.is_alive()
            if errors:
                raise errors[0]
            result['threads_after'] = threads(app.process.pid)
            if profiler:
                stop_profiler()
                assert result['profiler_exit_code'] in (0, -2, 130), result['profiler_output']
                assert not result.get('profiler_forced_cleanup'), 'profiler required forced cleanup'
                if args.strace:
                    assert Path(result['strace_summary']).is_file(), 'strace produced no syscall summary'
            result['close'] = app.close()
            result['stderr'] = (app.root / 'tui-stderr').read_text()
            result['attribution_reports'] = []
            result['epoch_reports'] = []
            result['chunk_reports'] = []
            for line in result['stderr'].splitlines():
                report = json.loads(line)
                if report.get('schema') == 'fbittorrent.cpu_attribution.v1':
                    result['attribution_reports'].append(report)
                elif report.get('schema') == 'fbittorrent.epoch_attribution.v1':
                    result['epoch_reports'].append(report)
                elif report.get('schema') == 'fbittorrent.chunk_attribution.v1' and args.chunk_diagnostics:
                    result['chunk_reports'].append(report)
                else:
                    raise RuntimeError(f'unexpected diagnostic stderr: {report!r}')
            if args.chunk_diagnostics:
                assert len(result['chunk_reports']) == 1, 'expected one chunk diagnostic at runtime teardown'
                chunk_report = result['chunk_reports'][0]
                assert chunk_report.get('pid') == app.process.pid, 'chunk diagnostic PID differs from app'
                assert chunk_report.get('teardown_succeeded') is True, 'chunk diagnostic lacks successful teardown'
                assert chunk_report.get('scope') == 'process_cumulative'
        except BaseException as error:
            result['failure'] = {'type': type(error).__name__, 'message': str(error)}
            raise
        finally:
            try:
                stop_profiler()
            finally:
                try:
                    app.cleanup()
                finally:
                    try:
                        seed.close()
                    finally:
                        if worker is not None:
                            worker.join(timeout=5)
                            result['transfer_worker_joined'] = not worker.is_alive()
                        subprocess.Popen = original_popen
                        result['seed_receipts'] = seed.receipts
                        (args.output / 'profile.json').write_text(json.dumps(result, indent=2)+'\n')
                        assert worker is None or not worker.is_alive(), 'transfer observer did not stop'
    print(json.dumps({'samples': len(result['samples']), 'transfer': result.get('transfer'),
                      'cleanup': 'owned fixture removed'}), flush=True)


if __name__ == '__main__':
    main()
