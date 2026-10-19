"""Two hundred genuine daemon records, one transferring from the private seed."""
import hashlib
import json
from pathlib import Path
import fixtures

HERE=Path(__file__).parent


def add_paused(client,sandbox):
    spec=json.loads((HERE/'fixtures/states/table-200.json').read_text())
    directory=sandbox.root/'table-metainfo';directory.mkdir()
    length=spec['size'];piece_length=16384
    pieces=hashlib.sha1(bytes(piece_length)).digest()*(length//piece_length)
    hashes=[]
    for index in range(spec['paused']):
        name=f'Table fixture {index:03d}.bin'
        info={'name':name,'piece length':piece_length,'private':1,'length':length,'pieces':pieces}
        path=directory/f'{index}.torrent';path.write_bytes(fixtures.bencode({'info':info}))
        reply=client.call('torrents.add',dict(source=dict(path=str(path)),save_path=str(sandbox.downloads),stopped=True))
        if not reply.get('ok'):raise RuntimeError(json.dumps(reply))
        hashes.append(reply['result']['hash'])
    rows=client.call('torrents.list',{})['result']['torrents']
    client.case.check(len(rows)==spec['count'] and sum(row['state']=='paused' for row in rows)==spec['paused'],
                      '200 rows come from real add operations; inactive rows stay paused',json.dumps(dict(total=len(rows),paused=sum(row['state']=='paused' for row in rows),hashes=hashes)))
    return rows
