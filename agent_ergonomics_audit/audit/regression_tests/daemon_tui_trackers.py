"""P6-01 TUI tracker status/reannounce with the real fixture and private query."""
import json
import time
import daemon as d
import fixtures


def tui_trackers(binary,systemd):
    import native
    with d.api_owned('tui_trackers',binary,systemd) as (case,box,h,process), fixtures.tracker_server(
            fail_first=1,interval_s=600,peers=[('127.0.0.1',9)],directory=box.root,echo_failure=True) as tracker, d.ApiConn(h,redact_urls=True) as client:
        client.hello()
        fixture=d.tracker_live_fixture(box,tracker.url+'?passkey=tui-private-token',name='tracker-tui.bin')
        client.call('torrents.add',dict(source=dict(path=str(fixture.torrent))))
        h.wait_for(lambda:((rows:=d.tracker_rows(client,fixture.info_hash))[0]['status']=='error',client.diagnostic(rows)),'first real announce fails before the TUI opens')
        native.prepare_imdb_fixture(box.env,['tracker-tui.bin'])
        ui=native.AttachedTui(case,str(binary),box,h,session='fbt-fbt-p6-01-ymi')
        try:
            target,done=ui.start();ui.assert_pane(target,'tracker-tui.bin')
            ui.keys(target,'Enter',literal=False);ui.assert_pane(target,'[Overview]')
            ui.keys(target,']]]');ui.assert_pane(target,'[Trackers]')
            ui.assert_pane(target,'! Error');ui.assert_pane(target,'fixture: not yet')
            ui.assert_pane(target,'(e) Reannounce')
            topic='torrent:'+fixture.info_hash+'.trackers'
            h.wait_for(lambda:((counts:=client.call('app.info')['result']['subscriptions']['by_topic']).get(topic)==1,json.dumps(counts)),'only the visible Trackers tab owns its live topic')
            for width,height in [(120,30),(80,24),(160,45)]:
                resized=ui.tmux('resize-window','-t',target,'-x',str(width),'-y',str(height))
                case.check(resized.returncode==0,'resize tracker panel to '+str((width,height)),resized.stderr)
                x,y=(0,0) if width<100 or height<30 else ((width-100)//2,(height-30)//2)
                def aligned():
                    pane=ui.capture(target);lines=pane.splitlines()
                    return len(lines)>y and lines[y].find('╭ tracker-tui.bin')==x,pane
                h.wait_for(aligned,'tracker panel redraws at the resized geometry')
                ui.assert_pane(target,'! Error');pane=ui.capture(target)
                path=case.path.with_suffix('.trackers-'+str(width)+'.txt');path.write_text(pane)
                case.check('fixture: not yet' in pane and 'passkey=' not in pane and 'tui-private-token' not in pane and '/announce?' not in pane,
                           'tracker error text and query-free URL render at '+str(width)+' columns',str(path)+'\n'+pane)
            started=time.time();ui.keys(target,'e')
            h.wait_for(lambda:((requests:=tracker.snapshot()) and len(requests)==2,json.dumps(requests)),'e sends the forced announce within two seconds',timeout=2)
            requests=tracker.snapshot()
            case.check(len(requests)==2 and 0<=requests[-1]['ts']-started<2 and requests[-1]['event']=='none',
                       'keyboard reannounce bypasses the ten-second backoff',json.dumps(requests))
            ui.assert_pane(target,'✓ OK',timeout=2)
            after=d.tracker_rows(client,fixture.info_hash)
            case.check(after[0]['status']=='ok' and after[0]['peers']==1 and '1' in ui.capture(target),
                       'the panel reflects the successful real response and one returned peer',client.diagnostic(after)+'\n'+ui.capture(target))
            ui.assert_pane(target,'(e) Reannounce')
            ui.keys(target,'e');ui.assert_pane(target,'Reannounce requested')
            time.sleep(.35)
            case.check(len(tracker.snapshot())==2,'second keyboard force within sixty seconds adds no announce',json.dumps(tracker.snapshot()))
            client.call('torrents.stop',dict(hashes=[fixture.info_hash]))
            ui.keys(target,'r');ui.assert_pane(target,'Trackers refreshed');ui.assert_pane(target,'✓ OK')
            case.check(d.tracker_rows(client,fixture.info_hash)==after,'paused torrent retains its final recorded tracker status',client.diagnostic(d.tracker_rows(client,fixture.info_hash)))
            ui.keys(target,'e');ui.assert_pane(target,'not_live')
            case.check(len(tracker.snapshot())==2,'paused keyboard force cannot announce',json.dumps(tracker.snapshot()))
            ui.keys(target,'Escape',literal=False)
            h.wait_for(lambda:((counts:=client.call('app.info')['result']['subscriptions']['by_topic']).get(topic,0)==0,json.dumps(counts)),'closing details releases the tracker topic')
            ui.keys(target,'L')
            pane=ui.capture(target)
            case.check('tui-private-token' not in pane and 'passkey=' not in pane,'retained TUI log/history contains no query or echoed passkey',pane)
            ui.keys(target,'Escape',literal=False);ui.keys(target,'q')
            h.wait_for(lambda:(done.exists(),ui.capture(target)),'tracker UI detaches without stopping its daemon')
            logs=h.stderr()+case.path.read_text()+tracker.log.read_text()
            case.check('tui-private-token' not in logs and 'passkey=' not in logs,'daemon, JSONL and tracker logs contain no query', 'Scanned private logs and captured panes.')
        finally:ui.close()
