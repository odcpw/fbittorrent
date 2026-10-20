"""Unlink payload data beneath a live transfer: owner state must become truthful."""
import json
from pathlib import Path
import daemon as d
import fixtures


def live_missing(binary,systemd,state):
    with d.Case('daemon','live_missing_'+state) as case,fixtures.daemon_pair(case,binary,systemd,65536,state) as pair:
        client=pair.client
        path=Path(pair.row['content_path'])
        assert path.is_relative_to(pair.box.downloads) and path.is_file() and not path.is_symlink()
        before=path.stat().st_size
        client.call('subscribe',dict(topics=['torrents','summary'],interval_ms=250))
        path.unlink()
        def missing():
            r=client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
            return r['state']=='error' and r.get('reason')=='missing_files',json.dumps(r)
        found=pair.h.wait_for(missing,'unlink under live '+state+' is detected without a restart or explicit recheck',timeout=10)
        case.check(found and not path.exists(),'live missing payload is not silently recreated or reported as healthy',json.dumps(dict(previous_state=state,previous_bytes=before,path_exists=path.exists())))
        if found:
            row=client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
            case.check(not row.get('completed_bytes') and not row.get('progress') and row.get('dl_bps',0)==0 and row.get('ul_bps',0)==0,
                       'missing single-file payload claims no completion or active rates',json.dumps(row))


def live_missing_downloading(binary,systemd):live_missing(binary,systemd,'downloading')
def live_missing_seeding(binary,systemd):live_missing(binary,systemd,'seeding')
CASES=(live_missing_downloading,live_missing_seeding)
