#!/usr/bin/env python3
"""Real native table actions, width boundaries, persistence and render budget."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from dashboard import Session,ROOT
from journey_support import JourneySession
import fixtures
from open_support import allow_open,add_text_torrent,calls,launcher_calls,blocked_payload_error
from fbtlog import Case,suite_status
from visual_fixture import running


def view(q):return q.state().get('view') or {}


def wait_view(q,predicate):
    try:return q.wait(lambda:s if (s:=view(q)) and predicate(s) else None)
    except TimeoutError as error:raise TimeoutError(json.dumps(q.state())) from error


def click_row(q,index,button='button1',times=1):
    def windows(node):
        result=[node] if node.get('name')=='fbittorrent' and node.get('app_id')=='org.quickshell' else []
        for child in node.get('nodes',[])+node.get('floating_nodes',[]):result+=windows(child)
        return result
    nodes=windows(q.compositor.ipc(kind=4))
    if len(nodes)!=1:raise RuntimeError('expected one private native dashboard')
    point=json.loads(q.ipc('rowPoint',index));rect=nodes[0]['rect'];offset=nodes[0]['window_rect']
    x=round(rect['x']+offset['x']+point['x']);y=round(rect['y']+offset['y']+point['y'])
    import private_pointer
    pointer=private_pointer.Pointer(q,x,y,273 if button=='button3' else 272,times)
    try:
        def entered():
            pointer.motion()
            return view(q).get('hover')==view(q).get('cursor') and bool(view(q).get('hover'))
        q.wait(entered)
        pointer.click()
        if times==2:q.wait(lambda:view(q).get('details',{}).get('visible'))
    finally:pointer.close()
    return dict(x=x,y=y,button=button,clicks=times)


def functional(binary):
    with Case('qs_dashboard_table','table_actions') as case,running(binary,'paused',case) as (sandbox,harness,client,row):
        fixtures.path_shim('xdg-open','exit 0',sandbox.root)
        fixtures.path_shim('mpv','exit 0',sandbox.root)
        fixtures.path_shim('wl-copy','cat > "$XDG_RUNTIME_DIR/copied-text"',sandbox.root)
        cache=Path(sandbox.env['XDG_CACHE_HOME'])/'fbittorrent/imdb'
        for directory in ('by-id','by-name-v2'):(cache/directory).mkdir(parents=True,exist_ok=True)
        ident='tt1234001'
        (cache/'by-id'/f'{ident}.json').write_text(json.dumps(dict(id=ident,name='Owned table show',year=2026,kind='tvSeries',stars='',poster_url='',blurb='',wiki_url='')))
        key=re.sub(r'[^a-z0-9]','_',row['name'].lower())[:120]
        (cache/'by-name-v2'/f'{key}.json').write_text(json.dumps(dict(id=ident)))
        q=JourneySession(case,sandbox)
        allow_open(sandbox,binary)
        try:
            q.compositor.ipc('output * mode 1920x1080')
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
            wait_view(q,lambda s:s.get('rows')==1)
            q.compositor.ipc('[title="^fbittorrent$"] floating enable, border none')
            q.compositor.ipc('seat seat0 cursor set 1 1')
            q.keys(['j'],lambda:view(q).get('cursor')==row['hash'])
            q.keys(['-k','Return'],lambda:view(q).get('details',{}).get('visible'))
            case.check(view(q)['details']['visible'],'Enter opens the details slot',json.dumps(view(q)))
            q.keys(['-k','Escape'],lambda:not view(q).get('details',{}).get('visible'))
            for width in (900,1098,1099,1100,1101,1280,1398,1399,1400,1401,1500,1598,1599,1600,1601):
                q.compositor.ipc('[title="^fbittorrent$"] resize set '+str(width)+' px 700 px')
                state=wait_view(q,lambda s:q.state()['view']['window']['width']==width and abs(sum(c['width'] for c in s['columns'])-s['listWidth'])<.1)
                columns={c['name']:c for c in state['columns']}
                case.check(columns['name']['width']>=210 and columns['up']['hidden']==(width<1600) and columns['ratio']['hidden']==(width<1600) and columns['added']['hidden']==(width<1600) and state['panes']['sidebar']==(width>=1100) and state['panes']['detailsDocked']==(width>=1400),
                           'width tier '+str(width)+' keeps NAME and documented columns',json.dumps(dict(width=width,list=state['listWidth'],columns=columns)))
                if width in (900,1280,1500,1600):
                    q.keys(['-k','Return'],lambda:view(q).get('details',{}).get('visible'))
                    image=case.path.with_name('dashboard-'+str(width)+'.png')
                    q.ipc('grab',image);q.wait(lambda:image.is_file() and image.stat().st_size>0)
                    q.keys(['-k','Escape'],lambda:not view(q).get('details',{}).get('visible'))
            image=case.path.with_suffix('.before-click.png');q.ipc('grab',image);q.wait(lambda:image.is_file() and image.stat().st_size>0)
            click=click_row(q,0,times=2)
            wait_view(q,lambda s:s.get('details'))
            case.check(view(q)['cursor']==row['hash'],'native double-click is Enter on the same hash',json.dumps(click))
            q.keys(['-k','Escape'],lambda:not view(q).get('details',{}).get('visible'))
            click_row(q,0,button='button3');time.sleep(.1)
            case.check(not view(q)['details']['visible'] and view(q)['dialog'] is None,'right-click in the table has no action',json.dumps(view(q)))
            q.compositor.ipc('seat seat0 cursor set 1 1')
            q.keys(['d'],lambda:view(q).get('dialog') is not None)
            before=client.call('torrents.get',dict(hash=row['hash']))
            case.check(view(q)['dialog']['selected']==0,'remove opens with Cancel selected',json.dumps(view(q)['dialog']))
            q.keys(['-k','Return'],lambda:view(q).get('dialog') is None)
            after=client.call('torrents.get',dict(hash=row['hash']))
            case.check(before['ok'] and after['ok'],'Enter immediately after remove cancels; outside pointer never hovers a button',json.dumps(after))
            q.keys(['D'],lambda:view(q).get('dialog') is not None)
            case.check(view(q)['dialog']==dict(kind='delete',selected=0),'delete-files also defaults to Cancel',json.dumps(view(q)['dialog']))
            q.keys(['-k','Escape'],lambda:view(q).get('dialog') is None)
            q.keys(['s'],lambda:view(q).get('sort',{}).get('key')=='name')
            q.keys(['S'],lambda:view(q).get('sort',{}).get('reverse') is True)
            prefs=Path(sandbox.env['XDG_STATE_HOME'])/'fbittorrent/dashboard.json'
            q.wait(lambda:prefs.exists() and json.loads(prefs.read_text()).get('sort')==dict(key='name',reverse=True))
            q.keys(['/fixture'],lambda:view(q).get('query')=='fixture')
            q.keys(['-k','Escape'],lambda:view(q).get('query')=='')
            q.keys(['p'],lambda:json.loads(q.ipc('confirmed',row['hash']))['intent']=='run')
            q.keys(['-k','space'],lambda:json.loads(q.ipc('confirmed',row['hash']))['intent']=='pause')
            case.check(client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']['intent']=='pause','p resumes and Space pauses through confirmed daemon state',q.ipc('confirmed',row['hash']))
            q.keys(['u'],lambda:view(q).get('focus')=='details')
            case.check(not client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']['upload_enabled'],
                       'table u opens Details without enabling upload',q.ipc('detailTexts'))
            q.keys(['u'],lambda:client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']['upload_enabled'])
            q.keys(['u'],lambda:not client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']['upload_enabled'])
            q.keys(['-k','Escape'],lambda:not view(q).get('details',{}).get('visible'))
            shutil.copyfile(sandbox.root/'pristine/UI fixture.bin',Path(row['content_path']))
            before=launcher_calls(sandbox)
            q.keys(['o'],lambda:blocked_payload_error((view(q).get('toast') or {}).get('text','')))
            case.check(launcher_calls(sandbox)==before,'o refuses the existing .bin payload without invoking an opener or player',view(q).get('toast'))
            before=len(calls(sandbox,'xdg-open'))
            q.keys(['O'],lambda:len(calls(sandbox,'xdg-open'))>before)
            folder=calls(sandbox,'xdg-open')[-1]
            case.check(folder==[str(Path(row['content_path']).parent)],'O opens the containing download folder',folder)
            document,payload=add_text_torrent(sandbox,client)
            q.wait(lambda:(json.loads(q.ipc('confirmed',document.info_hash)) or {}).get('state')=='done')
            q.shell('summon',json.dumps(dict(view='torrents',hash=document.info_hash)))
            q.wait(lambda:view(q).get('cursor')==document.info_hash)
            before=len(calls(sandbox,'xdg-open'));players=launcher_calls(sandbox)
            q.keys(['o'],lambda:len(calls(sandbox,'xdg-open'))>before)
            case.check(calls(sandbox,'xdg-open')[-1]==[str(payload)]
                       and all(launcher_calls(sandbox)[name]==players[name] for name in ('mpv','ffplay','vlc')),
                       'o opens a real safe .txt document literally without invoking a media player',calls(sandbox,'xdg-open'))
            client.call('torrents.remove',dict(hashes=[document.info_hash],delete_files=False))
            q.wait(lambda:not json.loads(q.ipc('confirmed',document.info_hash)))
            q.shell('summon',json.dumps(dict(view='torrents',hash=row['hash'])))
            q.wait(lambda:view(q).get('cursor')==row['hash'])
            copied_text=Path(sandbox.env['XDG_RUNTIME_DIR'])/'copied-text'
            q.keys(['y'],lambda:copied_text.exists() and copied_text.read_text()=='magnet:?xt=urn:btih:'+row['hash'])
            copied=[json.loads(line) for line in (sandbox.root/'shims/wl-copy.log').read_text().splitlines()]
            case.check(all(call==['--'] for call in copied) and copied_text.read_text()=='magnet:?xt=urn:btih:'+row['hash'],'y copies the exact magnet through stdin without exposing it in argv',json.dumps(copied))
            q.keys(['-k','Escape'],lambda:bool(view(q).get('window',{}).get('visible')))
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.is_file() and image.stat().st_size>0)
            q.keys(['q'],lambda:q.state()['view'] is None)
            case.check(q.state()['view'] is None,'top Esc stays open; q closes','native window destroyed')
        finally:q.close()
        shutil.copy2(q.log,q.log.with_suffix('.before-restart.log'))
        shutil.copy2(q.compositor.log,q.compositor.log.with_suffix('.before-restart.log'))
        q.config.rename(sandbox.root/'first-shell')
        q=JourneySession(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
            state=wait_view(q,lambda s:s.get('sort')==dict(key='name',reverse=True))
            q.keys(['j'],lambda:view(q).get('cursor')==row['hash'])
            q.keys(['i'],lambda:'imdb.com' in (sandbox.root/'shims/xdg-open.log').read_text())
            q.keys(['w'],lambda:'wikipedia.org' in (sandbox.root/'shims/xdg-open.log').read_text())
            q.keys(['f'],lambda:any(show['imdb']==ident for show in client.call('shows.list')['result']['shows']))
            q.keys(['/quit'],lambda:view(q).get('query')=='quit')
            q.keys(['-M','ctrl','q','-m','ctrl'],lambda:not q.state()['online'])
            case.check(not q.state()['pending'],'Ctrl+Q shuts down the real daemon; shutdown bye confirms the correlated action',json.dumps(q.state()))
        finally:q.close()


def fixture_cli(binary):
    import select,socket
    with Case('qs_dashboard_fixture','table_200_cli') as case,fixtures.Sandbox() as sandbox:
        error=case.path.with_suffix('.child.log').open('w')
        child=subprocess.Popen([sys.executable,str(ROOT/'agent_ergonomics_audit/audit/regression_tests/daemon.py'),str(binary),'--serve-fixture','table-200'],env=sandbox.env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=error,text=True)
        try:
            if not select.select([child.stdout],[],[],30)[0]:raise TimeoutError('fixture CLI did not publish its ready header')
            line=child.stdout.readline()
            if not line.startswith('FBTFIXTURE '):raise RuntimeError(line)
            ready=json.loads(line.split(' ',1)[1]);path=Path(ready['runtime'])/'fbittorrent/api.sock'
            case.check(ready['state']=='table-200' and path.is_socket(),'actual --serve-fixture table-200 CLI exposes its private real daemon',json.dumps(ready))
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as channel:
                channel.settimeout(3);channel.connect(str(path))
                with channel.makefile('rb') as stream:
                    def call(frame):
                        channel.sendall((json.dumps(frame)+'\n').encode());reply=stream.readline(1048577)
                        if len(reply)>1048576:raise RuntimeError('fixture reply exceeds frame cap')
                        return json.loads(reply)
                    hello=call(dict(id=1,op='hello',args=dict(proto=1,client=dict(name='fixture-control',version='test',kind='agent'),interactive=False)))
                    reply=call(dict(id=2,op='torrents.list',args={}))
                    case.check(hello['ok'] and reply['ok'] and len(reply['result']['torrents'])==200,'CLI fixture publishes 200 real API records',str(len(reply['result']['torrents'])))
            child.stdin.close();child.wait(timeout=25)
            case.check(child.returncode==0 and not path.exists(),'EOF tears down both fixture daemons and removes their socket','exit='+str(child.returncode))
        finally:
            if child.poll() is None:
                child.stdin.close()
                try:child.wait(timeout=25)
                except subprocess.TimeoutExpired:child.kill();child.wait();raise
            error.close()


def profile(binary):
    with Case('qs_dashboard_perf','table_200_rows') as case,running(binary,'table-200',case) as (sandbox,harness,client,row):
        os.environ['FBT_TEST_RENDER_TIMING']='1'
        q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"torrents"}')
            wait_view(q,lambda s:s.get('rows')==200)
            case.check(json.loads(q.native_hook('state'))['popupDelegates']==0,'closed popup instantiates no hidden torrent rows','zero delegates while dashboard renders 200 real rows')
            q.keys(['sss'],lambda:view(q).get('sort',{}).get('key')=='rate')
            time.sleep(1) # Exclude window creation; measure the specified steady ten-second interval.
            offset=q.log.stat().st_size
            q.native_hook('scroll',0) # Verify the guarded production IPC path once.
            if q.ipc('startScrolling')!='true':raise RuntimeError('private scroll driver refused')
            began=time.monotonic();next_sample=began;samples=[]
            while time.monotonic()-began<10:
                now=time.monotonic()
                if now>=next_sample:
                    current=client.call('torrents.get',dict(hash=row['hash']))['result']['torrent']
                    samples.append(dict(at=round(now-began,3),state=current['state'],bytes=current.get('completed_bytes'),rate=current.get('dl_bps')));next_sample+=1
                time.sleep(.005)
            q.wait(lambda:json.loads(q.ipc('scrollStats'))['count']==200,timeout=1)
            scrolling=json.loads(q.ipc('scrollStats'));scrolls=scrolling['count']
            image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.is_file() and image.stat().st_size>0)
            with q.log.open('rb') as trace:
                trace.seek(offset);text=trace.read().decode(errors='replace')
            timings=[float(value) for value in re.findall(r"(?:time in renderer:\s*|Frame rendered with 'software' renderloop in )([\d.]+)\s*ms",text)]
            artifact=case.path.with_suffix('.frames.json');artifact.write_text(json.dumps(dict(frames=timings,samples=samples,scrolls=scrolls,scrolling=scrolling),indent=2))
            case.check(scrolls==200 and 9900<=scrolling['elapsed_ms']<=10200 and len(samples)>=10,'continuous 20 Hz wheel hook plus real 1 Hz transfer samples for ten seconds',json.dumps(dict(scrolling=scrolling,samples=samples)))
            case.check(bool(timings) and max(timings,default=float('inf'))<=32,'every measured renderer frame is <= 32 ms; missing timing samples fail',json.dumps(dict(frames=len(timings),slowest=max(timings,default=None),artifact=str(artifact))))
            case.check(len({s['bytes'] for s in samples})>1,'private throttled seed/leech makes real progress during the budget interval',json.dumps(samples))
        except BaseException as error:
            case.check(False,'profile completes without an exception',repr(error));q.diagnose();raise
        finally:q.close();os.environ.pop('FBT_TEST_RENDER_TIMING',None)


if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--profile',action='store_true');parser.add_argument('--fixture-cli',action='store_true')
    args=parser.parse_args();binary=args.binary.resolve()
    # Case deliberately resets its suite; retain the prior attempt before that happens.
    previous=ROOT/'target/test-logs'/('qs_dashboard_perf' if args.profile else 'qs_dashboard_table')
    if previous.exists():
        from datetime import datetime,timezone
        archive=ROOT/'target/test-logs/fbt-p3-09-zlr/runs'/datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')/previous.name
        shutil.copytree(previous,archive)
    if args.fixture_cli:fixture_cli(binary)
    elif args.profile:profile(binary)
    else:functional(binary)
    sys.exit(suite_status('qs_dashboard_fixture' if args.fixture_cli else 'qs_dashboard_perf' if args.profile else 'qs_dashboard_table'))
