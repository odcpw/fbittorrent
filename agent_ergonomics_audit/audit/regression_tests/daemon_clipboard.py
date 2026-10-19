"""P6-13: private clipboard producer, real NUL wrapper, API actions and privacy."""
import json
import shlex
import sys
import time
import daemon as d
import fixtures

HASH='c7'*20
NAME='PrivateClipboardFixture'
MAGNET='magnet:?xt=urn:btih:'+HASH+'&dn='+NAME
SECRET='clipboard-secret-never-log-4711'


def run(name,binary,systemd):
    with d.Case('daemon',name) as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        config=d.Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('notify = false','notify = true'))
        inputs=box.root/'selections.ndjson';inputs.touch()
        trace=case.path.with_suffix('.clipboard.ndjson')
        program=box.root/'wl-paste.py'
        program.write_text('''import json,os,signal,subprocess,sys,time
input_path,log=sys.argv[1:3]
command=sys.argv[4:]
descendant=subprocess.Popen(['sleep','infinity'])
def emit(ev,**extra):
 with open(log,'a') as f:f.write(json.dumps(dict(ev=ev,at=time.monotonic(),pid=os.getpid(),descendant=descendant.pid,**extra))+'\\n')
def stop(*_):emit('SIGTERM');sys.exit(0)
signal.signal(signal.SIGTERM,stop)
emit('start',argv=sys.argv[3:])
time.sleep(1)
with open(input_path) as f:
 while True:
  line=f.readline()
  if not line:time.sleep(.02);continue
  text=json.loads(line)
  emit('selection',text=text[:160],bytes=len(text.encode()),command=command)
  print('clipboard-secret-never-log-4711',file=sys.stderr,flush=True)
  subprocess.run(command,input=text.encode(),check=True)
''')
        fixtures.path_shim('wl-paste','exec '+shlex.join([sys.executable,str(program),str(inputs),str(trace)])+' "$@"',box.root)
        action='add' if name=='clipboard_watch_add' else 'dismiss' if name=='clipboard_watch_offer' else ''
        fixtures.path_shim('notify-send','printf %s '+shlex.quote(NAME+' '+SECRET)+' >&2\nprintf %s '+shlex.quote(action),box.root)
        default=h.cli('settings')
        case.check(default.get('clipboard_watch') is False and not trace.exists(),'watch defaults off; a read starts no clipboard process',json.dumps(default))
        p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        try:
            with d.ApiConn(h) as c:
                hello=c.hello()['result']
                case.check('clipboard' in hello['caps'],'hello advertises clipboard capability',json.dumps(hello))
                c.call('subscribe',dict(topics=['events'],interval_ms=250))
                case.check(c.call('app.settings.get')['result']['clipboard_watch'] is False and not trace.exists(),'running default has no reader',str(trace.exists()))
                ack=h.cli('settings','--clipboard-watch','on')
                case.check(ack.get('clipboard_watch') is True and not ack.get('restart'),'CLI on applies live without engine restart',json.dumps(ack))
                def rows():return [json.loads(l) for l in trace.read_text().splitlines()] if trace.exists() else []
                h.wait_for(lambda:(any(r['ev']=='start' for r in rows()),json.dumps(rows())),'one fake wl-paste starts')
                started=rows()[0]
                case.check(started['argv']==['--watch','sh','-c','cat; printf "\\0"'],'actual command is the approved NUL wrapper',json.dumps(started))
                def send(text):
                    with inputs.open('a') as f:f.write(json.dumps(text)+'\n')
                    case.step('copy owned selection','producer delivers this selection through real sh/cat').ok(json.dumps(dict(text=text[:160],bytes=len(text.encode()),at=time.monotonic())))
                def events():
                    c.call('app.info');out=[]
                    for frame in c.pushed:
                        if frame.get('ev')=='clipboard.magnet':out.append(frame)
                        out.extend(ev for ev in frame.get('added',frame.get('items',[])) if ev.get('ev')=='clipboard.magnet')
                    return out
                send(MAGNET.replace(HASH,'d8'*20).replace(NAME,'DroppedOversized')+'&padding='+'x'*65536);send(SECRET);send(MAGNET)
                h.wait_for(lambda:(len(found:=events())==1,json.dumps(found)),'one bounded clipboard.magnet after producer delay',timeout=5)
                ev=events()[0]
                case.check(ev['hash']==HASH and ev.get('name')==NAME and ev['already_added'] is False,'complete selection supplies offered identity',json.dumps(ev))
                notifications=box.root/'shims/notify-send.log'
                h.wait_for(lambda:(notifications.exists() and 'Magnet on clipboard: '+NAME in notifications.read_text(),notifications.read_text() if notifications.exists() else 'none'),'one magnet notification')
                argv=[json.loads(l) for l in notifications.read_text().splitlines()]
                case.check(len(argv)==1 and 'add=Add' in argv[0] and 'dismiss=Dismiss' in argv[0],'one notification exposes API actions',json.dumps(argv))
                if name=='clipboard_watch_add':
                    def added():
                        result=c.call('torrents.list',{})['result']['torrents']
                        return any(row['hash']==HASH and row.get('origin')=='clipboard' for row in result),json.dumps(result)
                    h.wait_for(added,'notification Add inserts a real clipboard-origin ledger record')
                    case.step('notification action request','Add uses torrents.add over api.sock').ok(json.dumps(dict(op='torrents.add',args=dict(source=dict(magnet='magnet:?xt=urn:btih:'+HASH),origin='clipboard'))))
                else:
                    c.call('app.clipboard.dismiss',dict(hash=HASH),ref='clipboard-dismiss')
                    c.call('app.clipboard.dismiss',dict(hash=HASH),ref='clipboard-dismiss')
                send(MAGNET);send(MAGNET)
                h.wait_for(lambda:(len([r for r in rows() if r['ev']=='selection'])>=5,json.dumps(rows())),'producer delivers repeated selections')
                time.sleep(.35)
                case.check(len(events())==1 and len(notifications.read_text().splitlines())==1,'repeats create no event or notification',json.dumps(events()))
                if name=='clipboard_watch_dedupe':
                    payload=box.root/'ledger.bin';payload.write_bytes(b'owned ledger fixture')
                    item=fixtures.torrent_fixture(box.root,[payload])
                    c.call('torrents.add',dict(source=dict(path=str(item.torrent)),stopped=True))
                    send('magnet:?xt=urn:btih:'+item.info_hash+'&dn=AlreadyAdded')
                    h.wait_for(lambda:(len([r for r in rows() if r['ev']=='selection'])>=6,json.dumps(rows())),'producer copies an existing ledger hash')
                    time.sleep(.35)
                    case.check(len(events())==1,'ledger deduplication works independently of last hash',json.dumps(events()))
                combined=h.stderr()+json.dumps(c.call('app.log',dict(limit=1000))['result'])
                needles=[SECRET,NAME,MAGNET,'AlreadyAdded','DroppedOversized'];matches=[s for s in needles if s in combined]
                case.check(not matches,'fixed-string grep of daemon stderr and app.log is empty',json.dumps(dict(command='scan stderr and app.log',needles=needles,matches=matches)))
                off=h.cli('settings','--clipboard-watch','off');began=time.monotonic()
                h.wait_for(lambda:(not d.process_alive(started['pid']) and not d.process_alive(started['descendant']),json.dumps(dict(pid=started['pid'],descendant=started['descendant'],at=time.monotonic(),trace=rows()))),'off kills reader and descendant',timeout=5)
                case.check(off.get('clipboard_watch') is False,'CLI off persists the setting',json.dumps(off))
                if name=='clipboard_watch_off_kills_child':
                    h.cli('settings','--clipboard-watch','on')
                    h.wait_for(lambda:(len([r for r in rows() if r['ev']=='start'])==2,json.dumps(rows())),'on creates one replacement reader')
                    second=[r for r in rows() if r['ev']=='start'][-1]
                    c.call('app.settings.set',dict(idle_exit_secs=30))
                    p.wait(timeout=35)
                    case.check(not d.process_alive(second['pid']) and not d.process_alive(second['descendant']) and 'reason=Idle code=0' in h.stderr(),'watcher is not busy; idle drain kills its group',h.stderr()[-1000:])
                case.step('retain fixture observations','producer timestamps, selection prefixes and notification argv remain reviewable').ok(json.dumps(dict(trace=rows(),notify=argv,off_elapsed=time.monotonic()-began)),[trace])
        finally:
            if p.child.poll() is None:h.stop_serve(p)


def clipboard_watch_offer(binary,systemd):run('clipboard_watch_offer',binary,systemd)
def clipboard_watch_add(binary,systemd):run('clipboard_watch_add',binary,systemd)
def clipboard_watch_dedupe(binary,systemd):run('clipboard_watch_dedupe',binary,systemd)
def clipboard_watch_off_kills_child(binary,systemd):run('clipboard_watch_off_kills_child',binary,systemd)
def clipboard_never_logged(binary,systemd):run('clipboard_never_logged',binary,systemd)
