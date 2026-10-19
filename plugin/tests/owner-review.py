#!/usr/bin/env python3
"""Reported row, query and completed-Pieces behaviours on a private real desktop."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status

SUITE='qs_owner_review'
def peek(client,op,args=None):return client.response(client.send(op,args or {}))
def view(q):return q.state().get('view') or {}
def rendered(q,hash):
    return next((r['texts'] for r in json.loads(q.ipc('renderedRows')) if r['hash']==hash),[])
def detail_texts(q):return json.loads(q.ipc('detailTexts'))


def run(binary,which):
    if which=='popup':return popup(binary)
    if which=='review':return review(binary)
    if which=='stop':return stop(binary)
    with Case(SUITE,which) as case,fixtures.Sandbox() as box,daemon.Harness(case,binary,box,False) as h:
        process=h.start_serve(idle_exit_secs=0)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private daemon startup failed')
        with daemon.ApiConn(h) as client:
            client.hello()
            if which=='search':
                q=Session(case,box)
                try:
                    q.wait(lambda:q.ipc('ready')=='true')
                    q.shell('summon',json.dumps(dict(view='search',query='YTS · Latest movies')))
                    q.wait(lambda:view(q).get('focus')=='query' and view(q).get('query')=='YTS · Latest movies')
                    q.compositor.ipc('[title="^fbittorrent$"] focus')
                    q.keys(['-M','ctrl','a','-m','ctrl','Get Jiro S01E01'],lambda:view(q).get('query')=='Get Jiro S01E01')
                    case.check(True,'Ctrl+A replaces the entire focused Search query through native typing',json.dumps(view(q)))
                    q.keys(['-k','Home','New '],lambda:view(q).get('query')=='New Get Jiro S01E01')
                    case.check(True,'caret editing inserts at Home instead of forcing append-only input',view(q)['query'])
                    q.keys(['-k','Down'],lambda:view(q).get('focus')=='results')
                    q.keys(['/'],lambda:view(q).get('focus')=='query')
                    q.keys(['-M','ctrl','a','-m','ctrl','Replacement'],lambda:view(q).get('query')=='Replacement')
                    case.check(True,'returning from results preserves selectable editable query text',view(q)['query'])
                finally:q.close()
            else:
                source=box.root/'source';source.mkdir()
                payload=source/'Refresh fixture.bin';payload.write_bytes(b'x'*262144)
                torrent=fixtures.torrent_fixture(source,[payload]);hash=torrent.info_hash
                client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),stopped=True))
                if not h.wait_for(lambda:((r:=peek(client,'torrents.get',dict(hash=hash))['result']['torrent'])['state']=='paused',json.dumps(r)),'paused private fixture'):
                    raise RuntimeError('private paused fixture did not settle')
                q=Session(case,box)
                try:
                    q.compositor.ipc('output * mode 1920x1080')
                    q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=hash)))
                    q.wait(lambda:'Paused' in rendered(q,hash))
                    client.call('torrents.start',dict(hashes=[hash]))
                    shutil.copyfile(payload,box.downloads/payload.name)
                    client.call('torrents.recheck',dict(hashes=[hash]))
                    q.wait(lambda:peek(client,'torrents.get',dict(hash=hash))['result']['torrent']['checked'],timeout=15)
                    q.wait(lambda:peek(client,'torrents.get',dict(hash=hash))['result']['torrent']['state']=='done',timeout=6)
                    q.wait(lambda:'Done' in rendered(q,hash),timeout=6)
                    case.check(True,'real completion changes the rendered table state without sorting',json.dumps(rendered(q,hash)))
                    q.compositor.ipc('[title="^fbittorrent$"] focus')
                    client.call('torrents.rename',dict(hash=hash,name='Renamed fixture'))
                    q.wait(lambda:'Renamed fixture' in rendered(q,hash),timeout=6)
                    case.check(True,'confirmed daemon rename changes the rendered NAME cell without sorting',json.dumps(rendered(q,hash)))
                finally:q.close()
        h.stop_serve(process)
        if which=='pieces':
            q.config.rename(box.root/'before-restart-shell')
            for log in (q.log,q.compositor.log):shutil.copyfile(log,log.with_suffix('.before-restart.log'))
            h.notify.close()
            process=h.start_serve(idle_exit_secs=0)
            if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private restored daemon startup failed')
            with daemon.ApiConn(h) as client:
                client.hello();row=peek(client,'torrents.get',dict(hash=hash))['result']['torrent']
                case.check(row['state']=='done' and not row['checked'],'restart restores completed data without starting a check',json.dumps(row))
                result=client.call('torrents.pieces',dict(hash=hash),error='invalid_state')
                case.check('checking' not in result['error']['message'].lower(),'Pieces API does not claim a dormant completed torrent is checking',json.dumps(result))
                q=Session(case,box)
                try:
                    q.compositor.ipc('output * mode 1920x1080')
                    q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=hash)))
                    q.wait(lambda:view(q).get('details',{}).get('hash')==hash)
                    q.wait(lambda:q.ipc('detailTab','pieces')=='true')
                    q.wait(lambda:view(q).get('details',{}).get('tab')=='pieces')
                    try:q.wait(lambda:any('Recheck' in text for text in detail_texts(q)))
                    except TimeoutError as error:raise TimeoutError(json.dumps(dict(view=view(q),texts=detail_texts(q)))) from error
                    texts=detail_texts(q)
                    case.check(not any('until checking completes' in text for text in texts),'dashboard completed Pieces explains the absent map and offers Recheck',json.dumps(texts))
                finally:q.close()
                tmux_pieces(case,box,binary,h,hash)
            h.stop_serve(process)
        h.lock_free()
    return suite_status(SUITE)


def review(binary):
    with Case(SUITE,'review') as case,fixtures.Sandbox() as box,daemon.Harness(case,binary,box,False) as h:
        process=h.start_serve(idle_exit_secs=0)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private review daemon startup failed')
        with daemon.ApiConn(h) as client:
            client.hello()
            source=box.root/'source';source.mkdir()
            payload=source/'Already paused.bin';payload.write_bytes(b'x'*65536)
            torrent=fixtures.torrent_fixture(source,[payload])
            client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),stopped=True))
            q=Session(case,box)
            try:
                q.compositor.ipc('output * mode 1920x1080')
                q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
                q.wait(lambda:'Paused' in rendered(q,torrent.info_hash))
                staged='b'*40
                magnet='magnet:?xt=urn:btih:'+staged+'&dn=Review%20magnet'
                q.shell('summon',json.dumps(dict(view='torrents',add=dict(source=dict(magnet=magnet)))))
                q.wait(lambda:view(q).get('add',{}).get('hash')==staged and view(q).get('add',{}).get('owned'))
                def labels():return json.loads(q.ipc('viewTexts'))
                q.wait(lambda:all(caption in labels() for caption in ['All  2','Paused  1','Reviewing  1']))
                case.check('Reviewing' in rendered(q,staged) and view(q)['rows']==2,'review sheet leaves two rows with matching All 2 / Paused 1 / Reviewing 1 captions',json.dumps(dict(labels=labels(),rows=json.loads(q.ipc('renderedRows')))))
                case.check(any('u upload' in text for text in labels()) and not any('u seed' in text for text in labels()),'footer advertises u upload; Details uses effective Upload off/on wording',json.dumps(labels()))
                q.compositor.ipc('[title="^fbittorrent$"] focus')
                q.keys(['-k','Escape'],lambda:not view(q).get('add',{}).get('visible'))
                q.wait(lambda:view(q).get('rows')==1 and all(caption in labels() for caption in ['All  1','Paused  1','Reviewing  0']))
                case.check([r['hash'] for r in client.call('torrents.list',{})['result']['torrents']]==[torrent.info_hash],'Discard removes only the staged magnet and immediately repairs filter counts',json.dumps(labels()))
                try:q.wait(lambda:len(json.loads(q.ipc('renderedRows')))==1)
                except TimeoutError:pass
                rows=json.loads(q.ipc('renderedRows'))
                case.check(len(rows)==1 and rows[0]['hash']==torrent.info_hash and payload.name in rows[0]['texts'] and 'Paused' in rows[0]['texts'] and 'Reviewing' not in rows[0]['texts'],'Discard restores the surviving rendered row identity, name and state without a sort',json.dumps(rows))
            finally:q.close()
        h.stop_serve(process);h.lock_free()
    return suite_status(SUITE)


def stop(binary):
    with Case(SUITE,'stop') as case,fixtures.Sandbox() as box,daemon.Harness(case,binary,box,False) as h:
        process=h.start_serve(idle_exit_secs=300)
        if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private stop daemon startup failed')
        with daemon.ApiConn(h) as client:
            client.hello()
            source=box.root/'source';source.mkdir()
            payload=source/'Active private.bin';payload.write_bytes(b'x'*65536)
            torrent=fixtures.torrent_fixture(source,[payload])
            client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(box.downloads)))
            if not h.wait_for(lambda:((row:=peek(client,'torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state'] in ['downloading','stalled','checking'],json.dumps(row)),'fixture actively downloads from an empty destination'):raise RuntimeError('fixture did not start')
            q=Session(case,box)
            try:
                q.wait(lambda:q.ipc('ready')=='true')
                for screen in ['torrents','search','shows','rss','log','settings']:
                    q.shell('summon',json.dumps(dict(view=screen)))
                    q.wait(lambda:view(q).get('window',{}).get('view')==screen)
                    q.compositor.ipc('[title="^fbittorrent$"] focus')
                    if screen=='search':q.keys(['-k','Down'],lambda:view(q).get('focus')=='results')
                    chord=['-M','ctrl','q','-m','ctrl'] if screen=='settings' else ['Q']
                    q.keys(chord,lambda:view(q).get('stopping'))
                    case.check(view(q).get('stopSelected')==0 and client.call('app.info')['ok'],'Q from '+screen+' opens Cancel-first stop confirmation while daemon stays up',json.dumps(view(q)))
                    q.keys(['-k','Return'],lambda:not view(q).get('stopping'))
                    case.check(client.call('app.info')['ok'],'Enter on default Cancel preserves transfers from '+screen,json.dumps(q.state()))
                q.keys(['-M','ctrl','q','-m','ctrl'],lambda:view(q).get('stopping'))
                q.keys(['-k','Right','-k','Return'],lambda:not q.state()['online'])
                case.check(True,'explicit Stop confirmation shuts down daemon from Settings',json.dumps(q.state()))
            finally:q.close()
        process.wait();h.lock_free()
    return suite_status(SUITE)


def popup(binary):
    from visual import Capture
    from visual_fixture import running
    with Case(SUITE,'popup') as case,running(binary,'empty',case) as (box,h,client,_):
        client.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+'c'*40),stage=True))
        q=Capture(case,box,'owner')
        try:
            q.wait(lambda:json.loads(q.ipc('state'))['online']);q.ipc('open')
            q.wait(lambda:json.loads(q.ipc('state')).get('idle','').startswith('Idle'))
            case.check(True,'running idle daemon shows its actual optional countdown',q.ipc('state'))
            q.wait(lambda:json.loads(q.ipc('groups'))==[dict(key='reviewing',count=1)])
            case.check(True,'staged magnet is grouped as Reviewing in the actual popup',q.ipc('groups'))
            q.ipc('clickPower');q.wait(lambda:not json.loads(q.ipc('state'))['online'])
            case.check(json.loads(q.ipc('state'))['idle']=='','power-off clears the cached countdown in the real open popup',q.ipc('state'))
        finally:q.close()
    return suite_status(SUITE)


def tmux_pieces(case,box,binary,h,hash):
    socket='fbt-owner-review-pieces'
    def tmux(*args,check=True):return subprocess.run(['tmux','-L',socket,*args],env=box.env,text=True,capture_output=True,check=check,timeout=4)
    def capture():return tmux('capture-pane','-p','-t',socket).stdout
    def wait(predicate):
        until=time.monotonic()+10
        while time.monotonic()<until:
            text=capture()
            if predicate(text):return text
            time.sleep(.05)
        raise TimeoutError(capture())
    try:
        tmux('new-session','-d','-s',socket,'-x','120','-y','30',str(binary),'--no-start','tui')
        wait(lambda text:'Renamed fixture' in text)
        tmux('send-keys','-t',socket,'Enter');wait(lambda text:'[Overview]' in text)
        for tab in ('Files','Peers','Trackers','Log','Pieces'):
            tmux('send-keys','-t',socket,'-l',']');wait(lambda text:'['+tab+']' in text)
        text=wait(lambda text:'Recheck' in text)
        case.path.with_suffix('.tui.txt').write_text(text)
        case.check('until checking completes' not in text,'TUI completed Pieces explains the absent map and offers Recheck',text)
        tmux('send-keys','-t',socket,'Escape')
        wait(lambda text:'[Pieces]' not in text)
        with daemon.ApiConn(h) as reader:
            reader.hello()
            topic='torrent:'+hash+'.pieces'
            if not h.wait_for(lambda:((topic not in (r:=peek(reader,'app.info')['result']['subscriptions']['by_topic'])),json.dumps(r)),'TUI releases the Pieces subscription'):
                raise RuntimeError('TUI did not release Pieces sampling')
        tmux('send-keys','-t',socket,'-l','q')
        until=time.monotonic()+5
        while tmux('has-session','-t',socket,check=False).returncode==0:
            if time.monotonic()>=until:raise TimeoutError('owned TUI did not detach')
            time.sleep(.05)
    finally:tmux('kill-server',check=False)


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--case',choices=['search','rows','pieces','popup','review','stop'],required=True)
    args=parser.parse_args()
    previous=ROOT/'target/test-logs'/SUITE
    if previous.exists():shutil.copytree(previous,ROOT/'target/test-logs/owner-review-runs'/str(time.time_ns()))
    raise SystemExit(run(args.binary.resolve(),args.case))
