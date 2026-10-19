#!/usr/bin/env python3
"""Confirm a real private hash check from the dashboard, including its receipt."""
import argparse
import json
from pathlib import Path
import sys
from dashboard import ROOT
from journey_support import JourneySession
import daemon
import fixtures
from fbtlog import Case,suite_status


def run(binary):
    with Case('qs_recheck','accepted_check') as case,daemon.Sandbox() as box,daemon.Harness(case,binary,box) as h:
        payload=box.downloads/'owned-check.bin'
        with payload.open('wb') as output:
            for _ in range(128):output.write(b'owned hash check'*65536)
        size=payload.stat().st_size;piece=256*1024
        fixture=fixtures.torrent_fixture(box.root,[payload],piece_length=piece)
        sentinel=box.downloads/'unrelated.txt';sentinel.write_text('keep my data')
        process=h.start_serve();h.receive_ready();h.wait_engine();q=None
        try:
            with daemon.ApiConn(h) as client:
                client.hello()
                client.call('torrents.add',dict(source=dict(path=str(fixture.torrent)),save_path=str(box.downloads),stopped=True))
                def row():return client.call('torrents.get',dict(hash=fixture.info_hash))['result']['torrent']
                if not h.wait_for(lambda:((r:=row())['checked'] and r['state']=='paused',json.dumps(r)),'existing complete payload is checked with paused intent',timeout=15):raise RuntimeError('initial check failed')
                fixtures.corrupt_byte(payload,0)
                q=JourneySession(case,box)
                def view():return q.state().get('view') or {}
                def feedback():return json.loads(q.ipc('recheckState'))
                def actions():return [a for a in feedback()['actions'].values() if a['op']=='torrents.recheck']
                q.wait(lambda:q.ipc('ready')=='true')
                q.shell('summon',json.dumps(dict(view='torrents',hash=fixture.info_hash)))
                q.wait(lambda:view().get('cursor')==fixture.info_hash)
                q.keys(['-k','Return'],lambda:view().get('focus')=='details')
                q.wait(lambda:feedback()['button'] and feedback()['button']['enabled'])
                assert q.ipc('scrollRecheck')=='true'
                q.wait(lambda:(f:=feedback())['button']['y']>=0 and f['button']['y']+f['button']['height']<=f['viewport']['height'] and not f['viewport']['moving'])
                case.check(True,'Overview exposes the enabled Recheck button',json.dumps(feedback()))
                assert q.ipc('recheckButton')=='true'
                q.wait(lambda:(view().get('dialog') or {}).get('kind')=='recheck')
                q.keys(['-k','Return'],lambda:not view().get('dialog'))
                case.check(not actions(),'Recheck confirmation defaults to Cancel','no mutation was sent')
                assert q.ipc('recheckButton')=='true'
                q.wait(lambda:(view().get('dialog') or {}).get('kind')=='recheck')
                q.keys(['-k','Right','-k','Return'],lambda:not view().get('dialog') and bool(actions()))
                q.wait(lambda:actions()[0]['status']=='done',timeout=15)
                result=feedback();action=actions()[0]
                case.check(action.get('accepted') and not result['error'] and result['message']=='Recheck complete','real pending acknowledgement resolves to visible completion without no-answer feedback',json.dumps(result))
                final=row()
                events=json.loads(q.ipc('eventTrace'))
                receipt=next(e for e in events if e.get('ev')=='mutation.result' and e.get('op')=='torrents.recheck')
                case.check(final['intent']=='pause' and final['state']=='paused' and receipt['result']['have_bytes'][fixture.info_hash]==size-piece,'check measures the damaged piece and preserves paused intent',json.dumps(dict(torrent=final,receipt=receipt)))
                q.keys(['4'],lambda:(view().get('window') or {}).get('view')=='rss')
                q.keys(['1'],lambda:(view().get('window') or {}).get('view')=='torrents')
                case.check(not feedback()['error'] and feedback()['message']=='Recheck complete','switching screens retains confirmed completion, without a sticky timeout',json.dumps(feedback()))
                case.check(sentinel.read_text()=='keep my data','verification preserves unrelated files','sentinel unchanged')
                image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
                q.keys(['q'],lambda:q.state().get('view') is None)
                case.check(process.child.poll() is None,'closing the dashboard detaches from the same daemon','owner still running')
        except BaseException:
            if q:
                case.path.with_suffix('.state.json').write_text(json.dumps(dict(state=q.state(),feedback=feedback()),indent=2))
                q.ipc('grab',case.path.with_suffix('.failure.png'))
            raise
        finally:
            try:
                if q:q.close()
            finally:
                if process.child.poll() is None:h.stop_serve(process)
    return suite_status('qs_recheck')


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    args=parser.parse_args();sys.exit(run(args.binary.resolve()))
