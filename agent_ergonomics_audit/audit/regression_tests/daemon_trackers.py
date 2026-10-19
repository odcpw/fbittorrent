"""P7-04: real announces, ordered private tracker selection and durable edits."""
import base64
import json
import time
from pathlib import Path
import daemon as d
import fixtures


def tracker_add_remove(binary,systemd):
    with d.Case('daemon','tracker_add_remove') as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        response=dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=b''))).decode())
        with fixtures.HttpFixture(box.root,{'/a':response,'/b':response}) as http:
            payload=box.root/'tracker-edit.bin';payload.write_bytes(b'tracker fixture'*4096)
            fixture=fixtures.torrent_fixture(box.root,[payload],tracker=http.base+'/a')
            hash=fixture.info_hash;a=http.base+'/a';b=http.base+'/b'
            p=h.start_serve(idle_exit_secs=0)
            if not(h.receive_ready() and h.wait_engine()):return
            def count(path):return sum(v.startswith(path+'?') for v in http.requests)
            with d.ApiConn(h) as c:
                c.hello();c.call('torrents.add',dict(source=dict(path=str(fixture.torrent))))
                if not h.wait_for(lambda:(count('/a')>=2,json.dumps(dict(a=count('/a'),b=count('/b')))),'original private tracker receives two real one-second announces'):return
                for invalid in ('file:///tmp/tracker','ftp://127.0.0.1/announce','http:///','udp://127.0.0.1/announce','x'*4097):
                    failure=c.call('torrents.add_tracker',dict(hash=hash,url=invalid),error='invalid_args')
                    case.check(failure.get('error',{}).get('details',{}).get('key')=='url','invalid tracker reports url without echoing it',json.dumps(failure))
                failure=c.call('torrents.add_tracker',dict(hash='0'*40,url=b),error='not_found')
                case.check(failure.get('error',{}).get('details',{}).get('hash')=='0'*40,'unknown record identifies the hash',json.dumps(failure))
                epoch=c.call('app.info')['result']['engine_epoch']
                for _ in range(2):c.call('torrents.add_tracker',dict(hash=hash,url=b))
                row=c.call('torrents.get',dict(hash=hash))['result']['torrent']
                case.check(row.get('tracker_count')==2 and count('/b')==0,'private torrent retains only its first tracker until that tracker is removed',json.dumps(dict(torrent=row,b=count('/b'))))
                ledger=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json'
                kept=ledger.with_suffix('.kept');ledger.rename(kept);ledger.mkdir()
                try:
                    c.call('torrents.remove_tracker',dict(hash=hash,url=a),error='internal')
                    unchanged=c.call('torrents.get',dict(hash=hash))['result']['torrent']
                    case.check(unchanged.get('tracker_count')==2 and json.loads(kept.read_text())['records'][0].get('trackers')==[b],
                               'failed atomic ledger save leaves the live tracker set and prior record unchanged',json.dumps(unchanged))
                finally:
                    ledger.rmdir();kept.rename(ledger)
                for _ in range(2):
                    value=h.cli('api','torrents.remove_tracker','--json-args',json.dumps(dict(hash=hash,url=a)))
                    case.check(value.get('ok') is True and value.get('result')==dict(state='done'),'raw CLI removal returns the exact done result',json.dumps(value))
                if not h.wait_for(lambda:(count('/b')>=2,json.dumps(dict(a=count('/a'),b=count('/b')))),'added tracker receives real announces after private first-tracker removal'):return
                old=count('/a');new=count('/b')
                h.wait_for(lambda:(count('/b')>=new+2,json.dumps(dict(a=count('/a'),b=count('/b')))),'two subsequent real announce cycles complete',timeout=8)
                case.check(count('/a')==old and count('/b')>=new+2,'removed tracker receives none over two further announce cycles while the added tracker continues',json.dumps(dict(a_before=old,a_after=count('/a'),b_before=new,b_after=count('/b'))))
                row=c.call('torrents.get',dict(hash=hash))['result']['torrent']
                case.check(row.get('tracker_count')==1 and c.call('app.info')['result']['engine_epoch']==epoch,'edits keep one admitted tracker without restarting the engine',json.dumps(row))
                ledger=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json';saved=json.loads(ledger.read_text())['records'][0]
                case.check(saved.get('trackers')==[b] and saved.get('removed_trackers')==[a],'record durably merges additions and excludes removed metainfo trackers',json.dumps(saved))
            h.stop_serve(p);h.notify.close();old=count('/a');new=count('/b')
            p=h.start_serve(idle_exit_secs=0)
            if not(h.receive_ready() and h.wait_engine()):return
            with d.ApiConn(h) as c:
                c.hello()
                h.wait_for(lambda:(count('/b')>=new+2,json.dumps(dict(a=count('/a'),b=count('/b')))),'actual daemon restore announces to the added tracker')
                case.check(count('/a')==old,'removed metainfo tracker stays removed on restore',json.dumps(dict(before=old,after=count('/a'))))
                # Exercise an empty live set and subsequent wake; no timer polling.
                c.call('torrents.remove_tracker',dict(hash=hash,url=b));time.sleep(.2);old_b=count('/b')
                value=h.cli('api','torrents.add_tracker','--json-args',json.dumps(dict(hash=hash,url=a)))
                case.check(value.get('ok') is True and value.get('result')==dict(state='done'),'raw CLI can re-add a previously removed tracker',json.dumps(value))
                h.wait_for(lambda:(count('/a')>=old+2,json.dumps(dict(a=count('/a'),b=count('/b')))),'empty live tracker stream wakes when a tracker is re-added')
                case.check(count('/b')==old_b,'last removed tracker stays silent after the next cycle',json.dumps(dict(before=old_b,after=count('/b'))))
                c.call('torrents.stop',dict(hashes=[hash]))
                c.call('torrents.add_tracker',dict(hash=hash,url=b))
                c.call('torrents.remove_tracker',dict(hash=hash,url=b))
                case.check(c.call('torrents.get',dict(hash=hash))['result']['torrent'].get('state')=='paused','tracker configuration also changes on an admitted paused handle','paused')
            h.stop_serve(p)
            artifact=case.path.with_suffix('.tracker-requests.json');artifact.write_text(json.dumps(http.requests,indent=2)+'\n')
            case.step('retain actual local tracker requests','loopback HTTP announce requests remain reviewable').ok(
                json.dumps(dict(a=count('/a'),b=count('/b'))),[artifact])
