"""CLI display rename, durable across a real daemon restart without payload moves."""
import hashlib
import json

import fixtures
from daemon import ApiConn, Harness, Sandbox
from fbtlog import Case


def rename_verb(binary, systemd):
    from agent_contract import cli
    with Case('agent_contract', 'rename_verb') as case, Sandbox() as box, Harness(case, binary, box, systemd) as h:
        payload=box.downloads/'Original.bin';payload.write_bytes(b'owned rename payload'*4096)
        digest=hashlib.sha256(payload.read_bytes()).hexdigest()
        fixture=fixtures.torrent_fixture(box.root, [payload], name='original')
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            with ApiConn(h) as client:
                client.hello()
                client.call('torrents.add', dict(source=dict(path=str(fixture.torrent)), save_path=str(box.downloads)))
                assert h.wait_for(lambda: ((row:=client.call('torrents.get', dict(hash=fixture.info_hash))['result']['torrent'])['state']=='done', json.dumps(row)), 'owned payload completes before rename')
            renamed,_=cli(case,box,binary,'rename',fixture.info_hash,'  Personal café ✨  ','--json')
            case.check(renamed.get('action')=='rename' and renamed.get('hashes')==[fixture.info_hash], 'CLI accepts a personal display title for one hash', json.dumps(renamed))
            shown,_=cli(case,box,binary,'show',fixture.info_hash,'--json')
            case.check(shown.get('name')=='Personal café ✨', 'show uses the trimmed display title', json.dumps(shown))
            for invalid in ('bad\nname','x'*513):
                cli(case,box,binary,'rename',fixture.info_hash,invalid,'--json',code=2)
            h.stop_serve(process);h.lock_free();h.notify.close()
            process=h.start_serve();h.receive_ready();h.wait_engine()
            restored,_=cli(case,box,binary,'show',fixture.info_hash,'--json')
            case.check(restored.get('name')=='Personal café ✨', 'display title survives real owner restart', json.dumps(restored))
            cli(case,box,binary,'rename',fixture.info_hash,'','--json')
            reverted,_=cli(case,box,binary,'show',fixture.info_hash,'--json')
            case.check(reverted.get('name')==payload.name, 'empty rename restores the metadata title', json.dumps(reverted))
            case.check(hashlib.sha256(payload.read_bytes()).hexdigest()==digest and sorted(p.name for p in box.downloads.iterdir())==[payload.name], 'rename leaves file names and payload bytes intact', str(payload))
        finally:
            if process.child.poll() is None:h.stop_serve(process)
