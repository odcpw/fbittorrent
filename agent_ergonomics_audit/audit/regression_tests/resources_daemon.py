"""Real daemon bus budget, with private paused local torrents and /proc metrics."""
from contextlib import ExitStack
from decimal import Decimal
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import threading
import time

import daemon as d
import fixtures
from fbtlog import Case, REPO, suite_status


def proc(pid):
    root = Path('/proc') / str(pid)
    fields = (root / 'stat').read_text().rsplit(')', 1)[1].split()
    rss = next(int(line.split()[1]) for line in (root / 'status').read_text().splitlines() if line.startswith('VmHWM:'))
    return dict(cpu_ticks=int(fields[11]) + int(fields[12]), rss_kb=rss)


def window(pid, seconds):
    before = proc(pid)
    deadline = time.monotonic() + seconds
    peak = before['rss_kb']
    while time.monotonic() < deadline:
        time.sleep(min(.25, max(0, deadline - time.monotonic())))
        peak = max(peak, proc(pid)['rss_kb'])
    after = proc(pid)
    return dict(cpu_s=(after['cpu_ticks']-before['cpu_ticks'])/os.sysconf('SC_CLK_TCK'), rss_kb=peak)


def reference(harness, seconds, run=0):
    """Status every ten seconds; retain last /proc sample and wait4 precision.

    wait4 is supplementary because short CLI processes can finish between
    adjacent /proc samples, and kernel CPU ticks are only 10 ms on this host.
    """
    usage, ticks, peak = 0., 0, 0
    def reaped_ticks():
        fields = Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[13]) + int(fields[14])
    before_reaped = reaped_ticks()
    started = time.monotonic()
    calls = []
    for index in range(max(1, int(seconds / 10))):
        at = started + index*10
        while time.monotonic() < at:
            time.sleep(min(.1, at-time.monotonic()))
        output = harness.case.path.with_suffix(f'.status-{run+1}-{index}.json')
        observed = []
        with output.open('wb') as stdout:
            process = harness.case.popen([str(harness.binary), 'status', '--json'], env=harness.sandbox.env,
                                         cwd=harness.sandbox.root, stdout=stdout)
            deadline=time.monotonic()+20
            while True:
                if time.monotonic()>=deadline:
                    process.kill()
                    _, status, _ = os.wait4(process.pid, 0)
                    process.returncode=os.waitstatus_to_exitcode(status)
                    raise TimeoutError('private reference CLI exceeded 20 seconds')
                try: observed.append(proc(process.pid))
                except (FileNotFoundError, ProcessLookupError, StopIteration): pass
                pid, status, resource = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                time.sleep(.001)
        cpu = resource.ru_utime + resource.ru_stime
        usage += cpu
        last = max((row['cpu_ticks'] for row in observed), default=0)
        ticks += last
        proc_peak = max((row['rss_kb'] for row in observed), default=0)
        if not observed:
            raise RuntimeError('reference CLI exited without a /proc RSS sample')
        peak = max(peak, proc_peak)
        result = json.loads(output.read_text())
        harness.case.check(process.returncode == 0 and result.get('ok') is not False,
                           'reference status reads the same private daemon successfully',
                           json.dumps(dict(exit=process.returncode, cpu_s=cpu, proc_ticks=last, result=result), sort_keys=True))
        calls.append(dict(cpu_s=cpu, proc_ticks=last, rss_kb=proc_peak,
                          wait4_rss_kb=resource.ru_maxrss))
    while time.monotonic()-started < seconds:
        time.sleep(min(.25, seconds-(time.monotonic()-started)))
    return dict(cpu_s=(reaped_ticks()-before_reaped)/os.sysconf('SC_CLK_TCK'), direct_proc_cpu_s=ticks/os.sysconf('SC_CLK_TCK'), rss_kb=peak, precise_cpu_s=usage, calls=calls, cpu_source='kernel /proc parent cutime+cstime delta for only the reaped CLI children; tick rounding occurs once across all calls')


