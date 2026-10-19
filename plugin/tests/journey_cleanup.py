"""W8: explicit single/filtered removal, bounded Undo and safe file deletion."""
import hashlib
import json
from dashboard import Session
import daemon
import fixtures
from journey_support import JourneySession
from fbtlog import Case,suite_status


def run(binary):
    with Case('qs_journeys','w8_cleanup') as case,daemon.Sandbox() as box,daemon.Harness(case,binary,box) as h:
        data=box.root/'watched';data.mkdir()
        prepared=[]
        for name in ('Watched one','Watched two','Unrelated film'):
            payload=data/(name+'.mkv');payload.write_bytes((name.encode()+b'\0')*16384)
            prepared.append((fixtures.torrent_fixture(box.root,[payload],name=name),payload,hashlib.sha256(payload.read_bytes()).hexdigest()))
        sentinel=data/'unrelated notes.txt';sentinel.write_text('Never remove files outside the torrent')
        process=h.start_serve();h.receive_ready();h.wait_engine();q=None
        try:
            with daemon.ApiConn(h) as c:
                c.hello()
                for torrent,_,_ in prepared:c.call('torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(data)))
                def rows():return c.call('torrents.list',{})['result']['torrents']
                def hashes():return {row['hash'] for row in rows()}
                wanted={t.info_hash for t,_,_ in prepared}
                assert h.wait_for(lambda:(len(rows())==3 and all(r['state']=='done' for r in rows()),str(rows())),'three watched private films verified')
                q=JourneySession(case,box)
                def view():return q.state().get('view') or {}
                def press(keys,predicate):
                    q.compositor.ipc('[title="^fbittorrent$"] focus')
                    try:return q.keys(keys,predicate)
                    except TimeoutError as error:raise TimeoutError('W8 keys '+repr(keys)+': '+str(error)) from error
                def intact():return all(p.exists() and hashlib.sha256(p.read_bytes()).hexdigest()==digest for _,p,digest in prepared) and sentinel.read_text()=='Never remove files outside the torrent'
                q.wait(lambda:q.ipc('ready')=='true');q.shell('summon',json.dumps(dict(view='torrents',hash=prepared[0][0].info_hash)))
                q.wait(lambda:view().get('cursor')==prepared[0][0].info_hash)
                press(['d'],lambda:(view().get('dialog') or {}).get('kind')=='remove')
                press(['-k','Return'],lambda:not view().get('dialog'))
                case.check(hashes()==wanted and intact(),'single Remove defaults to Cancel; records and bytes are unchanged',str(hashes()))
                press(['d'],lambda:(view().get('dialog') or {}).get('kind')=='remove')
                press(['-k','Right','-k','Return'],lambda:not view().get('dialog') and prepared[0][0].info_hash not in hashes())
                q.wait(lambda:view().get('cleanup',{}).get('undo'))
                case.check(intact(),'Remove keeps the watched film on disk and exposes a confirmed Undo','all file digests retained')
                press(['U'],lambda:hashes()==wanted)
                case.check(intact(),'Undo restores the single torrent without changing payloads','same hashes and bytes')
                press(['/Watched','-k','Return'],lambda:view().get('query')=='Watched' and view().get('focus')=='table')
                press(['-M','ctrl','a','-m','ctrl'],lambda:len(view().get('selection',[]))==2)
                press(['d'],lambda:(view().get('dialog') or {}).get('kind')=='remove')
                press(['-k','Right','-k','Return'],lambda:not view().get('dialog') and hashes()=={prepared[2][0].info_hash})
                q.wait(lambda:view().get('cleanup',{}).get('undo'))
                case.check(intact(),'filtered selection removes only the two matching records and keeps every file',str(hashes()))
                press(['U'],lambda:hashes()==wanted)
                case.check(intact(),'one Undo restores every record from the filtered removal','all three records returned')
                # Delete uses a separate, explicit dialog and remains cancel-first.
                q.shell('summon',json.dumps(dict(view='torrents',hash=prepared[0][0].info_hash)))
                press(['-k','Escape'],lambda:view().get('query')=='')
                press(['D'],lambda:(view().get('dialog') or {}).get('kind')=='delete')
                press(['-k','Return'],lambda:not view().get('dialog'))
                case.check(hashes()==wanted and intact(),'Delete files also defaults to Cancel','all records and digests unchanged')
                press(['D'],lambda:(view().get('dialog') or {}).get('kind')=='delete')
                press(['-k','Right','-k','Return'],lambda:not view().get('dialog') and prepared[0][0].info_hash not in hashes())
                q.wait(lambda:not prepared[0][1].exists())
                case.check(hashes()==wanted-{prepared[0][0].info_hash} and all(p.exists() for _,p,_ in prepared[1:]) and sentinel.exists(),'explicit Delete removes only that torrent and its owned files','other films and unrelated sentinel retained')
                q.shell('summon',json.dumps(dict(view='torrents',hash=prepared[1][0].info_hash)))
                press(['d'],lambda:(view().get('dialog') or {}).get('kind')=='remove')
                press(['-k','Right','-k','Return'],lambda:prepared[1][0].info_hash not in hashes())
                q.wait(lambda:view().get('cleanup',{}).get('undo'))
                q.wait(lambda:not view().get('cleanup',{}).get('undo'),timeout=12)
                press(['U'],lambda:True)
                case.check(prepared[1][0].info_hash not in hashes() and prepared[1][1].exists(),'Undo expires after the daemon window and cannot resurrect an expired record','files retained; no late restore')
                press(['q'],lambda:q.state().get('view') is None)
                case.check(process.child.poll() is None,'closing the cleanup window leaves the daemon running','same private owner')
        finally:
            try:
                if q:q.close()
            finally:
                if process.child.poll() is None:h.stop_serve(process)
    return suite_status('qs_journeys')

if __name__=='__main__':
    import private_net
    private_net.enter()
    import argparse
    from pathlib import Path
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,required=True)
    args=parser.parse_args();raise SystemExit(run(args.binary.resolve()))
