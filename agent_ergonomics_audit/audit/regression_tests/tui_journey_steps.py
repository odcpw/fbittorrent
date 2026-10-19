"""Behaviour checks after each TUI add source; all transfer evidence is real."""
import hashlib
import json
import re
import time


def exercise(ui,f):
    c,h,case=f.client,f.h,f.case
    def need(value,label):
        if not value:raise RuntimeError(label)
    def press(key,text,literal=True):need(ui.contains(key,text,literal=literal),'missing '+text)
    def wait(fn,label,timeout=10):need(h.wait_for(fn,label,timeout=timeout),label)
    def observe(label,predicate,timeout=5):need(ui.observe(label,label,predicate,timeout),label)
    def tab(name):
        need(ui.details_tab(name),'details offers '+name)
    def payload():
        root=f.box.downloads
        return {str(path.relative_to(root)):hashlib.sha256(path.read_bytes()).hexdigest() for path in root.rglob('*') if path.is_file()}
    wait(lambda:((row:=f.row()).get('completed_bytes',0)>0 and row['state']=='downloading',json.dumps(row)),'real loopback payload begins',15)
    observe('80-column list shows a state word and measured progress',lambda p:f.name in p and 'Down' in p and re.search(r'\d+(?:\.\d+)?%',p) and len(p.splitlines())==24)
    before=f.row()['completed_bytes'];first=ui.capture(ui.target)
    wait(lambda:((row:=f.row())['completed_bytes']>before and row.get('dl_bps',0)>0,json.dumps(row)),'real payload bytes and measured download rate advance')
    observe('pushed progress redraws without manual refresh',lambda p:p!=first and 'Down' in p)
    press('Enter','[Overview]',False)
    tab('Files');observe('Files shows real payload names and include controls',lambda p:all(name in p for name in ('Episode.mkv','English.srt','sample.mkv','include/skip')))
    ui.press('End','select the final natural-order file',literal=False)
    observe('sample is the selected file',lambda p:any('▌' in line and 'sample.mkv' in line for line in p.splitlines()))
    def included(on):
        rows=c.call('torrents.files',dict(hash=f.torrent.info_hash))['result']['files']
        return next(r for r in rows if r['path'].endswith('sample.mkv'))['included']==on,json.dumps(rows)+'\n'+ui.capture(ui.target)
    ui.press(' ','skip the selected sample');wait(lambda:included(False),'file selection is applied by the daemon')
    observe('file skip is confirmed in the TUI',lambda p:any('▌' in line and 'sample.mkv' in line and '[ ]' in line for line in p.splitlines()) and 'Saving file selection' not in p)
    ui.press(' ','restore the selected sample');wait(lambda:included(True),'file inclusion restores without losing the torrent')
    observe('file inclusion is confirmed in the TUI',lambda p:any('▌' in line and 'sample.mkv' in line and '[x]' in line for line in p.splitlines()) and 'Saving file selection' not in p)
    tab('Peers');observe('Peers shows the actual loopback seed',lambda p:'127.0.0.1' in p and 'SEED' in p and 'yes' in p)
    tab('Trackers');observe('Trackers shows a real successful announce',lambda p:'/announce' in p and 'OK' in p)
    tab('Log');observe('Log exposes filters and an honest history or empty state',lambda p:'level:' in p.lower() or 'No matching messages' in p)
    ui.press('Escape','close details',lambda p:'[Torrents]' in p,literal=False)
    press('?','GLOBAL')
    seen=ui.capture(ui.target)
    for _ in range(20):
        if 'Remove' in seen and 'Settings' in seen:break
        previous=ui.capture(ui.target)
        need(ui.press('PageDown','page through the available Help commands',lambda p:p!=previous,literal=False),'Help cannot be paged')
        seen+='\n'+ui.capture(ui.target)
    need(case.check('Remove' in seen and 'Settings' in seen,'Help exposes commands omitted by a narrow footer',seen),'Help lost an available command')
    ui.press('Escape','return from Help',lambda p:'GLOBAL' not in p,literal=False)
    ui.press('p','pause the selected torrent');wait(lambda:((row:=f.row())['state']=='paused',json.dumps(row)),'p confirms paused state')
    observe('paused state and Resume action reach the terminal',lambda p:'Pause' in p and '(p) Resume' in p and 'Stopping' not in p)
    stopped=f.row()['completed_bytes'];time.sleep(.2)
    case.check(f.row()['completed_bytes']==stopped,'paused transfer stops gaining payload',str(stopped))
    ui.press('p','resume the selected torrent');wait(lambda:((row:=f.row())['state']=='downloading' and row['completed_bytes']>stopped,json.dumps(row)),'p resumes real payload')
    observe('resumed progress and Pause action reach the terminal',lambda p:'Down' in p and '(p) Pause' in p and 'Starting' not in p)
    ui.press('p','pause before the keep-files safety check');wait(lambda:((row:=f.row())['state']=='paused',json.dumps(row)),'payload is stable before removal')
    observe('the second pause is confirmed before removal',lambda p:'Pause' in p and '(p) Resume' in p and 'Stopping' not in p)
    preserved=payload();case.check(bool(preserved),'real downloaded files exist before removal',json.dumps(list(preserved)))
    press('D','AND ITS FILES');ui.press('Enter','default confirmation cancels file deletion',lambda p:'AND ITS FILES' not in p,literal=False)
    case.check(payload()==preserved and len(f.rows())==1,'default Cancel preserves record and exact payload bytes')
    press('d','keep files');ui.press('y','confirm keep-files removal')
    wait(lambda:(not f.rows(),ui.capture(ui.target)),'confirmed removal hides the row')
    case.check(payload()==preserved,'keep-files removal preserves every downloaded byte')
    observe('Undo is offered after confirmed removal',lambda p:'(U)' in p and 'Undo' in p)
    ui.press('U','undo the confirmed removal');wait(lambda:(len(f.rows())==1,ui.capture(ui.target)),'Undo restores the same hash')
    wait(lambda:((row:=f.row())['state']=='paused',json.dumps(row)),'Undo restores paused intent after its real check')
    case.check(payload()==preserved,'Undo retains the saved payload bytes')
    observe('the restored paused row is ready for its next action',lambda p:f.name in p and 'Pause' in p and '(p) Resume' in p and 'Removal undone' in p)
    ui.press('p','resume the restored torrent');wait(lambda:((row:=f.row())['state']=='downloading',json.dumps(row)),'restored download resumes')
    original=c.call('app.settings.get')['result']['max_active_downloads']
    press('F6','Download settings',False);ui.press('Right','change the queue-limit draft',literal=False);ui.press('Escape','cancel the settings draft',literal=False)
    case.check(c.call('app.settings.get')['result']['max_active_downloads']==original,'canceling Settings persists no draft')
    press('F6','Download settings',False);ui.press('Right','edit the real queue limit',literal=False);ui.press('Enter','save Settings',literal=False)
    wait(lambda:((settings:=c.call('app.settings.get')['result'])['max_active_downloads']==original+1,json.dumps(settings)),'Settings saves through API v1')
    press('F6','Download settings',False);observe('reopened Settings shows the saved queue limit',lambda p:re.search(r'Simultaneous downloads\s+'+str(original+1),p))
    ui.press('Escape','close Settings',literal=False)
    press('a','seed:off');ui.press(str(f.other.torrent),'type a second real file source');ui.press('Enter','start a second real download',literal=False)
    wait(lambda:((rows:=f.rows()) and len(rows)==2 and all(r['state']=='downloading' and r.get('completed_bytes',0)>0 for r in rows),json.dumps(rows)),'two real downloads are active for the shutdown control',15)
    before=f.row()['completed_bytes'];ui.press('q','q detaches without stopping the owner',lambda p:ui.done.exists() and ui.done.read_text()=='0' and f.process.child.poll() is None)
    wait(lambda:((row:=f.row())['completed_bytes']>before,json.dumps(row)),'payload continues after detaching the terminal')
    ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','80','-y','24')
    observe('reattach sees both real downloads',lambda p:f.name in p and f.other_name in p)
    press('Q','2 downloads');ui.press('Enter','default Cancel leaves both downloads running',lambda p:'2 downloads' not in p,literal=False)
    case.check(f.process.child.poll() is None and not ui.done.exists(),'Enter cannot confirm Q')
    press('Q','2 downloads');ui.press('y','explicit Q confirmation stops the owner and this TUI',lambda p:ui.done.exists() and ui.done.read_text()=='0' and f.process.child.poll()==0 and not h.engine_path.exists())
    h.lock_free()
