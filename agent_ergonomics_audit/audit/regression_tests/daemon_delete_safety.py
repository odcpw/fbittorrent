"""Deletion must be confined to declared files, live or dormant, even after path substitution."""
import hashlib
import json
from pathlib import Path
import daemon as d
import fixtures


def bundle(box,nested=False):
    base=box.downloads/'parent' if nested else box.downloads
    root=base/'owned-torrent';(root/'nested').mkdir(parents=True)
    payloads={'nested/payload.bin':b'a'*32768,'leaf.bin':b'b'*16384}
    for relative,data in payloads.items():(root/relative).write_bytes(data)
    stream=b''.join(payloads.values())
    info={'name':'Owned collection','private':1,'piece length':16384,
          'pieces':b''.join(hashlib.sha1(stream[i:i+16384]).digest() for i in range(0,len(stream),16384)),
          'files':[dict(length=len(data),path=relative.split('/')) for relative,data in payloads.items()]}
    torrent=box.root/'owned.torrent';torrent.write_bytes(fixtures.bencode(dict(info=info)))
    return root,torrent,hashlib.sha1(fixtures.bencode(info)).hexdigest(),payloads


def run(name,binary,systemd,dormant):
    with d.Case('daemon',name) as case:
        for substitution in ('ordinary','leaf','parent','root','ancestor'):
            with d.Sandbox() as box,d.Harness(case,binary,box,systemd) as h:
                root,torrent,hash_,payloads=bundle(box,substitution=='ancestor')
                unrelated=root/'my-unrelated-notes.txt';unrelated.write_bytes(b'owner file: preserve exactly')
                p=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
                with d.ApiConn(h) as client:
                    client.hello();client.call('torrents.add',dict(source=dict(path=str(torrent)),save_path=str(root),seed=not dormant))
                    expected='done' if dormant else 'seeding'
                    assert h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=hash_))['result']['torrent'])['state']==expected and row['checked'],json.dumps(row)), 'real bundle reaches '+expected)
                if dormant:
                    h.stop_serve(p);h.notify.close();p=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
                with d.ApiConn(h) as client:
                    client.hello();row=client.call('torrents.get',dict(hash=hash_))['result']['torrent']
                    case.check(row['state']==expected and row['checked'] is (not dormant),
                        'delete control uses an actual '+('dormant restored' if dormant else 'live seeding')+' torrent',json.dumps(row))
                    outside=box.root/'outside'
                    outside_payload=outside/root.name if substitution=='ancestor' else outside
                    (outside_payload/'nested').mkdir(parents=True)
                    for relative in payloads:(outside_payload/relative).write_bytes(b'outside: never delete')
                    outside_files={path:path.read_bytes() for path in outside.rglob('*') if path.is_file()}
                    if substitution=='leaf':
                        (root/'leaf.bin').unlink();(root/'leaf.bin').symlink_to(outside/'leaf.bin')
                    elif substitution=='parent':
                        (root/'nested').rename(root/'original-nested');(root/'nested').symlink_to(outside/'nested',target_is_directory=True)
                    elif substitution=='root':
                        retained=box.downloads/'original-root';root.rename(retained);unrelated=retained/unrelated.name
                        root.symlink_to(outside,target_is_directory=True)
                    elif substitution=='ancestor':
                        retained=box.downloads/'original-parent';root.parent.rename(retained)
                        unrelated=retained/root.name/unrelated.name
                        root.parent.symlink_to(outside,target_is_directory=True)
                    step=case.step('delete-with-files '+substitution,'only declared in-root files may be removed; outside and unrelated bytes survive')
                    reply=client.response(client.send('torrents.remove',dict(hashes=[hash_],delete_files=True)))
                    if reply.get('result',{}).get('state')=='pending':
                        reference=reply['result']['ref']
                        reply=next(frame for frame in client.events() if frame.get('ev')=='mutation.result' and frame.get('ref')==reference)
                        succeeded=reply.get('outcome')=='done'
                    else:succeeded=reply.get('ok') is True
                    step.close(all(path.is_file() and path.read_bytes()==data for path,data in outside_files.items())
                               and unrelated.is_file() and unrelated.read_bytes()==b'owner file: preserve exactly',json.dumps(dict(substitution=substitution,reply=reply,outside=[str(p) for p in outside_files])))
                    if substitution in ('ordinary','leaf'):
                        case.check(succeeded and not (root/'nested/payload.bin').exists()
                                   and not (root/'leaf.bin').is_symlink() and not (root/'leaf.bin').exists(),
                                   'successful deletion removes declared files and the leaf link itself',json.dumps(reply))
                    else:
                        case.check(not succeeded,
                                   'unsafe substituted directory is refused instead of deleting through it',json.dumps(reply))
                        retained=client.call('torrents.get',dict(hash=hash_))
                        case.check(retained.get('ok') is True,'refused unsafe deletion preserves the torrent record for recovery',json.dumps(retained))
                h.stop_serve(p);h.lock_free()


def delete_files_safety_live(binary,systemd):run('delete_files_safety_live',binary,systemd,False)
def delete_files_safety_dormant(binary,systemd):run('delete_files_safety_dormant',binary,systemd,True)
CASES=(delete_files_safety_live,delete_files_safety_dormant)
