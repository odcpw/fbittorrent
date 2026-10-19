#!/usr/bin/env python3
"""Normative budget threshold tests; these never launch an application."""
import copy
import math
import sys
from fbtlog import Case, suite_status
from resources_daemon import compare_baseline, bar_budget


def main():
    baseline = dict(daemon=dict(host='fixture', seconds=60), configs={key:dict(cpu_s=[10,10,10],rss_kb=[100,105,110]) for key in ('10x0','10x1','100x8')})
    current = dict(host='fixture', seconds=60, configs={key:dict(cpu_s=[11],rss_kb=[135]) for key in baseline['configs']})
    with Case('daemon_budget_test','thresholds') as case:
        result = compare_baseline(baseline,current)
        case.check(len(result)==6 and all(r['ok'] for r in result), '10-percent CPU floor and three-spread RSS boundary both pass for every named workload', str(result))
        for key in current['configs']:
            for metric in ('cpu_s','rss_kb'):
                planted=copy.deepcopy(current)
                planted['configs'][key][metric].append(max(planted['configs'][key][metric])+.001)
                case.check(sum(not r['ok'] for r in compare_baseline(baseline,planted))==1,
                           f'a single {key}/{metric} regression cannot be averaged away', str(planted))
    with Case('daemon_budget_test','kernel_tick_boundaries') as case:
        configs={f'{n}x{k}':dict(cpu_s=[.26 if k==0 else .28]*3) for n in (10,100) for k in (0,1)}
        references={str(n):[dict(cpu_s=.02)]*3 for n in (10,100)}
        case.check(all(row['ok'] for row in bar_budget(configs,references)),
                   'two additional kernel CPU ticks equal a two-tick CLI reference exactly', str(bar_budget(configs,references)))
        configs['100x1']['cpu_s']=[.29]*3
        case.check([row['ok'] for row in bar_budget(configs,references)] == [True,False],
                   'one tick over the reference fails without a tolerance', str(bar_budget(configs,references)))
    with Case('daemon_budget_test','reference_and_samples') as case:
        for change in ('host','seconds','missing','two','negative','nan'):
            wanted=copy.deepcopy(baseline); measured=copy.deepcopy(current)
            if change=='host': measured['host']='other'
            elif change=='seconds': measured['seconds']=1
            elif change=='missing': del wanted['configs']['10x0']
            elif change=='two': wanted['configs']['10x0']['cpu_s']=[10,10]
            elif change=='negative': measured['configs']['10x0']['cpu_s']=[-1]
            else: measured['configs']['10x0']['cpu_s']=[math.nan]
            try: compare_baseline(wanted,measured)
            except ValueError as error: case.check(isinstance(error, ValueError), 'invalid '+change+' observation is refused', str(error))
            else: case.check(False, 'invalid '+change+' observation is refused', str(measured))
    return suite_status('daemon_budget_test')


if __name__=='__main__':
    sys.exit(main())
