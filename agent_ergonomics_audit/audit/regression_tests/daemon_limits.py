"""P5-04: live session limits, independent torrent caps and durable turtle state."""
import json
import time
import daemon as d
import fixtures


def alt_limit_holds(binary,systemd):
    with d.Case('daemon','alt_limit_holds') as case,fixtures.daemon_pair(case,binary,systemd,262144) as pair:
        c=pair.seed_client
        c.call('subscribe',dict(topics=['summary'],interval_ms=250))
        c.call('transfer.set_limits',dict(alt_upload_bps=65536))
        began=time.monotonic();c.call('transfer.set_alt',dict(on=True))
        frames=[f for f in c.pushed if f.get('ev')=='state' and f.get('topic')=='summary' and f.get('data',{}).get('alt') is True]
        c.socket.settimeout(2)
        try:
            while not frames:
                frame=c.receive()
                if frame is None:raise EOFError('summary connection closed')
                if frame.get('ev')=='state' and frame.get('topic')=='summary' and frame.get('data',{}).get('alt') is True:frames.append(frame)
        finally:c.socket.settimeout(8)
        elapsed=time.monotonic()-began
        case.check(elapsed<=.25,'structural turtle state arrives within the 250ms subscription interval',json.dumps(dict(elapsed=elapsed,frames=frames)))
        time.sleep(5)
        before=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']['completed_bytes']
        start=time.monotonic();prior=before;previous=start
        for second in range(20):
            deadline=start+second+1;time.sleep(max(0,deadline-time.monotonic()))
            row=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent'];now=time.monotonic()
            completed=row['completed_bytes']
            case.step('sample actual limited transfer second '+str(second+1),'running incomplete loopback transfer with observed byte delta').close(
                row['state']=='downloading' and completed>=prior,
                json.dumps(dict(completed_bytes=completed,delta_bytes=completed-prior,elapsed=now-previous,bps=(completed-prior)/(now-previous))))
            prior,previous=completed,now
        rate=(prior-before)/(previous-start)
        case.check(49152<=rate<=81920,'64 KiB/s turtle transfer holds within ±25% over 20s after 5s warmup',json.dumps(dict(bytes=prior-before,seconds=previous-start,bps=rate)))


def global_unlimited_keeps_cap(binary,systemd):
    with d.Case('daemon','global_unlimited_keeps_cap') as case,fixtures.daemon_pair(case,binary,systemd,65536) as pair:
        before=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        changed=pair.client.call('transfer.set_limits',dict(upload_bps=0))['result']
        after=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        case.check(before.get('upload_limit_bps',0)==after.get('upload_limit_bps',0)==0 and before['upload_enabled'] is after['upload_enabled'] is False and 'upload_bps' not in changed['limits'],
                   'global upload 0 is unlimited and leaves default upload permission off',json.dumps(dict(before=before,after=after,reply=changed)))
        pair.client.call('transfer.set_limits',dict(upload_bps=1),error='invalid_args')
        pair.client.call('transfer.set_limits',dict(upload_bps=-1),error='invalid_args')
        pair.client.call('transfer.set_limits',dict(upload_bps=4294967296),error='invalid_args')


def alt_survives_restart(binary,systemd):
    with d.Case('daemon','alt_survives_restart') as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        for lap in range(2):
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as client:
                hello=client.hello()
                case.check('limits' in hello['result']['caps'],'limits capability is negotiated',json.dumps(hello))
                if lap==0:
                    client.call('transfer.set_limits',dict(download_bps=131072,alt_download_bps=65536,alt_upload_bps=32768))
                    client.call('transfer.set_alt',dict(on=True))
                    kept=client.call('transfer.set_limits',dict(download_bps=None))['result']['limits']
                    case.check(kept==dict(alt_download_bps=65536,alt_upload_bps=32768),'omission retains alternatives while null clears primary',json.dumps(kept))
                value=h.cli('limits')
                case.check(value.get('alt') is True and value.get('limits')==dict(alt_download_bps=65536,alt_upload_bps=32768),
                           'same turtle selection and limits survive clean daemon restart',json.dumps(value))
            h.stop_serve(p);h.notify.close()
        h.lock_free()


def limits_verb(binary,systemd):
    from agent_contract import cli
    with d.Case('agent_contract','limits_verb') as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        value,_=cli(case,box,binary,'limits','--alt-down','64K','--alt-up','32K','--alt','on','--json')
        case.check(value.get('alt') is True and value.get('running') is False,'offline limits persist without starting a daemon',json.dumps(value))
        case.check(not d.DaemonContract.socket_path(box).exists(),'offline limits do not create api.sock',str(box.root))
        cli(case,box,binary,'limits','--down','1','--json',code=2)
        p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        value=h.cli('limits','--up','0','--down','1M','--alt','off')
        case.check(value.get('alt') is False and value.get('limits')==dict(download_bps=1048576,alt_download_bps=65536,alt_upload_bps=32768),
                   'live limits CLI applies all requested fields without lifting torrent caps',json.dumps(value))
        h.stop_serve(p)
