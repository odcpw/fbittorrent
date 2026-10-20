#!/usr/bin/env python3
"""Summarize only complete, independently verified client comparison pairs."""
import argparse
import json
from pathlib import Path
import statistics
from head_to_head import WORKLOADS

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('directory',type=Path)
a=p.parse_args()
arguments=json.loads((a.directory/'manifest.json').read_text())['arguments']
assert not any(arguments[k] for k in ('profile','strace','pilot_mib','public_torrent'))
groups={}
for path in sorted(a.directory.glob('*/result.json')):
    row=json.loads(path.read_text())
    if row['instrumented']:continue
    assert not (path.parent/'failure.json').exists(),path
    teardown=json.loads((path.parent/'teardown.json').read_text())
    assert teardown['qualified'] and teardown['survivors']==0 and not teardown['emergency_cleanup'],path
    key=(row['bytes'],row['files'],row['peers'])
    groups.setdefault(key,{}).setdefault(row['client'],[]).append(row)
expected={(WORKLOADS[w][0],WORKLOADS[w][1],n) for w in arguments['workloads'] for n in arguments['seeds']}
assert set(groups)==expected, 'scheduled comparison cells are missing'
result=[]
for (size,files,peers),clients in groups.items():
    assert set(clients)=={'fbt','qbt'},clients.keys()
    assert len(clients['fbt'])==len(clients['qbt'])==arguments['runs']
    assert len({r['info_hash'] for rows in clients.values() for r in rows})==1
    assert len({r['payload_sha256'] for rows in clients.values() for r in rows})==1
    summary=dict(bytes=size,files=files,seeds=peers,runs=len(clients['fbt']),clients={})
    for client,rows in clients.items():
        def stat(values):
            return dict(median=statistics.median(values),minimum=min(values),maximum=max(values),values=values)
        stats={}
        for key in ('seconds','first_piece_s','playable_s','prefix_s','app_ready_s','cpu_s','seed_cpu_s','peak_rss_kb','write_calls','write_bytes','uploaded_bytes'):
            values=[r[key] for r in rows if r.get(key) is not None]
            if values:stats[key]=stat(values)
        if all('recheck' in r for r in rows):
            stats['recheck']={k:stat([r['recheck'][k] for r in rows]) for k in ('seconds','cpu_s','peak_rss_kb','read_bytes')}
        summary['clients'][client]=stats
    result.append(summary)
print(json.dumps(result,indent=2))
