"""Each operation gets its own completed, restarted, still-dormant record."""
import json
import daemon as d
import fixtures


def dormant_operations(binary, systemd):
    names=('limit','upload_cap','move','rename','upload','recheck','open','trackers')
    with d.Case('daemon','dormant_operations') as case,d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
        fixtures.path_shim('xdg-open','exit 0',box.root)
        items={}
        p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        with d.ApiConn(h) as c:
            c.hello()
            for i,key in enumerate(names):
                path=box.downloads/('dormant-'+key+'.bin');path.write_bytes(bytes([65+i])*32768)
                item=fixtures.torrent_fixture(box.root,[path],name=key)
                items[key]=item
                c.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads)))
            def complete():
                rows=c.call('torrents.list',{})['result']['torrents']
                return len(rows)==len(names) and all(r['state']=='done' and r['checked'] and r.get('completed_at',0)>0 for r in rows),json.dumps(rows)
            assert h.wait_for(complete,'all eight real payloads pass checks and save completion')
            dates={r['hash']:r['completed_at'] for r in c.call('torrents.list',{})['result']['torrents']}
        h.stop_serve(p);h.notify.close()
        p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        with d.ApiConn(h) as c:
            c.hello()
            def row(key):return c.call('torrents.get',dict(hash=items[key].info_hash))['result']['torrent']
            def dormant(key):
                value=row(key)
                case.check(value['state']=='done' and value['checked'] is False,'operation '+key+' starts with its own dormant completed record',json.dumps(value))
                return items[key].info_hash
            hash_=dormant('limit');reply=h.cli('limit',hash_,'--down','128K');value=row('limit')
            case.check(reply.get('ok') is True and value.get('download_limit_bps')==131072 and value['checked'] is False,
                       'CLI download cap edits the dormant record without admission',json.dumps(dict(reply=reply,row=value)))
            hash_=dormant('upload_cap');c.call('torrents.set_upload_limit',dict(hashes=[hash_],bps=65536));value=row('upload_cap')
            case.check(value.get('upload_limit_bps')==65536 and value['upload_enabled'] is False and value['checked'] is False,
                       'API upload cap edits dormant intent without enabling upload',json.dumps(value))
            hash_=dormant('rename');h.cli('rename',hash_,'Private durable display name')
            case.check(row('rename')['name']=='Private durable display name' and items['rename'].files[0].exists(),
                       'CLI rename changes the dormant display name while preserving its file',json.dumps(row('rename')))
            hash_=dormant('open');target=c.call('torrents.open_target',dict(hash=hash_))['result']['path'];h.cli('open',hash_)
            case.check(target==str(items['open'].files[0]) and row('open')['checked'] is False,
                       'open resolves the actual dormant file without a check',json.dumps(dict(path=target,row=row('open'))))
            hash_=dormant('trackers');url='http://127.0.0.1:1/dormant-fixture'
            c.call('torrents.trackers',dict(hash=hash_));c.call('torrents.add_tracker',dict(hash=hash_,url=url))
            trackers=c.call('torrents.trackers',dict(hash=hash_))['result']['trackers']
            case.check(any(t['url'].split('#')[0]==url for t in trackers),'tracker add is visible on the dormant record',json.dumps(trackers))
            c.call('torrents.remove_tracker',dict(hash=hash_,url=url))
            trackers=c.call('torrents.trackers',dict(hash=hash_))['result']['trackers']
            case.check(not trackers and row('trackers')['checked'] is False,'tracker remove retains dormancy',json.dumps(trackers))
            hash_=dormant('upload');h.cli('seed',hash_)
            assert h.wait_for(lambda:((r:=row('upload'))['state']=='seeding' and r['checked'] and r['upload_enabled'],json.dumps(r)),
                              'explicit torrent upload activates its own dormant data')
            h.cli('noseed',hash_)
            assert h.wait_for(lambda:((r:=row('upload'))['state']=='done' and r['upload_enabled'] is False,json.dumps(r)),
                              'turning upload off returns this torrent to done')
            hash_=dormant('recheck');event,_=d.recheck_reply(c,hash_,'dormant-operations-recheck')
            case.check(event['result']['have_bytes'][hash_]==32768,'recheck measures the complete dormant payload',json.dumps(event))
            hash_=dormant('move');destination=box.root/'moved';before=row('move')
            reply=h.cli('move',hash_,str(destination),timeout=20);after=row('move')
            moved=destination/items['move'].files[0].name
            case.check(reply.get('ok') is True and moved.is_file() and moved.read_bytes()==b'C'*32768 and not items['move'].files[0].exists()
                       and after['save_path']==str(destination) and after['state']=='done' and after['completed_at']==before['completed_at'],
                       'CLI move initializes dormant storage on demand and commits exact data and history',json.dumps(dict(reply=reply,before=before,after=after)))
            case.check(all(items[key].files[0].read_bytes()==bytes([65+names.index(key)])*32768 for key in names if key!='move'),
                       'every unrelated torrent payload remains unchanged in the shared source directory',str(box.downloads))
        h.stop_serve(p);h.notify.close()
        p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
        with d.ApiConn(h) as c:
            c.hello();rows=c.call('torrents.list',{})['result']['torrents'];by_hash={r['hash']:r for r in rows}
            case.check(all(r['state']=='done' and not r['checked'] and (r['hash']==items['upload'].info_hash or r['completed_at']==dates[r['hash']]) for r in rows),
                       'completion history survives another restart with all records dormant again',json.dumps(rows))
            case.check(by_hash[items['limit'].info_hash].get('download_limit_bps')==131072
                       and by_hash[items['upload_cap'].info_hash].get('upload_limit_bps')==65536
                       and by_hash[items['rename'].info_hash]['name']=='Private durable display name'
                       and by_hash[items['move'].info_hash]['save_path']==str(destination),
                       'download/upload caps, display name and moved destination survive another daemon restart',json.dumps(rows))
        h.stop_serve(p);h.lock_free()
