#!/usr/bin/env python3
"""Run the isolated benchmark, retaining host samples and stopping on sustained load.

Pass the complete benchmark command after --. This only signals its own child.
"""
import argparse
import json
from pathlib import Path
import signal
import subprocess
import time

from engine_benchmark_host import snapshot, sample


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host-samples', type=Path, required=True)
    parser.add_argument('--receipt', type=Path, required=True)
    parser.add_argument('--interval-seconds', type=float, default=30)
    parser.add_argument('--max-busy-percent', type=float, default=20)
    parser.add_argument('--abort-after', type=int, default=2)
    parser.add_argument('--external-work-pid', type=int, action='append', default=[],
                        help='also stop if an identified external worker consumes CPU')
    parser.add_argument('--external-work-inventory', type=Path,
                        help='JSON workers list with pid/start_ticks; changes invalidate attribution')
    parser.add_argument('--external-work-cpu-percent', type=float, default=5)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    identities = json.loads(args.external_work_inventory.read_text())['workers'] if args.external_work_inventory else []
    args.external_work_pid = sorted(set(args.external_work_pid) | {item['pid'] for item in identities})
    assert command and args.interval_seconds > 0 and args.abort_after > 0
    args.host_samples.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt = {'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
               'command': command, 'host_interval_seconds': args.interval_seconds,
               'background_busy_threshold_percent': args.max_busy_percent,
               'sustained_load_samples_required': args.abort_after,
               'external_work_pids': args.external_work_pid,
               'external_work_cpu_threshold_one_core': args.external_work_cpu_percent,
               'external_work_cpu_accounting': 'sum over the fixed identified worker inventory per actual sample duration',
               'external_work_identities': identities,
               'aborted_for_load': False, 'disqualified_for_load': False,
               'completed_with_disqualifying_load': False}
    process = subprocess.Popen(command)
    receipt['benchmark_pid'] = process.pid
    args.receipt.write_text(json.dumps(receipt, indent=2)+'\n')
    previous = snapshot([process.pid])
    busy_streak = 0
    try:
        with args.host_samples.open('a') as output:
            while True:
                try:
                    process.wait(timeout=args.interval_seconds)
                except subprocess.TimeoutExpired:
                    pass
                current = snapshot([process.pid])
                row = sample(previous, current, 'monitor', args.max_busy_percent)
                row['benchmark_running'] = process.poll() is None
                external_activity = [item for item in row['processes_with_cpu_or_heavy_role']
                                     if item['pid'] in args.external_work_pid and
                                     (item['cpu_percent_one_core'] or 0) > 0]
                external_cpu = sum(item['cpu_percent_one_core'] for item in external_activity)
                row['identified_external_worker_activity'] = external_activity
                row['identified_external_worker_cpu_percent_one_core'] = external_cpu
                complete_window = row['sample_seconds'] >= args.interval_seconds
                row['complete_monitor_window'] = complete_window
                if complete_window:
                    busy_streak = busy_streak+1 if row['estimated_background_busy_percent_all_cpus'] > args.max_busy_percent else 0
                row['consecutive_busy_samples'] = busy_streak
                reasons = []
                identity_changes = [item for item in identities
                                    if item['pid'] not in current['processes'] or
                                    current['processes'][item['pid']]['start_ticks'] != item['start_ticks']]
                row['identified_external_worker_identity_changes'] = identity_changes
                if identity_changes:
                    reasons.append('identified external worker identity changed or disappeared')
                if external_cpu > args.external_work_cpu_percent:
                    reasons.append('summed identified external worker CPU exceeds threshold')
                if row['heavy_jobs_outside_owned_pids']:
                    reasons.append('external compiler/index job observed')
                if not row['owned_cpu_accounting_consistent']:
                    reasons.append('CPU accounting inconsistent')
                if busy_streak >= args.abort_after:
                    reasons.append('sustained background residual exceeds threshold')
                row['load_disqualifiers'] = reasons
                output.write(json.dumps(row)+'\n')
                output.flush()
                if process.poll() is not None:
                    receipt['final_window_seconds'] = row['sample_seconds']
                    receipt['final_partial_window'] = not complete_window
                    receipt['final_window_identified_external_cpu_percent_one_core'] = external_cpu
                if reasons:
                    receipt['disqualified_for_load'] = True
                    receipt['load_disqualification_timestamp_utc'] = row['timestamp_utc']
                    receipt['load_disqualifiers'] = reasons
                    if process.poll() is None:
                        receipt['aborted_for_load'] = True
                        receipt['load_abort_timestamp_utc'] = row['timestamp_utc']
                        process.send_signal(signal.SIGINT)
                        process.wait(timeout=30)  # App/seed finally blocks clean the fixture.
                    else:
                        receipt['completed_with_disqualifying_load'] = True
                    break
                if process.poll() is not None:
                    break
                previous = current
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
            process.wait(timeout=30)
        receipt['exit_code'] = process.returncode
        receipt['finished_timestamp_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        args.receipt.write_text(json.dumps(receipt, indent=2)+'\n')
    raise SystemExit(process.returncode if not receipt['disqualified_for_load'] else 75)


if __name__ == '__main__':
    main()
