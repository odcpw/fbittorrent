#!/usr/bin/env python3
"""One real seed/leecher journey through dashboard Peers, popup and the bar."""
import argparse
import json
from pathlib import Path
import sys
import time
from dashboard import ROOT,Session
from visual import Capture
import daemon
import fixtures
import native
import private_net
from daemon_peers import Swarm
from fbtlog import Case,suite_status


def run(binary):
    private_net.enter()
    with Case('qs_peers','live_views') as case,Swarm(case,binary,False,payload_bytes=32*1024*1024,upload_bps=262144) as swarm:
        if not swarm.h.wait_for(swarm.observe,'actual seed and incomplete leecher connect',timeout=20):return
        native.prepare_imdb_fixture(swarm.box.env,['peer-payload.bin'])
        topic='torrent:'+swarm.hash+'.peers'
        def subscriptions():return swarm.client.call('app.info')['result']['subscriptions']['by_topic']
        q=Session(case,swarm.box)
        phase="dashboard ready"
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            phase='open dashboard and select current torrent'
            q.shell('summon',json.dumps(dict(view='torrents',hash=swarm.hash)))
            q.wait(lambda:(q.state().get('view')or{}).get('cursor')==swarm.hash)
            phase='dashboard connected count cell'
            q.wait(lambda:q.ipc('peerCountCell')=='1/2')
            case.check(q.ipc('peerCountCell')=='1/2','actual dashboard S/P cell shows connected seed/peer counts',q.ipc('peerCountCell'))
            phase='open visible Peers tab'
            q.ipc('resize',900,600)
            q.keys(['-k','Return'],lambda:(q.state().get('view')or{}).get('focus')=='details')
            def details():return (q.state().get('view')or{}).get('details')or{}
            case.check(subscriptions().get(topic,0)==0,'Overview does not sample hidden peers',json.dumps(subscriptions()))
            q.keys([']]'],lambda:details().get('tab')=='peers')
            q.wait(lambda:subscriptions().get(topic)==1)
            q.wait(lambda:len(json.loads(q.ipc('peerRows')))==2)
            def moving():
                peers=swarm.client.call('torrents.peers',dict(hash=swarm.hash))['result']['peers']
                return any(p.get('dl_bps',0)>0 for p in peers),json.dumps(peers)
            swarm.h.wait_for(moving,'real loopback payload produces measurable peer rates',timeout=12)
            phase='observe rendered peer fields and changing payload'
            first=json.loads(q.ipc('peerRows'))
            rendered='\n'.join(first)
            case.check(all(p['addr'] in rendered for p in swarm.peers) and 'seed yes' in rendered and 'seed no' in rendered and 'OUT' in rendered and 'TCP' in rendered and 'Downloaded' in rendered and 'uploaded' in rendered and '↓' in rendered and '↑' in rendered,
                       'actual dashboard delegates display addresses, clients, direction, transport, seed bits, counters and rates',rendered)
            before=swarm.client.call('torrents.peers',dict(hash=swarm.hash))['result']['peers']
            time.sleep(2)
            second=json.loads(q.ipc('peerRows'));after=swarm.client.call('torrents.peers',dict(hash=swarm.hash))['result']['peers']
            old={p['addr']:p['downloaded']for p in before}
            case.check(first!=second and any(p['downloaded']>old.get(p['addr'],p['downloaded'])for p in after) and any(p.get('dl_bps',0)>0 for p in after),
                       'rendered live rates/counters move with real loopback payload',json.dumps(dict(before=first,after=second,peers=after)))
            image=case.path.with_suffix('.dashboard.png');q.ipc('grab',str(image));q.wait(lambda:image.exists())
            phase='leave Peers using the Files tab button'
            case.check(q.ipc('clickDetailTab','files')=='true','actual Files tab button is clickable')
            q.wait(lambda:details().get('tab')=='files')
            q.wait(lambda:subscriptions().get(topic,0)==0)
            case.check(not json.loads(q.ipc('peerRows')),'leaving Peers destroys peer delegates and releases its sampler',json.dumps(subscriptions()))
            phase='return to Peers using the tab button'
            case.check(q.ipc('clickDetailTab','peers')=='true','actual Peers tab button is clickable')
            q.wait(lambda:details().get('tab')=='peers');q.wait(lambda:subscriptions().get(topic)==1)
            phase='hide dashboard and release its sampler'
            q.shell('hide');q.wait(lambda:q.state().get('view') is None);q.wait(lambda:subscriptions().get(topic,0)==0)
            case.check(subscriptions().get(topic,0)==0,'hiding dashboard releases the final peer sampler',json.dumps(subscriptions()))
        except Exception:
            case.check(False,phase,json.dumps(dict(state=q.state(),count_cell=q.ipc('peerCountCell'),rows=q.ipc('peerRows'))))
            raise
        finally:q.close()
        # A distinct client HOME/cache/config, sharing only this owned daemon's
        # runtime directory. No owner display or data is involved.
        with fixtures.Sandbox() as popupbox:
            popupbox.env['XDG_RUNTIME_DIR']=swarm.box.env['XDG_RUNTIME_DIR']
            popup=Capture(case,popupbox,'owner')
            try:
                popup.wait(lambda:json.loads(popup.ipc('state'))['online'])
                popup.wait(lambda:'1 seeds / 2 peers connected' in json.loads(popup.ipc('state'))['bar']['tooltip'])
                state=json.loads(popup.ipc('state'))
                case.check('peer-payload.bin:' in state['bar']['tooltip'] and not state['opened'] and subscriptions().get(topic,0)==0,
                           'closed bar shows current named connected counts without a peer subscription',json.dumps(state['bar']))
                popup.ipc('open');popup.wait(lambda:json.loads(popup.ipc('state'))['opened'])
                popup.wait(lambda:len(json.loads(popup.ipc('peerLabels')))>0)
                labels=json.loads(popup.ipc('peerLabels'))
                case.check(any('1 seeds / 2 peers connected' in label for label in labels),'real popup list exposes truthful connected seeds and peers',json.dumps(labels))
                popup.capture(case.path.with_suffix('.popup.png'))
                popup.ipc('close');popup.wait(lambda:not json.loads(popup.ipc('state'))['opened'])
                case.check(subscriptions().get(topic,0)==0,'popup and closed bar never acquire peer-detail sampling',json.dumps(subscriptions()))
            finally:popup.close()


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,default=ROOT/'target/debug/fbittorrent')
    args=parser.parse_args();run(args.binary.resolve());sys.exit(bool(suite_status('qs_peers')))
