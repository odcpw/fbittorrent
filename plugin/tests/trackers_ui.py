#!/usr/bin/env python3
"""Real tracker failure/force/pause and credentials through a private dashboard."""
import argparse
import json
from pathlib import Path
import sys
import time
from dashboard import ROOT,Session
import daemon
import fixtures
import native
import private_net
from fbtlog import Case,suite_status


def run(binary):
    private_net.enter()
    with daemon.api_owned('tracker_dashboard',binary,False) as (case,box,h,process),fixtures.tracker_server(fail_first=1,interval_s=600,peers=[('127.0.0.1',9)],directory=box.root,echo_failure=True) as tracker,daemon.ApiConn(h,redact_urls=True) as client:
        client.hello()
        # HTTP userinfo is deliberately unsupported by the runtime; its safe
        # presentation is exercised by tracker-view. Use an admitted query
        # credential here so this journey proves an actual successful retry.
        url=tracker.url+'?passkey=ui-private-query'
        fixture=daemon.tracker_live_fixture(box,url,name='tracker-dashboard.bin')
        client.call('torrents.add',dict(source=dict(path=str(fixture.torrent))))
        h.wait_for(lambda:((rows:=daemon.tracker_rows(client,fixture.info_hash))[0]['status']=='error',client.diagnostic(rows)),'real first announce returns a failure')
        native.prepare_imdb_fixture(box.env,['tracker-dashboard.bin'])
        topic='torrent:'+fixture.info_hash+'.trackers'
        def subscriptions():return client.call('app.info')['result']['subscriptions']['by_topic']
        q=Session(case,box)
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            q.shell('summon',json.dumps(dict(view='torrents',hash=fixture.info_hash)))
            q.wait(lambda:(q.state().get('view')or{}).get('cursor')==fixture.info_hash)
            q.keys(['-k','Return'],lambda:(q.state().get('view')or{}).get('focus')=='details')
            case.check(q.ipc('clickDetailTab','trackers')=='true','actual Trackers tab button opens the view')
            q.wait(lambda:subscriptions().get(topic)==1)
            def text():return '\n'.join(json.loads(q.ipc('trackerRows')))
            q.wait(lambda:'Error · Peers —' in text() and 'fixture: not yet' in text())
            before=text()
            case.check(all(secret not in before for secret in ('ui-private-user','ui-private-password','ui-private-query','passkey=')) and '/announce' in before and 'Last ' in before,'tracker failure, announce time and unknown peer count render without credentials',before)
            image=case.path.with_suffix('.error.png');q.ipc('grab',str(image));q.wait(lambda:image.exists())
            started=time.monotonic()
            case.check(q.ipc('trackerReannounce')=='true','actual Reannounce button sends the explicit force')
            h.wait_for(lambda:(len(tracker.snapshot())==2,json.dumps(tracker.snapshot())),'force reaches real tracker once within two seconds',timeout=2)
            q.wait(lambda:'OK · Peers 1' in text())
            case.check(time.monotonic()-started<2,'pushed success updates the visible dashboard without a read/refresh action',text())
            case.check(q.ipc('trackerReannounce')=='true','a second explicit activation uses the same action')
            time.sleep(.35);case.check(len(tracker.snapshot())==2,'backend coalesces repeated force for sixty seconds',json.dumps(tracker.snapshot()))
            client.call('torrents.stop',dict(hashes=[fixture.info_hash]))
            q.wait(lambda:'OK · Peers 1' in text())
            case.check('Last —' not in text(),'paused view retains its last real announce and peer count',text())
            value=h.cli('show',fixture.info_hash);shown=json.dumps(value)
            case.check(value['trackers'][0]['url']==tracker.url and all(secret not in shown for secret in ('ui-private-user','ui-private-password','ui-private-query','passkey=')),'ordinary CLI diagnostics strip credentials while retaining the tracker endpoint',shown)
            case.check(q.ipc('clickDetailTab','overview')=='true','leaving tracker detail uses the actual Overview button')
            q.wait(lambda:subscriptions().get(topic,0)==0)
            case.check(subscriptions().get(topic,0)==0,'hidden tracker details release their sampler',json.dumps(subscriptions()))
            case.check(q.ipc('clickDetailTab','trackers')=='true','return to the paused tracker view')
            q.wait(lambda:'OK · Peers 1' in text());q.shell('hide');q.wait(lambda:q.state().get('view')is None)
            q.wait(lambda:subscriptions().get(topic,0)==0)
            case.check(subscriptions().get(topic,0)==0,'closing the dashboard releases its final tracker observation',json.dumps(subscriptions()))
        finally:q.close()
        logs=h.stderr()+case.path.read_text()+tracker.log.read_text()+case.path.with_suffix('.qs.log').read_text()
        case.check(all(secret not in logs for secret in ('ui-private-user','ui-private-password','ui-private-query','passkey=')),'daemon, QML, harness and tracker diagnostics contain no credentials','Scanned private diagnostic logs; the complete URL is retained only in owned store/socket data.')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,default=ROOT/'target/debug/fbittorrent')
    args=parser.parse_args();run(args.binary.resolve());sys.exit(bool(suite_status('daemon')))
