"""P6-09: real port restart and owned inhibitor process-group lifetime."""
import json
import shlex
import socket
import sys
import time

import daemon as d
import fixtures


def port_restart_keeps_clients(binary,systemd):
    with d.api_owned('port_restart_keeps_clients',binary,systemd) as (case,box,h,p),d.ApiConn(h) as c:
        hello=c.hello()['result'];before=c.call('app.info')['result']
        c.call('subscribe',dict(topics=['events']))
        with socket.socket() as probe:
            probe.bind(('127.0.0.1',0));port=probe.getsockname()[1]
        ack=h.cli('settings','--port',str(port))
        case.check(ack.get('restart') is True and ack.get('applied')==['peer_port'],'CLI port edit uses the durable restart-class transaction',json.dumps(ack))
        def ready():
            info=c.call('app.info')['result']
            return info['state']=='running' and info['engine_epoch']==before['engine_epoch']+1,json.dumps(info)
        h.wait_for(ready,'the existing API connection answers after the port restart',timeout=15)
        after=c.call('app.info')['result'];settings=c.call('app.settings.get')['result']
        def transitions():
            c.call('app.info')
            events=[ev for ev in c.pushed if ev.get('ev')=='engine.changed']
            return len(events)>=2,json.dumps(events)
        h.wait_for(transitions,'existing subscription receives starting and running engine.changed',timeout=3)
        events=[ev for ev in c.pushed if ev.get('ev')=='engine.changed']
        case.check(after['instance']==hello['instance'] and after['server']['pid']==before['server']['pid'] and settings['peer_port']==port and any(ev.get('state')=='starting' for ev in events) and any(ev.get('state')=='running' for ev in events),
                   'same instance and client survive both engine transitions and persist the selected port',json.dumps(dict(instance=hello['instance'],events=events,settings=settings)))
        with socket.create_connection(('127.0.0.1',port),timeout=2):
            case.step('connect to the selected private fixture peer port','new TCP listener accepts on loopback').ok(str(port))


def inhibitor_lifecycle(binary,systemd):
    with d.Case('daemon','inhibitor_lifecycle') as case,fixtures.daemon_pair(case,binary,systemd,65536) as pair:
        trace=case.path.with_suffix('.inhibit.ndjson')
        program=pair.box.root/'inhibitor.py'
        program.write_text('''import json,os,signal,subprocess,sys,time
out=sys.argv[1]
p=subprocess.Popen(['sleep','infinity'])
def emit(ev):
 with open(out,'a') as f:f.write(json.dumps(dict(ev=ev,pid=os.getpid(),descendant=p.pid,at=time.monotonic()))+'\\n')
def stop(*_):emit('SIGTERM');sys.exit(0)
signal.signal(signal.SIGTERM,stop)
emit('start')
while True:signal.pause()
''')
        fixtures.path_shim('systemd-inhibit','exec '+shlex.join([sys.executable,str(program),str(trace)]),pair.box.root)
        def rows():return [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
        pair.h.wait_for(lambda:(len([r for r in rows() if r['ev']=='start'])==1,json.dumps(rows())),'one owned shim child holds inhibition during actual downloading',timeout=7)
        started=rows()[0]
        state=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        time.sleep(.3)
        case.check(state['state']=='downloading' and d.process_alive(started['pid']) and d.process_alive(started['descendant']) and len([r for r in rows() if r['ev']=='start'])==1,
                   'one inhibitor group exists during the real transfer; no second child is launched',json.dumps(dict(row=state,shim=rows())))
        began=time.monotonic();pair.client.call('torrents.stop',dict(hashes=[pair.hash]))
        pair.h.wait_for(lambda:(any(r['ev']=='SIGTERM' for r in rows()) and not d.process_alive(started['pid']) and not d.process_alive(started['descendant']),json.dumps(rows())),
                        'last download stop kills the launcher and its descendant within five seconds',timeout=5)
        case.check(any(r['ev']=='SIGTERM' and r['at']-began<=5 for r in rows()),'recorded SIGTERM meets the stop deadline',json.dumps(rows()))
        pair.client.call('torrents.start',dict(hashes=[pair.hash]))
        pair.h.wait_for(lambda:(len([r for r in rows() if r['ev']=='start'])==2,json.dumps(rows())),'resuming work creates one new inhibitor',timeout=5)
        second=[r for r in rows() if r['ev']=='start'][-1]
        pair.h.stop_serve(pair.p)
        case.check(not d.process_alive(second['pid']) and not d.process_alive(second['descendant']) and len([r for r in rows() if r['ev']=='SIGTERM'])==2,
                   'daemon drain kills the second group before isolation is removed',json.dumps(rows()))
        case.step('retain shim timestamps and state transitions','start/SIGTERM evidence is available after sandbox deletion').ok(json.dumps(rows()),[trace])
