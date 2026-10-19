#!/usr/bin/env python3
"""Edit private tracker lists from real TUI/dashboard controls and restart twice."""
import argparse
import base64
import json
from pathlib import Path
import sys
import time
from dashboard import ROOT,Session
import daemon as d
import fixtures
import native
import private_net
from fbtlog import Case,suite_status


def run(binary):
    private_net.enter()
    with Case('daemon','tracker_edits_ui') as case,fixtures.Sandbox() as box,d.Harness(case,binary,box,False) as h:
        response=dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=b''))).decode())
        with fixtures.HttpFixture(box.root,dict.fromkeys(['/a','/b','/c'],response)) as http:
            a,b,c=[http.base+'/'+name+'?passkey=edit-private-'+name for name in 'abc']
            fixture=d.tracker_live_fixture(box,a,name='tracker-edits.bin')
            hash_=fixture.info_hash
            native.prepare_imdb_fixture(box.env,['tracker-edits.bin'])
            def count(name):return sum(path.startswith('/'+name+'?') for path in http.requests)
            def start():
                p=h.start_serve(idle_exit_secs=0)
                if not(h.receive_ready() and h.wait_engine()):raise RuntimeError('owned tracker daemon did not start')
                return p
            def rows(client):return d.tracker_rows(client,hash_)
            def urls(client,expected):
                values=rows(client);return sorted(v['url'].split('#')[0] for v in values)==sorted(v.split('?')[0] for v in expected),client.diagnostic(values)
            process=start()
            with d.ApiConn(h,redact_urls=True) as client:
                client.hello();client.call('torrents.add',dict(source=dict(path=str(fixture.torrent))))
                h.wait_for(lambda:(count('a')>=2,str(count('a'))),'original private tracker receives real announces')
                ui=native.AttachedTui(case,str(binary),box,h,session='fbt-fbt-p7-04-aq7')
                try:
                    target,done=ui.start();ui.assert_pane(target,'tracker-edits.bin')
                    def type_private(value):
                        if 'input hidden' not in ui.capture(target):raise RuntimeError('refuse to type private input without a masked tracker prompt')
                        result=ui.tmux('send-keys','-t',target,'-l',value)
                        case.check(result.returncode==0,'literal private input reaches the masked TUI field','tmux exit='+str(result.returncode))
                    ui.keys(target,'Enter',literal=False);ui.assert_pane(target,'[Overview]')
                    ui.keys(target,']]]');ui.assert_pane(target,'[Trackers]')
                    if not ui.assert_pane(target,'(+) Add'):raise RuntimeError('tracker actions did not become ready')
                    ui.keys(target,'+');ui.assert_pane(target,'input hidden');type_private(b)
                    pane=ui.capture(target);case.check('edit-private-' not in pane and 'passkey=' not in pane,'TUI masks the complete typed tracker URL',pane)
                    ui.keys(target,'Escape',literal=False);case.check(urls(client,[a])[0],'canceling TUI Add leaves the list unchanged')
                    ui.keys(target,'+');type_private(b);ui.keys(target,'Enter',literal=False)
                    h.wait_for(lambda:urls(client,[a,b]),'TUI Add saves the exact private backup URL')
                    ui.assert_pane(target,'/b');case.check(count('b')==0,'private first-tracker policy keeps a newly added backup silent',str(count('b')))
                    ui.keys(target,'x');ui.assert_pane(target,'[Cancel]');ui.keys(target,'Enter',literal=False)
                    case.check(urls(client,[a,b])[0],'Enter on default TUI Cancel performs no removal')
                    ui.keys(target,'x');ui.keys(target,'y')
                    h.wait_for(lambda:urls(client,[b]),'explicit TUI confirmation removes only the selected tracker')
                    h.wait_for(lambda:(count('b')>=2,str(count('b'))),'private backup begins real announces after first tracker removal')
                    ui.keys(target,'+');type_private('ftp://host.invalid/a?key=edit-private-invalid');ui.keys(target,'Enter',literal=False)
                    ui.assert_pane(target,'Invalid tracker URL');case.check(urls(client,[b])[0],'unsupported TUI tracker URL is rejected without changing the list')
                    ui.keys(target,'Escape',literal=False);ui.keys(target,'q')
                    h.wait_for(lambda:(done.exists(),ui.capture(target)),'TUI detaches while its edited tracker remains active')
                finally:ui.close()
            h.stop_serve(process);h.notify.close();process=start()
            with d.ApiConn(h,redact_urls=True) as client:
                client.hello();h.wait_for(lambda:urls(client,[b]),'TUI tracker additions and removals survive actual daemon restart')
                q=Session(case,box)
                phase='dashboard ready'
                try:
                    q.wait(lambda:q.ipc('ready')=='true')
                    q.shell('summon',json.dumps(dict(view='torrents',hash=hash_)))
                    q.wait(lambda:(q.state().get('view') or {}).get('cursor')==hash_)
                    q.keys(['-k','Return'],lambda:(q.state().get('view') or {}).get('focus')=='details')
                    case.check(q.ipc('clickDetailTab','trackers')=='true','actual dashboard Trackers button opens the editor surface')
                    def state():return json.loads(q.ipc('trackerEditorState'))
                    def text():return '\n'.join(json.loads(q.ipc('trackerRows')))
                    q.wait(lambda:'/b' in text());phase='masked dashboard add'
                    q.ipc('trackerButton','add');q.wait(lambda:state().get('mode')=='add')
                    q.keys([c],lambda:state().get('length')==len(c))
                    masked=state();case.check(masked['masked'] and 'edit-private-' not in masked['display'],'actual dashboard input masks the typed private URL',json.dumps(masked))
                    q.ipc('trackerButton','add-cancel');q.wait(lambda:state().get('mode')=='')
                    case.check(urls(client,[b])[0],'dashboard Add Cancel performs no mutation')
                    q.ipc('trackerButton','add');q.wait(lambda:state().get('mode')=='add')
                    q.keys([c],lambda:state().get('length')==len(c));q.ipc('trackerButton','add-submit')
                    h.wait_for(lambda:urls(client,[b,c]),'actual dashboard Add button saves the private backup')
                    q.wait(lambda:'/c' in text());case.check(count('c')==0,'dashboard edit preserves private first-tracker policy',str(count('c')))
                    phase='dashboard removal defaults to Cancel'
                    q.ipc('trackerButton','remove');q.wait(lambda:state().get('mode')=='remove')
                    case.check(state()['cancelFocused'],'dashboard Remove starts on Cancel',json.dumps(state()))
                    q.keys(['-k','Return'],lambda:state().get('mode')=='')
                    case.check(urls(client,[b,c])[0],'Enter on default dashboard Cancel performs no removal')
                    phase='explicit dashboard removal'
                    q.ipc('trackerButton','remove');q.wait(lambda:state().get('mode')=='remove')
                    q.keys(['-k','Right','-k','Return'],lambda:state().get('mode')=='')
                    h.wait_for(lambda:urls(client,[c]),'explicit dashboard confirmation removes the selected exact URL')
                    q.wait(lambda:'/c' in text() and '/b' not in text())
                    h.wait_for(lambda:(count('c')>=2,str(count('c'))),'dashboard-added tracker receives real announces')
                    case.check('passkey=' not in text() and 'edit-private-' not in text(),'live dashboard tracker text hides credentials',text())
                    image=case.path.with_suffix('.dashboard.png');q.ipc('grab',str(image));q.wait(lambda:image.exists())
                    q.shell('hide');q.wait(lambda:q.state().get('view') is None)
                except Exception:
                    case.check(False,phase,json.dumps(dict(view=q.state(),editor=state(),rows=q.ipc('trackerRows'))));raise
                finally:q.close()
            h.stop_serve(process);h.notify.close();before_a=count('a');before_b=count('b');before_c=count('c');process=start()
            with d.ApiConn(h,redact_urls=True) as client:
                client.hello();h.wait_for(lambda:urls(client,[c]),'dashboard edits survive a second actual daemon restart')
                h.wait_for(lambda:(count('c')>=before_c+2,str(count('c'))),'restored private tracker continues real announces')
                case.check(count('a')==before_a and count('b')==before_b,'removed trackers stay silent after restore',json.dumps(dict(a=count('a'),b=count('b'),c=count('c'))))
            h.stop_serve(process)
            logs=case.path.read_text()+h.stderr()+case.path.with_suffix('.qs.log').read_text()
            case.check('edit-private-' not in logs and 'passkey=' not in logs,'tracker editor diagnostics never echo typed or saved credentials','Scanned owned harness, TUI captures, daemon and QML logs; exact URLs stay only in private owned ledger/socket data.')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,default=ROOT/'target/debug/fbittorrent')
    args=parser.parse_args();run(args.binary.resolve());sys.exit(bool(suite_status('daemon')))
