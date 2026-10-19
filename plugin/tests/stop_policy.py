#!/usr/bin/env python3
"""Actual popup client: power drains the owner; glance freezes its idle clock."""
import argparse
import json
from pathlib import Path
import sys
import time
from visual import Capture,ROOT
import daemon as d
import fixtures
import private_net
import fbtlog
from fbtlog import Case,suite_status
from daemon_policy import hello,subscribe,idle_exit


def power(binary):
    with Case('plugin_policy','power') as case,fixtures.daemon_pair(case,binary,False,65536) as pair:
        c=pair.client;c.call('subscribe',dict(topics=['summary'],interval_ms=250))
        q=Capture(case,pair.box,'owner')
        try:
            q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open')
            q.wait(lambda:json.loads(q.ipc('cursor'))['rows']==1)
            case.check(q.ipc('clickPower')=='true','click the real popup power control')
            q.wait(lambda:json.loads(q.ipc('cursor')).get('stopping'))
            cursor=json.loads(q.ipc('cursor'))
            case.check(cursor['confirmFocused']=='cancel' and cursor['selected']==0,
                       'active transfer stop confirmation defaults to Cancel',json.dumps(cursor))
            before=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent']['completed_bytes']
            case.check(q.ipc('clickStopChoice','Cancel')=='true','click the actual Cancel button')
            q.wait(lambda:not json.loads(q.ipc('cursor')).get('stopping'))
            assert pair.h.wait_for(lambda:((r:=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'])['completed_bytes']>before,json.dumps(r)), 'Cancel leaves the real download moving',timeout=5)
            case.check(q.ipc('clickPower')=='true','reopen real power confirmation')
            q.wait(lambda:json.loads(q.ipc('cursor')).get('stopping'))
            case.check(q.ipc('clickStopChoice','Stop')=='true','explicit Stop clicks the actual confirmation button')
            terminal=next(frame for frame in c.events() if frame.get('ev')=='bye')
            pair.p.wait(timeout=10,code=0)
            q.wait(lambda:not json.loads(q.ipc('state'))['online'])
            case.check(terminal.get('reason')=='shutdown' and not pair.h.engine_path.exists() and not json.loads(q.ipc('state'))['idle'],
                       'plugin power produces shutdown bye, clean daemon exit, unlinked socket and cleared countdown',json.dumps(terminal))
            pair.h.lock_free()
        finally:q.close()


def glance(binary):
    with Case('plugin_policy','glance') as case,d.Sandbox() as box,d.Harness(case,binary,box,False) as h:
        # Keep startup unhurried. Configure the short real timer only after the
        # actual plugin is online and its popup presence has been acknowledged.
        p=h.start_serve(idle_exit_secs=30)
        assert h.receive_ready() and h.wait_engine()
        with d.ApiConn(h) as c:
            hello(c,'bar');stream,_=subscribe(c)
            q=Capture(case,box,'owner')
            try:
                q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open')
                q.wait(lambda:json.loads(q.ipc('state'))['opened'])
                c.call('app.settings.set',dict(idle_exit_secs=30))
                # The normal public setting minimum is 30 seconds. No test-only
                # production hook or shortened policy bypass is used here.
                def summary():
                    return next(f['data']['daemon'] for f in c.events() if f.get('topic')=='summary')
                began=time.monotonic();first=summary();samples=[]
                while time.monotonic()-began<31:
                    samples.append(summary())
                values=[s.get('idle_exit_in_s') for s in samples]
                case.check(p.child.poll() is None and values and all(v==values[0] for v in values) and not any(s['busy_reasons'] for s in samples),
                           'real popup glance freezes the countdown beyond the configured 30-second idle delay',json.dumps(dict(first=first,values=values,elapsed=time.monotonic()-began)))
                q.ipc('close');q.wait(lambda:not json.loads(q.ipc('state'))['opened'])
                elapsed=idle_exit(case,h,p,c,timeout=36)
                case.check(25<=elapsed<=36,'closing the popup resumes its remaining real countdown and exits idle',json.dumps(dict(elapsed=elapsed)))
                h.lock_free()
            finally:q.close()


def default_delay(binary):
    with Case('plugin_policy','default_delay') as case,d.Sandbox() as box,d.Harness(case,binary,box,False) as h:
        p=h.start_serve(idle_exit_secs=None)
        assert h.receive_ready() and h.wait_engine()
        with d.ApiConn(h) as c:
            hello(c,'bar');_,snapshot=subscribe(c)
            config=c.call('app.settings.get')['result']
            case.check(config['idle_exit_secs']==300 and 290<=snapshot.get('idle_exit_in_s',0)<=300,
                       'serve without any idle override uses the actual 300-second config default',json.dumps(dict(settings=config,snapshot=snapshot)))
            c.call('app.settings.set',dict(idle_exit_secs=29),error='invalid_args')
            c.call('app.settings.set',dict(idle_exit_secs=30))
            case.check(c.call('app.settings.get')['result']['idle_exit_secs']==30,
                       'public config rejects 29 seconds and persists its allowed 30-second minimum')
        h.stop_serve(p);h.lock_free()


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--case',choices=['power','glance','default_delay'],required=True);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    private_net.enter()
    globals()[args.case](args.binary.resolve())
    sys.exit(suite_status('plugin_policy'))
