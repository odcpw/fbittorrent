#!/usr/bin/env python3
"""A real player keeps a paused transfer's owner alive, then idle expiry stops it."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[2]/'plugin/tests'))
import daemon as d
import fbtlog
import fixtures
import private_net
from daemon_policy import fresh
from dashboard import Session
from watch_media import Player
from watch_player import swarm,row


def run(binary,source,seconds,log):
    fbtlog.LOG_ROOT=log.resolve()
    with fbtlog.Case('lifecycle_idle','player') as case,fixtures.Sandbox() as box:
        player=Player(case,box);player.argv.append('--speed=0.05')
        with swarm(case,binary,source,box,player) as (h,p,client,peer,item,magnet):
            client.call('torrents.add',dict(source=dict(magnet=magnet),save_path=str(box.downloads)))
            assert h.wait_for(lambda:((r:=row(client,item.info_hash)).get('playable',{}).get('ready'),json.dumps(r)),
                              'verified video becomes ready without any interactive client',timeout=60)
            target=client.call('torrents.open_target',dict(hash=item.info_hash))['result']
            client.call('torrents.stop',dict(hashes=[item.info_hash]))
            player.child=subprocess.Popen([*player.argv,target['stream_url']],env=box.env,stdout=player.stream,stderr=subprocess.STDOUT)
            bar=None
            try:
                player.attach(player.child)
                deadline=time.monotonic()+10
                while (player.get('time-pos') or 0)<.1:
                    assert time.monotonic()<deadline;time.sleep(.1)
                client.call('app.settings.set',dict(idle_exit_secs=seconds))
                started=time.monotonic();before=player.get('time-pos');snapshots=[]
                while time.monotonic()-started<seconds+2:
                    assert p.child.poll() is None,'daemon exited while a real player was streaming'
                    assert not player.diagnostics(),player.diagnostics()[-10:]
                    snapshots.append(fresh(h));time.sleep(.5)
                assert case.check((player.get('time-pos') or 0)>before and 'playback' in snapshots[-1]['busy_reasons']
                    and 'idle_exit_in_s' not in snapshots[-1],
                    'real playback survives past idle expiry and its busy reason suppresses the countdown',json.dumps(snapshots[-1]))
                # The actual bar remains a non-interactive observer throughout the idle countdown.
                bar=Session(case,box,recipe='bar-host');bar.wait(lambda:bar.ipc('ready')=='true')
                bar.wait(lambda:bar.state()['appearance']['state']=='playback')
                assert case.check('Player connected' in bar.state()['appearance']['tooltip'],'real bar explains why active playback keeps the owner alive',json.dumps(bar.state()))
                quit_started=time.monotonic();player.close();ended=time.monotonic()
                assert h.wait_for(lambda:('idle_exit_in_s' in (s:=fresh(h)) and s['idle_exit_in_s']>0,json.dumps(s)),
                                  'last player disconnect starts a fresh idle countdown',timeout=8)
                bar.wait(lambda:bar.state().get('appearance',{}).get('state')=='idle')
                deadline=ended+seconds+10
                while p.child.poll() is None:
                    assert time.monotonic()<deadline,'idle daemon did not exit';time.sleep(.5)
                assert case.check(p.child.returncode==0 and time.monotonic()-quit_started>=seconds-1 and time.monotonic()-ended<=seconds+10,
                    'idle owner exits at the configured real-time deadline',json.dumps(dict(seconds=time.monotonic()-ended,exit=p.child.returncode)))
                bar.wait(lambda:bar.state().get('online') is False and bar.state().get('appearance',{}).get('state')=='stopped_recently')
                assert case.check('idle' in bar.state()['appearance']['tooltip'].lower(),
                                  'real bar icon reports idle shutdown',json.dumps(bar.state()))
            finally:
                player.close()
                if bar:bar.close()
    return fbtlog.suite_status('lifecycle_idle')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('binary',type=Path);parser.add_argument('source',type=Path)
    parser.add_argument('--seconds',type=int,default=300);parser.add_argument('--log-root',type=Path,required=True)
    args=parser.parse_args();private_net.enter();sys.exit(run(args.binary.resolve(),args.source.resolve(),args.seconds,args.log_root))