def measure(binary, torrents, subscribers, seconds, runs):
    samples, references = [], []
    with Case('daemon_budget', f't{torrents}_s{subscribers}') as case:
        for run in range(runs):
            with d.Sandbox() as sandbox, d.Harness(case, binary, sandbox, False) as harness, ExitStack() as clients:
                d.private_environment(sandbox)
                process = harness.start_serve(idle_exit_secs=0)
                if not (harness.receive_ready() and harness.wait_engine()):
                    raise RuntimeError('private budget daemon not ready')
                try:
                    client = clients.enter_context(d.ApiConn(harness))
                    client.hello()
                    for index in range(torrents):
                        source = sandbox.root / f'payload-{index}'
                        source.mkdir()
                        payload = source / f'bus-{index}.bin'
                        payload.write_bytes(hashlib.sha256(str(index).encode()).digest() * 8192)
                        item = fixtures.torrent_fixture(source, [payload], name=f'fixture-{index}')
                        client.call('torrents.add', dict(source=dict(path=str(item.torrent)), save_path=str(sandbox.downloads), stopped=True))
                    def settled():
                        frame = client.call('torrents.list', {})
                        rows = frame.get('result', {}).get('torrents', [])
                        return frame.get('ok') is True and len(rows) == torrents and all(row['state'] == 'paused' for row in rows), json.dumps(frame, sort_keys=True)
                    if not harness.wait_for(settled, 'all local distinct fixtures are paused before measuring', timeout=60):
                        raise RuntimeError('budget fixtures still checking')
                    stop = threading.Event()
                    readers = []
                    failures = []
                    for index in range(subscribers):
                        subscriber = clients.enter_context(d.ApiConn(harness, timeout=20))
                        subscriber.hello(kind='bar')
                        subscriber.call('subscribe', dict(topics=['summary'], interval_ms=5000))
                        def read(connection=subscriber):
                            while not stop.is_set():
                                try:
                                    if connection.receive() is None:
                                        if not stop.is_set(): failures.append('unexpected EOF')
                                        return
                                except OSError as error:
                                    if not stop.is_set(): failures.append(str(error))
                                    return
                        reader = threading.Thread(target=read)
                        reader.start(); readers.append((reader, subscriber))
                    step = case.step(f'measure run {run+1}: {torrents} torrents, {subscribers} subscribers for {seconds}s',
                                     'daemon /proc user+system CPU seconds and peak RSS, healthy connections')
                    pid = harness.daemon_pid()
                    value = window(pid, seconds)
                    samples.append(value)
                    step.close(not failures, json.dumps(dict(pid=pid, **value, failures=failures), sort_keys=True))
                    stop.set()
                    for reader, subscriber in readers:
                        subscriber.socket.shutdown(2)
                        reader.join(timeout=3)
                        case.check(not reader.is_alive(), 'budget subscriber reader stopped', str(reader.is_alive()))
                    if subscribers == 0:
                        step = case.step(f'measure reference run {run+1}', 'status --json every ten seconds against this same daemon for the same window')
                        value = reference(harness, seconds, run)
                        references.append(value)
                        step.ok(json.dumps(value, sort_keys=True))
                finally:
                    clients.close()
                    harness.stop_serve(process)
                    harness.lock_free()
    return samples, references


def compare_baseline(baseline, current):
    """P0-06's median + max(10%, three spreads), for named daemon workloads."""
    for key in ('host', 'seconds'):
        if baseline.get('daemon', {}).get(key) != current[key]:
            raise ValueError(f'daemon baseline reference mismatch for {key}; remeasure deliberately')
    results = []
    for config, measured in current['configs'].items():
        if config not in baseline['configs']:
            raise ValueError(f'daemon baseline lacks {config}; remeasure deliberately')
        for metric in ('cpu_s', 'rss_kb'):
            values = baseline['configs'][config][metric]
            observations = measured[metric]
            if len(values) != 3 or not observations or any(not math.isfinite(value) or value < 0 for value in [*values, *observations]):
                raise ValueError(f'{config}/{metric} needs three finite nonnegative baseline samples and measured values')
            exact = [Decimal(str(value)) for value in values]
            center = statistics.median(exact)
            margin = max(center*Decimal('.10'), 3*(max(exact)-min(exact)))
            results.append(dict(config=config, metric=metric, baseline=values, current=observations,
                                median=float(center), margin=float(margin), limit=float(center+margin),
                                ok=max(Decimal(str(value)) for value in observations) <= center+margin))
    return results


