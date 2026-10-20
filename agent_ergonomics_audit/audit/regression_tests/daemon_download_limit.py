"""P7-06: actual download buckets, state continuity, durable intent and CLI syntax."""
import json
import time
from pathlib import Path
import daemon as d
import fixtures


def download_cap_holds(binary,systemd):
    with d.Case('daemon','download_cap_holds') as case,fixtures.daemon_pair(case,binary,systemd,262144) as pair:
        c=pair.client;args=dict(hashes=[pair.hash],bps=65536)
        c.call('transfer.set_limits',dict(download_bps=262144,alt_download_bps=32768))
        # Bound the baseline fixture: do not measure requests reserved before
        # the cap existed. Install it while paused, then admit transfer with it.
        c.call('torrents.stop',dict(hashes=[pair.hash]))
        c.call('torrents.set_download_limit',args);c.call('torrents.set_download_limit',args)
        c.call('torrents.start',dict(hashes=[pair.hash]))
        pair.h.wait_for(lambda:((row:=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'])['state']=='downloading',json.dumps(row)),'resume the actual transfer with the baseline cap already installed')
        c.call('subscribe',dict(topics=['torrents'],interval_ms=250))
        epoch=c.call('app.info')['result']['engine_epoch']
        c.call('torrents.set_download_limit',dict(hashes=[pair.hash],bps=1),error='invalid_args')
        c.call('torrents.set_download_limit',dict(hashes=[pair.hash],bps=4294967296),error='invalid_args')
        time.sleep(5)
        before=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'];started=time.monotonic();prior=before['completed_bytes'];states=[]
        for second in range(20):
            time.sleep(max(0,started+second+1-time.monotonic()))
            row=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'];states.append(row['state'])
            case.step('sample capped download second '+str(second+1),'dl_bps within ±25% of 65536; same downloading state and monotonic bytes').close(
                row['state']=='downloading' and 49152<=row.get('dl_bps',0)<=81920 and row['completed_bytes']>=prior,
                json.dumps(dict(dl_bps=row.get('dl_bps'),state=row['state'],completed_bytes=row['completed_bytes'],delta=row['completed_bytes']-prior,download_limit_bps=row.get('download_limit_bps'),effective=row.get('effective_download_limit_bps'))))
            prior=row['completed_bytes']
        elapsed=time.monotonic()-started;rate=(prior-before['completed_bytes'])/elapsed
        case.check(49152<=rate<=81920,'real byte delta independently stays within ±25% over 20 seconds after five-second warmup',json.dumps(dict(bytes=prior-before['completed_bytes'],seconds=elapsed,bps=rate,states=states)))
        c.call('torrents.set_download_limit',dict(hashes=[pair.hash],bps=131072))
        def measure_live(bps, label):
            low,high=bps*.75,bps*1.25
            assert pair.h.wait_for(lambda:((r:=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'])['state']=='downloading' and low<=r.get('dl_bps',0)<=high,json.dumps(r)),
                                   label+' reaches its measured rate after the live change',timeout=10)
            first=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'];begin=time.monotonic()
            previous=first['completed_bytes']
            for second in range(12):
                time.sleep(max(0,begin+second+1-time.monotonic()))
                r=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent'];states.append(r['state'])
                case.check(r['state']=='downloading' and r['completed_bytes']>=previous
                           and r.get('download_limit_bps')==131072 and r.get('effective_download_limit_bps')==bps,
                           label+' keeps the live handle and durable cap',json.dumps(r))
                previous=r['completed_bytes']
            elapsed=time.monotonic()-begin;rate=(previous-first['completed_bytes'])/elapsed
            case.check(low<=rate<=high,label+' enforces actual completed-byte rate within 25 percent over 12 seconds',
                       json.dumps(dict(bytes=previous-first['completed_bytes'],seconds=elapsed,measured_bps=rate,requested_bps=bps)))
        measure_live(131072,'mid-transfer 128 KiB cap replacement')
        c.call('transfer.set_alt',dict(on=True))
        measure_live(32768,'32 KiB turtle composition')
        c.call('transfer.set_alt',dict(on=False))
        observed=[p.get('state') for f in c.pushed for p in list(f.get('changed',{}).values())+f.get('items',[])+f.get('added',[]) if 'state' in p]
        case.check(len(c.pushed)<=512 and 'checking' not in observed+states and c.call('app.info')['result']['engine_epoch']==epoch,
                   'pushed state sequence and engine epoch confirm no recheck or engine restart',json.dumps(dict(epoch=epoch,states=states,pushed_states=observed,frames=len(c.pushed))))


def limit_verb(binary,systemd):
    from agent_contract import cli
    with d.Case('agent_contract','limit_verb') as case,fixtures.daemon_pair(case,binary,systemd,262144) as pair:
        for bad in ('0','1','-1','4294967296'):
            cli(case,pair.box,binary,'limit',pair.hash,'--down',bad,'--json',code=2)
        value,_=cli(case,pair.box,binary,'limit',pair.hash,'--down','64K','--json')
        case.check(value==dict(ok=True,action='limit',hashes=[pair.hash],download_limit_bps=65536),'CLI emits the specified cap envelope',json.dumps(value))
        c=pair.client
        row=c.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        ledger=Path(pair.box.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json'
        saved=json.loads(ledger.read_text())['records'][0]
        case.check(row.get('download_limit_bps')==saved.get('download_limit')==65536,'API and durable record agree with CLI cap',json.dumps(dict(torrent=row,record=saved)))
        pair.h.stop_serve(pair.p);pair.h.notify.close()
        p=pair.h.start_serve(idle_exit_secs=0);pair.h.receive_ready();pair.h.wait_engine()
        with d.ApiConn(pair.h) as fresh:
            fresh.hello()
            def restored():
                row=fresh.call('torrents.get',dict(hash=pair.hash)).get('result',{}).get('torrent',{})
                return row.get('download_limit_bps')==65536 and row.get('state')=='downloading',json.dumps(row)
            pair.h.wait_for(restored,'actual daemon restore keeps the persisted cap')
            fresh.call('torrents.set_download_limit',dict(hashes=[pair.hash]))
            row=fresh.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
            case.check(row.get('download_limit_bps')==65536,'omitted bps retains the existing cap',json.dumps(row))
            missing=fresh.call('torrents.set_download_limit',dict(hashes=['0'*40]),error='not_found')
            case.check(missing.get('error',{}).get('details',{}).get('hash')=='0'*40,'no-op cap request still validates the record identity',json.dumps(missing))
            value,_=cli(case,pair.box,binary,'limit','all','--down','none','--json')
            case.check(value.get('download_limit_bps') is None and value.get('hashes')==[pair.hash],'CLI all expansion explicitly clears the cap',json.dumps(value))
            row=fresh.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
            case.check('download_limit_bps' not in row and 'download_limit' not in json.loads(ledger.read_text())['records'][0],
                       'unlimited is omitted from API and durable record',json.dumps(row))
        pair.h.stop_serve(p)
