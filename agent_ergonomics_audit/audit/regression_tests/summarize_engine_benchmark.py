#!/usr/bin/env python3
"""Validate complete paired benchmark receipts and summarize every measured row."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import statistics


def distribution(values):
    values = sorted(values)
    n = len(values)
    assert n
    return {'n': n, 'median': statistics.median(values),
            'mean': statistics.mean(values), 'min': values[0], 'max': values[-1],
            'p95_nearest_rank': values[math.ceil(.95*n)-1],
            'p99_nearest_rank': values[math.ceil(.99*n)-1],
            'p99_9_conservative': values[math.ceil(.999*n)-1],
            'p99_99_conservative': values[math.ceil(.9999*n)-1],
            'sample_cv_percent': statistics.stdev(values)/statistics.mean(values)*100
                if n > 1 and statistics.mean(values) else 0}


def summarize(source):
    data = json.loads(source.read_text())
    rows = data['runs']
    warmups = data['warmups']
    engines = ('baseline_native', 'native', 'qbittorrent')
    pairs = sorted({row['pair'] for row in rows})
    assert pairs == list(range(1, len(pairs)+1))
    assert len(warmups) == 3 and {r['engine'] for r in warmups} == set(engines)
    expected = {(pair, engine) for pair in pairs for engine in engines}
    assert len(rows) == len(expected) and {(r['pair'], r['engine']) for r in rows} == expected
    all_rows = rows + warmups
    payload_bytes = data['payload_mib']*1048576
    receipts = data['seed_receipts']
    for row in all_rows:
        label = f"warmup:{row['engine']}" if row.get('warmup') else f"{row['pair']}:{row['engine']}"
        matches = [r for r in receipts if r['run'] == label and r['bytes_sent'] > 0]
        assert len(matches) == 1 and matches[0] == row['seed_receipt'], label
        receipt = matches[0]
        download = row['download']
        assert receipt['bytes_sent'] == payload_bytes
        assert download['verified_sha256'] == data['payload_sha256']
        assert download['add_started_monotonic'] <= receipt['first_payload_monotonic'] <= receipt['last_payload_monotonic'] <= download['complete_monotonic']
        assert not row['stderr']
        if row['engine'] != 'qbittorrent':
            for key in ('close', 'reclose'):
                assert all(value == 0 for value in row[key]['after_tui_close'].values())
    output = {'source': str(source), 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
              'blocks': len(pairs), 'measured_transfers': len(rows), 'warmups_excluded': len(warmups),
              'verified_payloads_and_joined_receipts': len(all_rows),
              'payload_mib': data['payload_mib'], 'payload_sha256': data['payload_sha256'],
              'binary_sha256': data['binary_sha256'], 'baseline_binary_sha256': data['baseline_binary_sha256'],
              'ratings_sha256': data['ratings_sha256'], 'variants': {},
              'limits': 'All measured rows retained. PSS is app processes only. CPU-s is processor work, not elapsed time. '
                        'Nearest-rank high percentiles converge to max at these small sample sizes; conservative, not extreme-tail qualification. '
                        'Bootstrap interval assumes independent matched blocks; ordinary desktop activity remains.'}
    for engine in engines:
        variant = [r for r in rows if r['engine'] == engine]
        metrics = {}
        for field in ('cpu_seconds', 'user_cpu_seconds', 'system_cpu_seconds', 'seconds',
                      'mib_per_second', 'peak_pss_mib', 'peak_rss_mib', 'peak_threads'):
            metrics[field] = distribution([r['download'][field] for r in variant])
        for field in ('opening_ms', 'reopen_to_completed_ms'):
            metrics[field] = distribution([r[field] for r in variant])
        metrics['status_cli_ms'] = distribution([v for r in variant for v in r['status_cli_ms']])
        for field in ('tui_close_ms', 'total_stop_ms'):
            metrics[field] = distribution([r['close'][field] for r in variant])
        metrics['after_tui_close_pss_mib'] = distribution([r['close']['after_tui_close']['Pss']/1024 for r in variant])
        metrics['cpu_seconds_per_mib'] = distribution([r['download']['cpu_seconds']/data['payload_mib'] for r in variant])
        metrics['cpu_percent_one_core'] = distribution([100*r['download']['cpu_seconds']/r['download']['seconds'] for r in variant])
        metrics['wire_seconds'] = distribution([r['seed_receipt']['last_payload_monotonic']-r['seed_receipt']['first_payload_monotonic'] for r in variant])
        metrics['first_payload_ms'] = distribution([1000*(r['seed_receipt']['first_payload_monotonic']-r['download']['add_started_monotonic']) for r in variant])
        metrics['wire_mib_per_second'] = distribution([data['payload_mib']/(r['seed_receipt']['last_payload_monotonic']-r['seed_receipt']['first_payload_monotonic']) for r in variant])
        for field in ('idle_empty', 'idle_completed'):
            sampled = [r[field] for r in variant if field in r]
            metrics[field] = {k: distribution([r[k] for r in sampled])
                              for k in ('pss_mib', 'rss_mib', 'threads', 'cpu_percent_one_core')}
        output['variants'][engine] = metrics
    savings = []
    for pair in pairs:
        block = {r['engine']: r for r in rows if r['pair'] == pair}
        before = block['baseline_native']['download']['cpu_seconds']
        after = block['native']['download']['cpu_seconds']
        savings.append(100*(before-after)/before)
    output['paired_cpu_reduction_percent'] = distribution(savings)
    output['paired_cpu_reduction_percent']['per_block'] = savings
    if len(pairs) >= 20:
        rng = random.Random(0)
        boot = sorted(statistics.median(rng.choices(savings, k=len(savings))) for _ in range(10000))
        output['paired_cpu_reduction_percent']['bootstrap_95_percent_interval'] = [boot[249], boot[9749]]
        output['paired_cpu_reduction_percent']['bootstrap_resamples'] = 10000
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.write_text(json.dumps(summarize(args.source), indent=2)+'\n')


if __name__ == '__main__':
    main()
