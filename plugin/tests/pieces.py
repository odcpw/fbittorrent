#!/usr/bin/env python3
"""Verified Pieces diagnostics from a real, deliberately incomplete private torrent."""
import argparse
import base64
import datetime
import json
from pathlib import Path
import shutil
import subprocess
import time
from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status


def detail(q):
    return (q.state().get('view') or {}).get('details') or {}


def run(binary):
    previous=ROOT/'target/test-logs/qs_pieces_live'
    if previous.exists():
        shutil.copytree(previous,ROOT/'target/test-logs/fbt-p6-02-bw1/runs'/datetime.datetime.now().strftime('%H%M%S'))
    with Case('qs_pieces_live','fixture') as setup,fixtures.Sandbox() as box:
        source=box.root/'source';source.mkdir()
        payload=source/'fixture.mkv';piece=16384
        payload.write_bytes(b'\x11'*(1000*piece))
        torrent=fixtures.torrent_fixture(source,[payload])
        target=box.downloads/payload.name
        with target.open('wb') as output:
            output.truncate(1000*piece)
            output.write(b'\x11'*(256*piece))
            output.seek(999*piece);output.write(b'\x11'*piece)
        with daemon.Harness(setup,binary,box,False) as harness:
            process=harness.start_serve(idle_exit_secs=0)
            if not (harness.receive_ready() and harness.wait_engine()):raise RuntimeError('owned Pieces daemon did not start')
            with daemon.ApiConn(harness) as client:
                hello=client.hello()['result']
                if 'pieces' not in hello['caps']:raise RuntimeError('positive test requires the real pieces service')
                client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(box.downloads),stopped=True))
                if not harness.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['checked'],json.dumps(row)),'owned1000-piece check completes',timeout=12):raise RuntimeError('check did not complete')
                if not harness.wait_for(lambda:((value:=client.call('torrents.pieces',dict(hash=torrent.info_hash))['result']).get('playable_end')==256,json.dumps(value)),'real watch service samples the verified playable prefix',timeout=6):raise RuntimeError('prefix was not sampled')
                data=client.call('torrents.pieces',dict(hash=torrent.info_hash))['result']
                expected=json.loads((fixtures.HERE/'fixtures/states/pieces-1000.json').read_text())
                setup.check(data['count']==1000 and data['have']==expected['have'] and data['priority']==[0,999] and data.get('playable_end')==256,
                            'real paused handle exposes exactly first256 plus final verified pieces',json.dumps(data))
                client.call('torrents.pieces',dict(hash='0'*40),error='not_found')
                q=Session(setup,box)
                try:
                    q.compositor.ipc('output * mode 1920x1080')
                    q.wait(lambda:q.ipc('ready')=='true')
                    q.shell('summon',json.dumps(dict(view='torrents',hash=torrent.info_hash)))
                    q.wait(lambda:detail(q).get('hash')==torrent.info_hash)
                    q.wait(lambda:'no DHT, no PeX' in detail(q).get('networking',''))
                    setup.check(True,'real dashboard Overview identifies private networking',detail(q)['networking'])
                    with Case('qs_pieces_live','pieces_bar') as case:
                        # Click the actual Pieces control directly; unrelated
                        # diagnostic tabs are exercised by their own cases.
                        case.check(q.ipc('detailTab','pieces')=='true','actual Pieces tab button accepts a native pointer click','Pieces control found and clicked')
                        q.wait(lambda:detail(q).get('tab')=='pieces')
                        q.wait(lambda:(detail(q).get('pieces') or {}).get('count')==1000 and detail(q)['pieces'].get('playable_end')==256)
                        shown=detail(q)['pieces']
                        case.check(shown['have']==257 and shown['priority']==[0,999] and shown.get('playable_end')==256,
                                   'state().view.details.pieces mirrors the actual verified bitfield',json.dumps(shown))
                        topics=client.call('app.info')['result']['subscriptions']['by_topic']
                        topic='torrent:'+torrent.info_hash+'.pieces'
                        case.check(topics.get(topic)==1,'one visible Pieces pane owns exactly one sampled detail topic',json.dumps(topics))
                        image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
                        q.shell('hide')
                        q.wait(lambda:topic not in client.call('app.info')['result']['subscriptions']['by_topic'])
                        case.check(q.state().get('view') is None,'hiding the dashboard releases Pieces sampling and its view',json.dumps(q.state()))
                finally:q.close()
                with Case('qs_pieces_live','tui_subscription') as case:
                    socket='fbt-fbt-p6-02-bw1';session=socket
                    env=dict(box.env,FBITTORRENT_NO_START='1')
                    def tmux(*args,check=True):
                        return subprocess.run(['tmux','-L',socket,*args],env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=check,timeout=4)
                    def capture():return tmux('capture-pane','-p','-t',session).stdout
                    def wait(predicate):
                        deadline=time.monotonic()+10
                        while time.monotonic()<deadline:
                            if predicate():return
                            time.sleep(.05)
                        raise TimeoutError(capture())
                    try:
                        tmux('new-session','-d','-s',session,'-x','120','-y','30',str(binary),'tui')
                        wait(lambda:'fixture.mkv' in capture())
                        tmux('send-keys','-t',session,'Enter')
                        wait(lambda:'[Overview]' in capture())
                        wait(lambda:'no DHT, no PeX' in capture())
                        case.check(True,'real TUI Overview identifies private networking',capture())
                        # Each navigation key is a real key event. A pasted
                        # multi-character sequence is deliberately not navigation.
                        for label in ('Files','Peers','Trackers','Log','Pieces'):
                            tmux('send-keys','-t',session,'-l',']')
                            wait(lambda:'['+label+']' in capture())
                        wait(lambda:'257 / 1000 verified pieces' in capture())
                        text=capture();case.path.with_suffix('.txt').write_text(text)
                        case.check('[Pieces]' in text and '▲' in text and '▒' in text,'actual TUI consumes the real verified piece subscription',text)
                        lines=text.splitlines()
                        filled=next((i for i,line in enumerate(lines) if '█' in line and '▒' in line and 'verified' not in line),None)
                        marked=next((i for i,line in enumerate(lines) if '▲' in line and 'first/last' not in line),None)
                        case.check(filled is not None and marked==filled+1 and '▲' not in lines[filled],
                                   'real TUI keeps verified/missing completion visible with priority on the following row',text)
                        topic='torrent:'+torrent.info_hash+'.pieces'
                        case.check(client.call('app.info')['result']['subscriptions']['by_topic'].get(topic)==1,'TUI owns one visible sampled detail topic','one subscription')
                        tmux('send-keys','-t',session,'Escape')
                        wait(lambda:topic not in client.call('app.info')['result']['subscriptions']['by_topic'])
                        case.check(True,'closing TUI Details releases its Pieces subscription','absent in real app.info')
                        tmux('send-keys','-t',session,'-l','q')
                        wait(lambda:tmux('has-session','-t',session,check=False).returncode!=0)
                        case.check(client.call('app.info')['ok'],'TUI detach leaves the owned daemon running','app.info success')
                    finally:tmux('kill-server',check=False)
            harness.stop_serve(process);harness.lock_free()
    return suite_status('qs_pieces_live')


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
