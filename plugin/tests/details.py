#!/usr/bin/env python3
"""Native Details/Files/Log acceptance against real private daemon and IMDb HTTP."""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import shutil
import sys
from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status


def view(q):return q.state().get('view') or {}
def detail(q):return view(q).get('details') or {}
def press(case,q,args,predicate,expect):
    q.keys(args,predicate)
def ledger(sandbox,hash):
    path=Path(sandbox.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json'
    return next(r for r in json.loads(path.read_text())['records'] if r['hash']==hash)

def run(binary):
    import datetime
    archive=ROOT/'target/test-logs/fbt-p3-10-ii0/runs'/datetime.datetime.now().strftime('%H%M%S')
    for suite in ['qs_details','daemon']:
        previous=ROOT/'target/test-logs'/suite
        if previous.exists():shutil.copytree(previous,archive/suite)
    with daemon.imdb_owned('details_setup' ,binary,False) as (setup,sandbox,harness,http,directory,cache):
        with daemon.ApiConn(harness) as client:
            hello=client.hello()['result']
            if not {'subscribe','log','imdb','files'}.issubset(hello['caps']):raise RuntimeError('positive Details acceptance requires real subscribe/log/imdb/files services')
            spec=json.loads((fixtures.HERE/'fixtures/states/files-natural.json').read_text())
            torrent=fixtures.multi_file_fixture(spec['names'])
            added=client.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(sandbox.downloads),stopped=True))
            hash=torrent.info_hash
            second=fixtures.multi_file_fixture(['Other.txt'],name='Other')
            client.call('torrents.add',dict(source=dict(path=str(second.torrent)),save_path=str(sandbox.downloads),stopped=True))
            q=Session(setup,sandbox)
            try:
                q.compositor.ipc('output * mode 1920x1080')
                q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=hash)))
                q.wait(lambda:detail(q).get('hash')==hash)
                q.wait(lambda:q.compositor.ipc('[title="^fbittorrent$"] floating enable, border none'))
                q.compositor.ipc('seat seat0 cursor set 1 1')
                with Case('qs_details','overview_poster_renders') as case:
                    q.wait(lambda:detail(q).get('poster')=='loaded',timeout=12)
                    card=client.call('imdb.lookup',dict(id='tt1234001'))['result']['card']
                    case.check(Path(card['poster_path']).is_file() and detail(q)['poster']=='loaded','native asynchronous Image renders the real locally cached HTTP fixture poster',json.dumps(dict(card=card,details=detail(q))))
                    image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
                with Case('qs_details','details_tabs_cycle') as case:
                    press(case,q,[']'],lambda:detail(q).get('tab')=='files','] cycles from list into Files')
                    press(case,q,['['],lambda:detail(q).get('tab')=='overview','[ returns to Overview')
                    press(case,q,['-k','Return'],lambda:view(q).get('focus')=='details','Enter focuses the docked pane')
                    press(case,q,[']'],lambda:detail(q).get('tab')=='files','] cycles from focused pane into Files')
                with Case('qs_details','files_natural_order') as case:
                    q.wait(lambda:len(detail(q).get('files',[]))==10)
                    case.check(detail(q)['files']==spec['natural_indices'],'actual file delegates use natural sort_key order despite scrambled indices',json.dumps(detail(q)))
                    press(case,q,['/e10'],lambda:len(detail(q).get('files',[]))==1,'focused Files filter uses a case-insensitive substring')
                    press(case,q,['-k','Escape'],lambda:not q.ipc('fileEditing')=='true','Esc exits only the file input')
                    press(case,q,['/','-M','ctrl','a','-m','ctrl','-k','BackSpace','-k','Return'],lambda:len(detail(q).get('files',[]))==10,'clearing filter restores all files')
                    image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
                with Case('qs_details','files_toggle_persists') as case:
                    press(case,q,['-k','space'],lambda:client.call('torrents.files',dict(hash=hash))['result']['files'][3]['included'] is False,'Space skips E1 at its real index 3')
                    press(case,q,['/E10'],lambda:detail(q).get('files')==[0],'Files filter matches E10 case-insensitively')
                    press(case,q,['-k','Escape'],lambda:not q.ipc('fileEditing')=='true','Esc returns to Files actions')
                    press(case,q,['n'],lambda:not client.call('torrents.files',dict(hash=hash))['result']['files'][0]['included'],'n skips only the filtered subset')
                    press(case,q,['a'],lambda:client.call('torrents.files',dict(hash=hash))['result']['files'][0]['included'],'a includes only the filtered subset')
                    case.check(not view(q)['add']['visible'],'Files a never opens the dashboard add sheet',json.dumps(detail(q)))
                    press(case,q,['/','-M','ctrl','a','-m','ctrl','-k','BackSpace','-k','Return'],lambda:len(detail(q).get('files',[]))==10,'clear file filter before reopen')
                    first=ledger(sandbox,hash);case.check(3 not in first['only_files'],'skip is durably recorded as only_files',json.dumps(first))
                    press(case,q,['-k','Escape'],lambda:view(q).get('focus')=='table','Esc returns focus to the torrent list')
                    q.shell('hide');q.wait(lambda:q.state()['view'] is None)
                    q.shell('summon',json.dumps(dict(view='torrents',hash=hash)))
                    q.wait(lambda:detail(q).get('hash')==hash)
                    if detail(q).get('tab')!='files':press(case,q,[']'],lambda:detail(q).get('tab')=='files','new pane opens Files after reopen')
                    q.wait(lambda:len(detail(q).get('files',[]))==10)
                    case.check(client.call('torrents.files',dict(hash=hash))['result']['files'][3]['included'] is False,'pane reopen preserves the confirmed include/skip state',json.dumps(detail(q)))
                    # Restart the exact owned daemon; same private ledger and compositor.
                    client.close()
                    process=harness.imdb_process
                    harness.stop_serve(process)
                    harness.notify.close()
                    process=harness.start_serve();harness.imdb_process=process
                    if not(harness.receive_ready() and harness.wait_engine()):raise RuntimeError('owned daemon restart did not become ready')
                    client=daemon.ApiConn(harness);client.hello();harness.resources.callback(client.close)
                    q.wait(lambda:q.ipc('ready')=='true',timeout=12)
                    after=ledger(sandbox,hash)
                    files=client.call('torrents.files',dict(hash=hash))['result']['files']
                    case.check(after['only_files']==first['only_files'] and not files[3]['included'],'real daemon restart restores ledger.only_files',json.dumps(dict(before=first,after=after,files=files)))
                with Case('qs_details','detail_topic_released') as case:
                    q.wait(lambda:detail(q).get('hash')==hash)
                    if detail(q).get('tab')!='files':press(case,q,[']'],lambda:detail(q).get('tab')=='files','select Files before sampling check')
                    q.wait(lambda:'torrent:'+hash+'.files' in client.call('app.info')['result']['subscriptions']['by_topic'])
                    before=client.call('app.info')['result']['subscriptions']['by_topic']
                    q.shell('hide');q.wait(lambda:q.state()['view'] is None)
                    q.wait(lambda:not any(k.startswith('torrent:') for k in client.call('app.info')['result']['subscriptions']['by_topic']))
                    after=client.call('app.info')['result']['subscriptions']['by_topic'];case.check(not any(k.startswith('torrent:') for k in after),'hide unsubscribes every torrent detail topic',json.dumps(dict(before=before,after=after)))
                with Case('qs_details','pane_width_persists') as case:
                    q.shell('summon',json.dumps(dict(view='torrents',hash=hash)));q.wait(lambda:detail(q).get('hash')==hash)
                    q.wait(lambda:q.compositor.ipc('[title="^fbittorrent$"] floating enable, border none, resize set 1800 px 800 px'))
                    q.wait(lambda:view(q).get('window',{}).get('width')==1800)
                    actual=float(q.native_hook('setPaneWidth','420'));case.check(actual==420,'guarded width hook uses the same drag clamp',json.dumps(detail(q)))
                    prefs=Path(sandbox.env['XDG_STATE_HOME'])/'fbittorrent/dashboard.json'
                    q.wait(lambda:prefs.exists() and json.loads(prefs.read_text()).get('details_width')==420)
                    q.shell('hide');q.wait(lambda:q.state()['view'] is None);q.shell('summon',json.dumps(dict(view='torrents',hash=hash)))
                    q.wait(lambda:q.compositor.ipc('[title="^fbittorrent$"] floating enable, border none, resize set 1800 px 800 px'))
                    q.wait(lambda:detail(q).get('width')==420)
                    q.close();q.config.rename(sandbox.root/'first-shell');shutil.copy2(q.log,q.log.with_suffix('.before-shell-restart.log'))
                    q=Session(setup,sandbox);q.compositor.ipc('output * mode 1920x1080')
                    q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=hash)))
                    q.wait(lambda:q.compositor.ipc('[title="^fbittorrent$"] floating enable, border none, resize set 1800 px 800 px'))
                    q.wait(lambda:detail(q).get('width')==420)
                with Case('qs_details','details_sheet_below_1400') as case:
                    q.wait(lambda:q.compositor.ipc('[title="^fbittorrent$"] resize set 1399 px 800 px'));q.wait(lambda:detail(q).get('mode')=='sheet')
                    press(case,q,['-k','Return'],lambda:view(q).get('panes',{}).get('detailsSheet'),'Enter at1399 opens the overlay sheet')
                    case.check(detail(q)['width']<=720 and not view(q)['panes']['detailsDocked'],'sheet uses the720px bound and replaces the dock',json.dumps(detail(q)))
                    press(case,q,['-k','Escape'],lambda:not view(q).get('panes',{}).get('detailsSheet'),'Esc at1399 closes the sheet and releases its detail selection')
                with Case('qs_details','log_tab_filters_hash') as case:
                    q.wait(lambda:q.compositor.ipc('[title="^fbittorrent$"] resize set 1800 px 800 px'));q.wait(lambda:detail(q).get('mode')=='docked' and detail(q).get('hash')==hash)
                    while detail(q).get('tab')!='overview':press(case,q,[']'],lambda:True,'] returns to Overview before wrap check')
                    press(case,q,['['],lambda:detail(q).get('tab')=='log','[ wraps Overview into Log')
                    q.wait(lambda:client.call('app.info')['result']['subscriptions']['by_topic'].get('log')==1)
                    q.wait(lambda:len(json.loads(q.ipc('logEntries')))>0)
                    entries=json.loads(q.ipc('logEntries'))
                    case.check(all(e.get('hash')==hash for e in entries),'Log renders only the selected torrent hash',json.dumps(entries))
                    image=case.path.with_suffix('.png');q.ipc('grab',image);q.wait(lambda:image.exists() and image.stat().st_size>0)
                    q.shell('hide');q.wait(lambda:client.call('app.info')['result']['subscriptions']['by_topic'].get('log',0)==0)
            finally:q.close()
    return suite_status('qs_details') or suite_status('daemon')

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,required=True)
    args=parser.parse_args();sys.exit(run(args.binary.resolve()))
