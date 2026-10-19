"""Compare real human CLI rows with the same confirmed rows in a narrow TUI."""
import json
import subprocess
from fbtlog import Case


def list_human_matches_tui_format(binary,systemd):
    from tui_smoke import owned_fixture,private_net
    import daemon as d
    private_net.enter()
    with Case('agent_contract','list_human_matches_tui_format') as case:
        with owned_fixture(binary,case) as (sandbox,h,ui,process,torrent):
            with d.ApiConn(h) as client:
                client.hello()
                client.call('torrents.stop',dict(hashes=[torrent.info_hash]))
                if not h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state']=='paused',json.dumps(row)),
                                  'pause the real payload before comparing stable progress'):
                    raise RuntimeError('format comparison did not reach stable paused state')
                rows=client.call('torrents.list',{})['result']['torrents']
                for row in rows:
                    client.call('torrents.rename',dict(hash=row['hash'],name='M3 '+row['state']+' row'))
                rows=client.call('torrents.list',{})['result']['torrents']
                result=subprocess.run([str(binary),'list'],env=dict(sandbox.env,NO_COLOR='1'),capture_output=True,text=True,timeout=10)
                case.check(result.returncode==0 and not result.stderr,'human list succeeds without errors',result.stdout+result.stderr)
                definitions={row['state']:row for row in json.loads((d.REPO/'contracts/ui/states.json').read_text())}
                ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','90','-y','24')
                for row in rows:
                    state=definitions[row['state']]
                    label=state['text_glyph']+' '+state['short']
                    progress='—' if row.get('progress') is None else f'{row["progress"]*100:.0f}%'
                    matching=[line for line in result.stdout.splitlines() if row['name'] in line]
                    case.check(len(matching)==1 and label in matching[0] and progress in matching[0],
                               'human list has the same worded state and known/unknown progress as the TUI',
                               json.dumps(dict(row=row,line=matching,label=label,progress=progress)))
                    ui.observe('compare '+row['state']+' row at 90 columns','each visible row has its own state word and truthful progress',
                               lambda pane,name=row['name'],label=label,progress=progress:any(name in line and label in line and progress in line for line in pane.splitlines()))
                ui.press('q','detach after comparing every row',lambda pane:ui.done.exists() and ui.done.read_text()=='0' and process.child.poll() is None)
