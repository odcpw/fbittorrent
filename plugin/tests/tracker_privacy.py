#!/usr/bin/env python3
"""Synthetic tracker credentials through the real API, CLI, terminal and Qt UIs."""
import argparse
import base64
import json
from pathlib import Path
import sys
import urllib.parse
from dashboard import ROOT,Session
import daemon as d
import fixtures
import native
import private_net
import fbtlog
from fbtlog import Case,suite_status
from tui_smoke import PaneUi
from visual import Capture

TOKENS=('0123456789abcdef0123456789abcdef','QUJDREVGR0hJSktMTU5PUFFS','shortkey-a','shortkey-b')
SECRETS=tuple(set(TOKENS+tuple(urllib.parse.quote(v,safe='') for v in TOKENS)))
def clean(value):return all(secret not in str(value) for secret in SECRETS)

class Guarded(d.ApiConn):
    def diagnostic(self,value):
        text=super().diagnostic(value)
        for secret in SECRETS:text=text.replace(secret,'[synthetic credential]')
        return text
    def receive(self):
        value=super().receive()
        if value is not None:
            self.case.check(clean(json.dumps(value)),'raw API replies, errors and pushed frames contain no synthetic credential',
                            'checked unmodified socket frame before diagnostic masking')
        return value

def run(binary):
    private_net.enter()
    with Case('tracker_privacy','all_surfaces') as case,fixtures.Sandbox() as box:
        box.env['RUST_LOG']='librqbit_tracker_comms=trace';box.keys+=['RUST_LOG']
        config=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('notify = false','notify = true'))
        fixtures.path_shim('notify-send','exit 0',box.root)
        with fixtures.HttpFixture(box.root,{}) as http:
            a=http.base+'/'+TOKENS[0]+'/announce?passkey='+TOKENS[2]
            b=http.base+'/'+TOKENS[1]+'/announce?token='+TOKENS[3]
            for url,token,key in [(a,TOKENS[0],TOKENS[2]),(b,TOKENS[1],TOKENS[3])]:
                path=urllib.parse.urlsplit(url).path
                reason='denied '+token+' '+key+' at '+url
                http.routes[path]=dict(base64=base64.b64encode(fixtures.bencode({'failure reason':reason})).decode())
            fixture=d.tracker_live_fixture(box,a,name='private-tracker-fixture.bin')
            native.prepare_imdb_fixture(box.env,['private-tracker-fixture.bin'])
            with d.Harness(case,binary,box,False) as h:
                process=h.start_serve(idle_exit_secs=0)
                if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('private daemon failed to start')
                with Guarded(h,redact_urls=True) as client:
                    client.hello();client.call('torrents.add',dict(source=dict(path=str(fixture.torrent)),origin='handler'))
                    hash_=fixture.info_hash
                    def rows():return d.tracker_rows(client,hash_)
                    h.wait_for(lambda:((values:=rows())[0]['status']=='error',client.diagnostic(values)),
                               'real credential-bearing announce returns an echoed failure')
                    first=rows()[0]['url'];client.call('torrents.add_tracker',dict(hash=hash_,url=b))
                    current=rows();other=next(row['url'] for row in current if row['url']!=first)
                    originals={first:a,other:b}
                    case.check(len(current)==2 and len({row['url'] for row in current})==2
                               and len({row['url'].split('#')[0] for row in current})==1,
                               'identical redacted endpoints retain two distinct opaque removal identities',client.diagnostic(current))
                    client.call('torrents.add_tracker',dict(hash=hash_,url=first),error='invalid_args')
                    client.call('torrents.add_tracker',dict(hash=hash_,url='ftp://tracker.invalid/'+TOKENS[0]),error='invalid_args')
                    topic='torrent:'+hash_+'.trackers'
                    client.call('subscribe',dict(topics=[topic,'log']))
                    def snapshot():
                        client.call('app.info')
                        return any(v.get('ev')=='snapshot.end' and v.get('topic')==topic for v in client.pushed),'waiting for actual tracker snapshot'
                    h.wait_for(snapshot,'tracker snapshot is delivered without credentials')
                    for args in [('api','torrents.trackers','--json-args',json.dumps(dict(hash=hash_))),('show',hash_),('log',)]:
                        value=h.cli(*args);case.check(clean(json.dumps(value)),'CLI API/show/log output is credential-free',json.dumps(value))

                    # Actual popup with its own compositor and Service/API connection.
                    popup=Capture(case,box,'owner')
                    try:
                        popup.wait(lambda:json.loads(popup.ipc('state'))['online']);popup.ipc('open')
                        popup.wait(lambda:json.loads(popup.ipc('state'))['opened'])
                        case.check(clean(popup.ipc('state')),'real popup/bar state and feedback contain no tracker credentials',popup.ipc('state'))
                    finally:popup.close()
                    popup.log.rename(case.path.with_suffix('.popup.qs.log'))
                    popup.config.rename(box.root/'closed-popup')

                    ui=PaneUi(case,str(binary),box,h,session='fbt-tracker-privacy')
                    try:
                        ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','90','-y','24')
                        if not ui.observe('wait for the selectable torrent','the pushed fixture row is present before opening details',lambda pane:'private-tracker-fixture.bin' in pane):raise RuntimeError('TUI fixture row did not arrive')
                        if not ui.contains('Enter','[Overview]',literal=False):raise RuntimeError('TUI details did not open')
                        if not ui.details_tab('Trackers'):raise RuntimeError('TUI Trackers tab did not open')
                        ui.observe('inspect real tracker rows','TUI shows redacted URLs and remote errors',
                                   lambda pane:'REDACTED/announce' in pane and 'denied' in pane and clean(pane))
                        before=sorted(row['url'] for row in rows());selected=before[0]
                        if not ui.contains('x','[Cancel]'):raise RuntimeError('tracker-specific confirmation did not open')
                        case.check(clean(ui.capture(ui.target)) and 'fbt-tracker=' not in ui.capture(ui.target),
                                   'TUI removal confirmation hides credentials and internal identity',ui.capture(ui.target))
                        ui.press('Enter','default Cancel preserves both trackers',lambda pane:'[Cancel]' not in pane,literal=False)
                        case.check(sorted(row['url'] for row in rows())==before,'TUI Cancel preserves every exact tracker')
                        if not ui.contains('x','[Cancel]'):
                            raise RuntimeError('tracker-specific confirmation did not reopen')
                        ui.press('y','explicitly remove only the selected opaque tracker',lambda pane:'[Cancel]' not in pane)
                        h.wait_for(lambda:([row['url'] for row in rows()]==[v for v in before if v!=selected],
                                           client.diagnostic(rows())),'TUI removal uses the exact internal identity')
                        record=json.loads((Path(box.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json').read_text())['records'][0]
                        case.check(originals[selected] in record.get('removed_trackers',[]) and originals[next(v for v in before if v!=selected)] not in record.get('removed_trackers',[]),
                                   'durable tombstone contains only the selected original credential URL')
                        ui.press('Escape','close tracker details',literal=False);ui.contains('L','Log · L/Esc Back')
                        case.check(clean(ui.capture(ui.target)),'TUI message history exposes no credential',ui.capture(ui.target))
                        ui.press('Escape','close message history',literal=False)
                        ui.press('q','detach after privacy checks',lambda pane:ui.done.exists() and ui.done.read_text()=='0')
                    finally:ui.close()
                    client.call('torrents.add_tracker',dict(hash=hash_,url=originals[selected]))

                    q=Session(case,box)
                    try:
                        q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=hash_)))
                        q.wait(lambda:(q.state().get('view') or {}).get('cursor')==hash_)
                        q.keys(['-k','Return'],lambda:(q.state().get('view') or {}).get('focus')=='details')
                        case.check(q.ipc('clickDetailTab','trackers')=='true','open actual dashboard tracker controls')
                        q.wait(lambda:len(json.loads(q.ipc('trackerRows')))==2)
                        case.check(clean(q.ipc('trackerRows')) and 'REDACTED/announce' in q.ipc('trackerRows'),
                                   'dashboard rows and failure text remain redacted',q.ipc('trackerRows'))
                        before=sorted(row['url'] for row in rows());selected=before[0]
                        def editor():return json.loads(q.ipc('trackerEditorState'))
                        q.ipc('trackerButton','remove');q.wait(lambda:editor().get('mode')=='remove')
                        case.check(editor()['cancelFocused'] and clean(editor()['confirmation']) and 'fbt-tracker=' not in editor()['confirmation'],
                                   'dashboard Remove confirms only a redacted endpoint and defaults to Cancel',json.dumps(editor()))
                        q.keys(['-k','Return'],lambda:editor().get('mode')=='')
                        case.check(sorted(row['url'] for row in rows())==before,'dashboard Cancel preserves both opaque identities')
                        q.ipc('trackerButton','remove');q.wait(lambda:editor().get('mode')=='remove')
                        q.keys(['-k','Right','-k','Return'],lambda:editor().get('mode')=='')
                        h.wait_for(lambda:([row['url'] for row in rows()]==[v for v in before if v!=selected],client.diagnostic(rows())),
                                   'dashboard explicit removal targets the correct private tracker')
                        q.ipc('clickDetailTab','log');q.wait(lambda:clean(q.ipc('logEntries')))
                        case.check(clean(q.ipc('logEntries')) and clean(q.ipc('logRows')),'dashboard log/history carries only redacted diagnostics',q.ipc('logRows'))
                        q.shell('hide')
                    finally:q.close()
                    case.check(any(urllib.parse.urlsplit(a).path in request for request in http.requests),
                               'real tracker receives its original credential path internally')
                    entries=client.call('app.log',dict(limit=1000))['result']['entries']
                    case.check(clean(json.dumps(entries)) and clean(h.stderr()),'API logs and original daemon stderr/journal bytes contain no synthetic credentials',
                               'checked raw history entries and original captured stderr')
                    notify=box.root/'shims/notify-send.log'
                    h.wait_for(lambda:(notify.exists(),'waiting for owned notify-send shim'),'real handler Add notification is delivered privately')
                    case.check(clean(notify.read_text()),'actual notification argv contains no tracker credential',notify.read_text())
                h.stop_serve(process);h.lock_free()
                logs=[case.path.with_suffix('.popup.qs.log'),case.path.with_suffix('.qs.log')]
                case.check(all(clean(path.read_text()) for path in logs),'popup/dashboard QML logs contain no credentials','checked raw private QML logs')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    run(args.binary.resolve());sys.exit(suite_status('tracker_privacy'))
