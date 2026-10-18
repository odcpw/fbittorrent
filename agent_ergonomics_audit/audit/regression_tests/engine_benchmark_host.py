#!/usr/bin/env python3
"""Read-only host fingerprint and load samples for the engine benchmark.

Default: ten one-second preflight samples. --duration-seconds adds periodic
monitoring; --watch-pid stops that monitoring when the benchmark PID exits.
Every sample is appended, including busy samples. No jobs or settings change.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import time
import tomllib


BUILD_NAMES = {'cargo', 'rustc', 'cc', 'cc1', 'cc1plus', 'gcc', 'g++', 'c++',
               'clang', 'clang++', 'collect2', 'ld', 'ld.lld', 'mold', 'ninja',
               'make', 'cmake', 'lto-wrapper', 'lto1'}


def read(path):
    try:
        return Path(path).read_text().strip()
    except OSError as error:
        return {'unavailable': str(error)}


def command(arguments):
    try:
        result = subprocess.run(arguments, capture_output=True, text=True, timeout=10)
        return {'command': arguments, 'exit_code': result.returncode,
                'stdout': result.stdout.strip(), 'stderr': result.stderr.strip()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'command': arguments, 'unavailable': str(error)}


def artifact(path):
    path = Path(path).resolve()
    try:
        with path.open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        stat = path.stat()
        return {'path': str(path), 'sha256': digest, 'bytes': stat.st_size,
                'mtime_ns': stat.st_mtime_ns}
    except OSError as error:
        return {'path': str(path), 'unavailable': str(error)}


def pressure(kind):
    rows = {}
    for line in Path(f'/proc/pressure/{kind}').read_text().splitlines():
        name, *values = line.split()
        rows[name] = {key: int(value) if key == 'total' else float(value)
                      for key, value in (item.split('=') for item in values)}
    return rows


def process_kind(name, executable):
    if name in BUILD_NAMES or executable in BUILD_NAMES or name.startswith('rustc'):
        return 'compiler/build'
    if any(word in name.lower() or word in executable.lower()
           for word in ('fannasarchive-b', 'indexer', 'index-build', 'recollindex', 'updatedb')):
        return 'index'
    return 'other'


def snapshot(owned_roots):
    processes = {}
    for directory in Path('/proc').iterdir():
        if not directory.name.isdigit():
            continue
        try:
            fields = (directory/'stat').read_text().rsplit(')', 1)[1].split()
            name = (directory/'comm').read_text().strip()
            try:
                executable = (directory/'exe').resolve().name
            except OSError:
                executable = name
            pid = int(directory.name)
            processes[pid] = {'pid': pid, 'ppid': int(fields[1]), 'name': name,
                              'state': fields[0], 'start_ticks': int(fields[19]),
                              'cpu_ticks': int(fields[11])+int(fields[12]),
                              'reaped_child_cpu_ticks': int(fields[13])+int(fields[14]),
                              'kind': process_kind(name, executable)}
        except (OSError, ValueError, IndexError):
            continue  # Processes can exit between procfs reads.
    owned = set(owned_roots) | {os.getpid()}
    while True:
        descendants = {pid for pid, process in processes.items() if process['ppid'] in owned}
        expanded = owned | descendants
        if expanded == owned:
            break
        owned = expanded
    for pid, process in processes.items():
        process['owned'] = pid in owned
    # Reaped child CPU moves into its parent's cutime/cstime. Summing both
    # self and reaped-child work over the live owned family preserves work
    # from completed app/CLI processes between snapshots without counting a
    # still-live child twice. The monitor stays alive and reaps its benchmark.
    owned_family_cpu_ticks = sum(process['cpu_ticks']+process['reaped_child_cpu_ticks']
                                 for process in processes.values() if process['owned'])
    cpu = [int(value) for value in Path('/proc/stat').read_text().splitlines()[0].split()[1:9]]
    return {'monotonic': time.monotonic(), 'uptime': float(Path('/proc/uptime').read_text().split()[0]),
            'cpu': cpu, 'processes': processes, 'owned_family_cpu_ticks': owned_family_cpu_ticks,
            'cpu_psi': pressure('cpu'),
            'memory_psi': pressure('memory')}


def sample(before, after, phase, max_busy):
    elapsed = after['monotonic']-before['monotonic']
    ticks = os.sysconf('SC_CLK_TCK')
    cpu = [end-start for start, end in zip(before['cpu'], after['cpu'])]
    busy = 100*(sum(cpu)-cpu[3]-cpu[4])/sum(cpu)
    observed = []
    for pid, process in after['processes'].items():
        previous = before['processes'].get(pid)
        if previous and previous['start_ticks'] == process['start_ticks']:
            delta = process['cpu_ticks']-previous['cpu_ticks']
        elif process['start_ticks'] >= before['uptime']*ticks:
            delta = process['cpu_ticks']
        else:
            delta = None
        row = dict(process, cpu_percent_one_core=100*delta/ticks/elapsed if delta is not None else None)
        if delta or row['kind'] != 'other':
            observed.append(row)
    observed.extend(dict(process, cpu_percent_one_core=None, exited_during_sample=True)
                    for pid, process in before['processes'].items()
                    if pid not in after['processes'] and process['kind'] != 'other')
    observed.sort(key=lambda row: row['cpu_percent_one_core'] or 0, reverse=True)
    heavy = [row for row in observed if row['kind'] != 'other']
    outside = [row for row in heavy if not row['owned']]
    observed_owned_cpu = sum(row['cpu_percent_one_core'] or 0 for row in observed if row['owned'])
    owned_family_ticks = after['owned_family_cpu_ticks']-before['owned_family_cpu_ticks']
    owned_cpu = 100*max(0, owned_family_ticks)/ticks/elapsed
    background_raw = 100*(sum(cpu)-cpu[3]-cpu[4]-max(0, owned_family_ticks))/sum(cpu)
    background_busy = max(0, background_raw)
    memory = {key: value.strip() for key, value in
              (line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
              if key in ('MemTotal', 'MemAvailable', 'SwapTotal', 'SwapFree')}
    row = {'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
           'phase': phase, 'sample_seconds': elapsed, 'host_busy_percent_all_cpus': busy,
           'owned_cpu_percent_one_core': owned_cpu,
           'observed_live_owned_cpu_percent_one_core': observed_owned_cpu,
           'owned_family_cpu_ticks_delta': owned_family_ticks,
           'owned_family_cpu_ticks_before': before['owned_family_cpu_ticks'],
           'owned_family_cpu_ticks_after': after['owned_family_cpu_ticks'],
           'whole_host_cpu_ticks_delta': cpu,
           'whole_host_capacity_cores_during_sample': sum(cpu)/ticks/elapsed,
           'estimated_background_busy_percent_unclamped': background_raw,
           'estimated_background_busy_percent_all_cpus': background_busy,
           'host_iowait_percent_all_cpus': 100*cpu[4]/sum(cpu),
           'loadavg': Path('/proc/loadavg').read_text().strip(), 'memory': memory,
           'processes_with_cpu_or_heavy_role': observed, 'heavy_jobs': heavy,
           'heavy_jobs_outside_owned_pids': outside,
           'owned_cpu_accounting_consistent': owned_family_ticks >= 0 and background_raw >= 0,
           'wait_for_host': bool(outside) or background_busy > max_busy or owned_family_ticks < 0 or background_raw < 0,
           'wait_reasons': (['external compiler/index process observed'] if outside else []) +
                           ([f'background busy exceeds {max_busy:g}% of all CPUs']
                            if background_busy > max_busy else []) +
                           (['owned CPU accounting raced process exit/reaping']
                            if owned_family_ticks < 0 or background_raw < 0 else [])}
    for kind in ('cpu', 'memory'):
        current = after[f'{kind}_psi']
        row[f'{kind}_psi'] = current
        row[f'{kind}_psi_stall_percent_during_sample'] = {
            name: 100*(values['total']-before[f'{kind}_psi'][name]['total'])/(elapsed*1_000_000)
            for name, values in current.items()}
    return row


def fingerprint(args):
    root = Path(__file__).resolve().parents[3]
    rustc = args.rustc or os.environ.get('RUSTC') or shutil.which('rustc')
    mounts = command(['findmnt', '-J', '-T', tempfile.gettempdir(), '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS'])
    profiles = tomllib.loads((root/'Cargo.toml').read_text()).get('profile', {})
    paths = {'binary': args.binary, 'baseline_binary': args.baseline_binary,
             'qbittorrent': args.qbt, 'ratings': args.ratings,
             'benchmark_script': Path(__file__).with_name('engine_benchmark.py'),
             'host_script': Path(__file__), 'cargo_manifest': root/'Cargo.toml',
             'monitor_script': Path(__file__).with_name('run_engine_benchmark_monitored.py'),
             'cargo_lock': root/'Cargo.lock'}
    return {'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
            'kernel': platform.platform(), 'cpu': command(['lscpu', '-J']),
            'cpu_affinity': sorted(os.sched_getaffinity(0)), 'fixture_tmp_dir': tempfile.gettempdir(),
            'fixture_mount': mounts, 'governors': {
                str(path.parent): read(path) for path in
                Path('/sys/devices/system/cpu/cpufreq').glob('policy*/scaling_governor')},
            'energy_performance_preferences': {str(path.parent): read(path) for path in
                Path('/sys/devices/system/cpu/cpufreq').glob('policy*/energy_performance_preference')},
            'power_profile': command(['powerprofilesctl', 'get']),
            'no_turbo': read('/sys/devices/system/cpu/intel_pstate/no_turbo'),
            'smt_active': read('/sys/devices/system/cpu/smt/active'),
            'toolchain': {'rustc': command([rustc, '--version']) if rustc else None,
                          'python': platform.python_version(),
                          'qbittorrent': command([str(args.qbt), '--version'])},
            'cargo_profiles': profiles, 'declared_build_flags': args.build_flags,
            'monitor_environment_rustflags': os.environ.get('RUSTFLAGS'),
            'git_sha': command(['git', '-C', str(root), 'rev-parse', 'HEAD']),
            'git_worktree_status': command(['git', '-C', str(root), 'status', '--porcelain']),
            'artifacts': {name: artifact(path) for name, path in paths.items() if path},
            'owned_process_roots': args.owned_pid,
            'sampling': {'preflight_seconds': args.preflight_seconds,
                         'preflight_interval_seconds': args.preflight_interval_seconds,
                         'complete_preflight_windows': args.complete_preflight_windows,
                         'cpu_accounting': 'owned-family-v1: self plus reaped-child ticks over unique live owned TGIDs',
                         'monitor_interval_seconds': args.interval_seconds,
                         'monitor_duration_seconds': args.duration_seconds,
                         'max_background_busy_percent_all_cpus': args.max_busy_percent},
            'limits': 'Read-only snapshots; no tuning or process control. CPU process percentages are '
                      'per core; host busy is across all CPUs. Background busy subtracts observed '
                      'owned-family self plus reaped-child CPU and is an estimate; procfs reads race '
                      'with process exit/reaping. Short-lived external jobs between snapshots can '
                      'be missed; aggregate host CPU includes their work. Build flags are declared '
                      'metadata, not proof of binary provenance. Busy samples are retained.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--baseline-binary', type=Path)
    parser.add_argument('--qbt', type=Path, default=Path('/usr/bin/qbittorrent-nox'))
    parser.add_argument('--ratings', type=Path)
    parser.add_argument('--rustc')
    parser.add_argument('--build-flags')
    parser.add_argument('--owned-pid', type=int, action='append', default=[])
    parser.add_argument('--fingerprint', type=Path, required=True)
    parser.add_argument('--samples', type=Path, required=True)
    parser.add_argument('--preflight-seconds', type=float, default=10,
                        help='zero skips preflight when starting a monitor after the benchmark PID is known')
    parser.add_argument('--preflight-interval-seconds', type=float, default=1,
                        help='sampling window during preflight; default preserves one-second diagnostics')
    parser.add_argument('--complete-preflight-windows', action='store_true',
                        help='do not shorten the last preflight window to the remaining duration')
    parser.add_argument('--interval-seconds', type=float, default=30)
    parser.add_argument('--duration-seconds', type=float, default=0,
                        help='periodic monitoring duration after preflight, zero means preflight only')
    parser.add_argument('--watch-pid', type=int, help='stop monitoring when this benchmark PID exits')
    parser.add_argument('--max-busy-percent', type=float, default=20)
    args = parser.parse_args()
    assert args.preflight_seconds >= 0 and args.preflight_interval_seconds > 0 and args.interval_seconds > 0 and args.duration_seconds >= 0
    assert args.preflight_seconds+args.duration_seconds > 0
    args.fingerprint.parent.mkdir(parents=True, exist_ok=True)
    args.samples.parent.mkdir(parents=True, exist_ok=True)
    args.fingerprint.write_text(json.dumps(fingerprint(args), indent=2)+'\n')
    before = snapshot(args.owned_pid)
    preflight_end = time.monotonic()+args.preflight_seconds
    end = preflight_end+args.duration_seconds
    with args.samples.open('a') as output:
        while time.monotonic() < end:
            now = time.monotonic()
            phase = 'preflight' if now < preflight_end else 'monitor'
            if phase == 'monitor' and args.watch_pid and not Path(f'/proc/{args.watch_pid}').exists():
                break
            remaining = (preflight_end if phase == 'preflight' else end)-now
            interval = args.preflight_interval_seconds if phase == 'preflight' else args.interval_seconds
            time.sleep(interval if phase == 'preflight' and args.complete_preflight_windows else min(interval, remaining))
            after = snapshot(args.owned_pid)
            row = sample(before, after, phase, args.max_busy_percent)
            output.write(json.dumps(row)+'\n')
            output.flush()
            print(json.dumps({key: row[key] for key in
                  ('timestamp_utc', 'phase', 'host_busy_percent_all_cpus',
                   'estimated_background_busy_percent_all_cpus', 'wait_for_host', 'wait_reasons')}), flush=True)
            before = after


if __name__ == '__main__':
    main()
