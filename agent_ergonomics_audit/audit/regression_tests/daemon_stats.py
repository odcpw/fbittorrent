"""P6-04: actual bytes and completion totals survive a daemon restart."""
import base64
import json
import os
import socket
import subprocess
import tomllib
from pathlib import Path
import daemon as d
import fixtures


def status_totals(case,box,binary,expected,session=True):
    """Check actual JSON and human status, including a nonzero upload source."""
    from agent_contract import cli
    value,_=cli(case,box,binary,'status','--json')
    case.check(value.get('all_time')==expected['all_time'] and
               (value.get('session')==expected['session'] if session else 'session' not in value),
               'status JSON reports the same real session and durable totals; closed status invents no session',json.dumps(dict(status=value,expected=expected)))
    step=case.step('read plain CLI status','exit 0 and both byte directions match the measured counters')
    child=case.popen([str(binary),'status'],env=box.env,cwd=box.root,stdout=subprocess.PIPE,text=True)
    output,_=child.communicate(timeout=10)
    pairs=[('all_time','All time')]+([('session','Session')] if session else [])
    wanted=[f"{label}: downloaded {expected[key]['downloaded']} B; uploaded {expected[key]['uploaded']} B" for key,label in pairs]
    step.close(child.returncode==0 and all(line in output for line in wanted) and (session or 'Session:' not in output),output)
    return value


def stats_persist(binary,systemd):
    from agent_contract import cli
    with d.Case('daemon','stats_persist') as case,fixtures.daemon_pair(case,binary,systemd,1048576) as pair:
        def finished(client,hash_,completed=1):
            row=client.call('torrents.get',dict(hash=hash_))['result']['torrent']
            stats=client.call('app.stats')['result']
            return row['state']=='done' and stats['all_time']['completed']>=completed,json.dumps(dict(torrent=row,stats=stats))
        assert pair.h.wait_for(lambda:finished(pair.client,pair.hash),'first actual transfer completes and increments statistics',timeout=20)
        before=pair.client.call('app.stats',dict(history=True))['result']
        size=pair.payload.stat().st_size
        seed=pair.seed_client.call('app.stats')['result']
        case.check(before['session']['downloaded']>=size and before['all_time']['downloaded']==before['session']['downloaded'] and seed['all_time']['uploaded']>=size and seed['all_time']['downloaded']==0,
                   'real received bytes and independently observed seed uploads are counted, not pre-existing verified bytes',json.dumps(dict(before=before,seed=seed,size=size)))
        status_totals(case,pair.box,binary,before)
        status_totals(case,pair.owned[1][0],binary,seed)
        sub=pair.client.call('subscribe',dict(topics=['stats']))['result']['sub']
        pushed=next(pair.client.events())
        case.check(pushed.get('topic')=='stats' and pushed.get('data',{}).get('all_time')==before['all_time'] and pushed['data']['session']==before['session'],
                   'pushed stats expose the same real session and all-time totals',json.dumps(pushed))
        pair.client.call('unsubscribe',dict(sub=sub))
        # Restart just the engine first: bytes and the daemon's session identity stay.
        epoch=pair.client.call('app.info')['result']['engine_epoch']
        pair.client.call('app.restart',dict(scope='engine'))
        assert pair.h.wait_for(lambda:((info:=pair.client.call('app.info')['result'])['state']=='running' and info['engine_epoch']>epoch,json.dumps(info)),'new engine epoch is running')
        engine=pair.client.call('app.stats')['result']
        case.check(engine['session']==before['session'] and engine['all_time']==before['all_time'],
                   'an engine restart neither resets the session nor recounts verified restored data',json.dumps(dict(before=before,after=engine)))
        status_totals(case,pair.box,binary,engine)
        pair.h.stop_serve(pair.p);pair.h.notify.close()
        path=Path(pair.box.env['XDG_STATE_HOME'])/'fbittorrent/stats.json'
        saved=json.loads(path.read_text())
        case.check(set(saved)=={'downloaded','uploaded','completed'} and saved==before['all_time'] and path.stat().st_mode&0o777==0o600,
                   'drain persists exactly the three totals through a private 0600 store',json.dumps(dict(path=str(path),saved=saved,mode=oct(path.stat().st_mode&0o777))))
        stopped=status_totals(case,pair.box,binary,dict(all_time=saved),session=False)
        case.check(stopped.get('all_time')==saved and stopped.get('running') is False,'stopped status reports durable all_time without starting a daemon',json.dumps(stopped))
        replacement=pair.h.start_serve(idle_exit_secs=0);pair.h.receive_ready();pair.h.wait_engine()
        with d.ApiConn(pair.h) as fresh:
            fresh.hello();after=fresh.call('app.stats')['result']
            case.check(after['all_time']==before['all_time'] and after['session']['downloaded']==after['session']['uploaded']==0 and after['session']['started_at']>before['session']['started_at'],
                       'daemon restart preserves all_time and starts a fresh session with no verified-byte recount',json.dumps(dict(before=before,after=after)))
            status_totals(case,pair.box,binary,after)
            seedbox=pair.owned[1][0];cfg=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())
            body=fixtures.bencode(dict(interval=1,peers=socket.inet_aton('127.0.0.1')+int(cfg['peer_port']).to_bytes(2,'big')))
            root=seedbox.root/'second-session';root.mkdir()
            with fixtures.HttpFixture(root,{'/announce':dict(base64=base64.b64encode(body).decode())}) as http:
                payload=seedbox.downloads/'second-session.bin';payload.write_bytes(os.urandom(524288))
                item=fixtures.torrent_fixture(root,[payload],tracker=http.base+'/announce')
                pair.seed_client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(seedbox.downloads),seed=True))
                assert pair.seed.wait_for(lambda:((row:=pair.seed_client.call('torrents.get',dict(hash=item.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),'second actual seed verifies')
                fresh.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(pair.box.downloads)))
                assert pair.h.wait_for(lambda:finished(fresh,item.info_hash,saved['completed']+1),'second session completes an actual new transfer',timeout=15)
                final=fresh.call('app.stats')['result']
                status=status_totals(case,pair.box,binary,final)
                case.check(final['session']['downloaded']>=524288 and final['all_time']['downloaded']>=saved['downloaded']+524288 and final['all_time']['completed']==saved['completed']+1 and status.get('all_time')==final['all_time'],
                           'second-session bytes and one new completion add to all_time; status agrees',json.dumps(dict(before=before,after_restart=after,after_second_session=final,status=status)))
            pair.h.stop_serve(replacement)
            case.check(json.loads(path.read_text())==final['all_time'],'final drain saves the second session exactly',path.read_text())
