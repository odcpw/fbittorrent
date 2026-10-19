"""Staged records are visible metainfo, never daemon lifetime ownership."""
import json
from pathlib import Path
import daemon as d
from daemon_bus import RevisionStream
from daemon_policy import owned, hello, idle_exit


def staged_not_busy(binary, systemd):
    with owned('staged_not_busy', binary, systemd, seconds=3) as (case, box, h, process), d.ApiConn(h) as client:
        hello(client)
        fixture=d.recheck_local_fixture(box,name='staged-idle.bin')
        folder=fixture.files[0].parent
        before=d.staged_files_snapshot(folder)
        client.call('torrents.add',dict(source=dict(path=str(fixture.torrent)),stage=True,save_path=str(folder)))
        magnet_hash='cd'*20
        client.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+magnet_hash+'&dn=Idle%20metadata&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce'),stage=True,save_path=str(folder)))
        # The unresolved lookup remains inside its real 90-second budget while
        # the three-second idle policy closes this owned daemon.
        row=client.call('torrents.get',dict(hash=magnet_hash))['result']['torrent']
        case.check(row['staged'] and row['state']=='paused' and row.get('reason')=='user',
                   'unanswered staged magnet is still resolving before the idle countdown',json.dumps(row))
        sub=client.call('subscribe',dict(topics=['summary'],interval_ms=250))['result']['sub']
        stream=RevisionStream(client)
        frames=stream.snapshot(sub,['summary'])
        summary=next(frame['data'] for frame in frames if frame.get('topic')=='summary')
        counts=summary['counts'];policy=summary['daemon']
        case.check(all(value==(2 if key=='staged' else 0) for key,value in counts.items()),
                   'both staged records appear only in Summary.counts.staged',json.dumps(counts))
        case.check(policy['busy_reasons']==[] and 0<policy.get('idle_exit_in_s',0)<=3,
                   'staged rows and their live metadata resolver do not cancel idle exit',json.dumps(policy))
        case.check(summary['top']==[] and summary['disk']['needed_bytes']==0,
                   'uncommitted adds occupy neither the Summary top rows nor download disk demand',json.dumps(summary))
        ledger_path=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json'
        ledger=json.loads(ledger_path.read_text())
        case.check(len(ledger['records'])==2 and all(r['staged'] for r in ledger['records']),
                   'ledger retains both staged records while daemon is alive',json.dumps(ledger))
        elapsed=idle_exit(case,h,process,client,timeout=7)
        after=d.staged_files_snapshot(folder)
        case.check(before==after and elapsed<7,
                   'idle exit cancels metadata work, exits zero and preserves every existing payload byte',
                   json.dumps(dict(sha256_before=before,sha256_after=after,idle_wait_s=elapsed)))
