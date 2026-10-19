"""P6-03: real sampler clock, bounded pushed history and actual loopback rates."""
import json
import time
import daemon as d
import fixtures


def history_points(binary,systemd):
    with d.api_owned('history_points',binary,systemd) as (case,box,h,p),d.ApiConn(h) as c:
        hello=c.hello()['result'];case.check('history' in hello['caps'],'hello advertises implemented history',json.dumps(hello['caps']))
        for args in ({},{'history':False}):
            result=c.call('app.stats',args)['result']
            case.check('history' not in result,'history is opt-in on app.stats',json.dumps(result))
        c.call('app.stats',dict(history=1),error='invalid_args')
        sub=c.call('subscribe',dict(topics=['stats','summary'],interval_ms=250))['result']['sub']
        frames=[];stats=[];began=time.monotonic()
        for frame in c.events():
            if frame is None:raise EOFError('history stream EOF')
            frames.append(frame)
            if frame.get('ev')=='state' and frame.get('topic')=='stats':stats.append((time.monotonic(),frame))
            if len(stats)==2:break
            if len(frames)>64 or time.monotonic()-began>8:raise RuntimeError('bounded history capture exceeded')
        histories=[frame['data']['history'] for _,frame in stats]
        case.check(stats[1][0]-stats[0][0]>=4.75 and all(len(v['download'])==len(v['upload'])<=600 and v['interval_s']==1 for v in histories),
                   'stats has ordered matching arrays and a five-second cadence even with a 250ms requested interval',json.dumps(dict(elapsed=stats[1][0]-stats[0][0],frames=frames)))
        summaries=[f['data']['history'] for f in frames if f.get('topic')=='summary' and f.get('ev')=='state']
        case.check(bool(summaries) and all(len(v)<=60 for v in summaries),'summary carries at most sixty real download points',json.dumps(summaries))
        c.call('unsubscribe',dict(sub=sub));before=len(c.call('app.stats',dict(history=True))['result']['history']['download'])
        time.sleep(1.1)
        after=c.call('app.stats',dict(history=True))['result']['history']
        case.check(len(after['download'])>before,'the single global sampler keeps running without subscribers',json.dumps(dict(before=before,after=after)))
        counts=c.call('app.info')['result']['subscriptions']
        case.check(counts['total']==0,'last unsubscribe releases the stats transport',json.dumps(counts))


def status_history_field(binary,systemd):
    from agent_contract import cli
    with d.Case('agent_contract','status_history_field') as case,fixtures.daemon_pair(case,binary,systemd,65536) as pair:
        # 8 MiB at 64 KiB/s remains incomplete for the entire real minute.
        start=time.monotonic();values=[]
        for second in range(62):
            time.sleep(max(0,start+second+1-time.monotonic()))
            history=pair.client.call('app.stats',dict(history=True))['result']['history']
            values=history['download']
            case.step('observe real one-second rate history '+str(second+1),'one global sample per elapsed second, no generated padding').close(
                len(values)==len(history['upload'])<=600 and history['interval_s']==1,
                json.dumps(dict(elapsed=time.monotonic()-start,length=len(values),download=values,upload=history['upload'])))
            if len(values)>=60 and all(n>0 for n in values[-10:]):break
        value,_=cli(case,pair.box,binary,'status','--json')
        history=value.get('history',[])
        row=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        case.check(len(history)==60 and all(n>0 for n in history[-10:]) and row['state']=='downloading' and row['completed_bytes']<row['size'],
                   'status has sixty actual samples with the last ten nonzero during a local incomplete transfer',
                   json.dumps(dict(history=history,last_ten=history[-10:],torrent=row,seconds=time.monotonic()-start)))