def bar_budget(configs, references):
    # These CPU seconds are integral kernel ticks divided by CLK_TCK. Decimal
    # subtraction preserves exact boundary equality (0.28 - 0.26 == 0.02).
    checks = []
    for torrents in (10,100):
        reference_cpu = Decimal(str(statistics.median(row['cpu_s'] for row in references[str(torrents)])))
        cpu = lambda k: Decimal(str(statistics.median(configs[f'{torrents}x{k}']['cpu_s'])))
        incremental = max(Decimal(0), cpu(1)-cpu(0))
        checks.append(dict(torrents=torrents, incremental_bar_cpu_s=float(incremental), reference_cli_cpu_s=float(reference_cpu), ok=incremental <= reference_cpu))
    return checks


def main(args):
    if os.environ.get('FBT_TEST_LOG_ROOT'):
        import fbtlog
        fbtlog.LOG_ROOT = Path(os.environ['FBT_TEST_LOG_ROOT'])
    if args.seconds <= 0 or args.runs < 1 or args.torrents < 0 or args.subscribers not in range(33):
        raise ValueError('positive measurement window/runs, nonnegative torrent count, at most 32 subscribers')
    configs, references = {}, {}
    matrix = [(n,k) for n in (10,100) for k in (0,1,8)] if args.matrix else [(args.torrents,args.subscribers)]
    for torrents, subscribers in matrix:
        rows, ref = measure(args.binary, torrents, subscribers, args.seconds, args.runs)
        if len(rows)!=args.runs:
            with Case('daemon_budget','matrix_incomplete') as case:
                case.check(False, 'every scheduled configuration completes all runs', f'{torrents}x{subscribers}: completed {len(rows)} of {args.runs}')
            return suite_status('daemon_budget')
        configs[f'{torrents}x{subscribers}'] = {metric:[row[metric] for row in rows] for metric in ('cpu_s','rss_kb')}
        if ref: references[str(torrents)] = ref
    result = dict(configs=configs, reference={metric:[row[metric] for row in references.get('100', next(iter(references.values()), []))] for metric in ('cpu_s','rss_kb')},
                  reference_samples=references, seconds=args.seconds,
                  binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(), host=os.uname().nodename)
    if args.matrix:
        checks = bar_budget(configs, references)
        result['bar_budget'] = checks
        with Case('daemon_budget','bar_criterion') as case:
            for check in checks:
                case.check(check['ok'], 'one 5000 ms bar subscription costs no more CPU than status every ten seconds', json.dumps(check,sort_keys=True))
    if args.baseline:
        comparisons = compare_baseline(json.loads(args.baseline.read_text()), result)
        result['comparisons'] = comparisons
        with Case('daemon_budget', 'baseline_compare') as case:
            for comparison in comparisons:
                case.check(comparison['ok'], 'daemon CPU/RSS stays within median plus max(10 percent, three spreads)', json.dumps(comparison, sort_keys=True))
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
    if args.write_baseline:
        if suite_status('daemon_budget') != 0 or any(len(v['cpu_s']) != args.runs for v in configs.values()):
            return 1
        if args.runs != 3 or not args.matrix or args.seconds != 60:
            raise ValueError('daemon baseline requires the complete 60-second matrix, three runs each')
        # Keep P0-06's existing TUI reference intact. Its compare() consumes
        # idle/active only; P1-08 adds named daemon configurations and CLI data.
        baseline = json.loads(args.write_baseline.read_text())
        baseline['configs'].update(configs)
        baseline['reference'].update(result['reference'])
        baseline['daemon'] = {key:value for key,value in result.items() if key not in ('configs','reference')}
        args.write_baseline.write_text(json.dumps(baseline,indent=2,sort_keys=True)+'\n')
    return suite_status('daemon_budget')
