"""Presentation states derived from the frozen wire objects; no live-engine claim."""
from copy import deepcopy
import json
from pathlib import Path
import tomllib

ROOT=Path(__file__).resolve().parents[2]


def write(config):
    frames=[json.loads(line)['frame'] for line in (ROOT/'contracts/ipc/v1/subscribe.ndjson').read_text().splitlines() if json.loads(line)['dir']=='s2c']
    summary=next(frame['data'] for frame in frames if frame.get('topic')=='summary' and frame['ev']=='state')
    torrent_frames=[json.loads(line)['frame'] for line in (ROOT/'contracts/ipc/v1/torrents.ndjson').read_text().splitlines() if json.loads(line)['dir']=='s2c']
    torrents=[]
    for frame in torrent_frames:
        result=frame.get('result',{})
        torrents.extend(result.get('torrents',[]))
        if 'torrent' in result:torrents.append(result['torrent'])
    error=next(row for row in torrents if row['state']=='error')
    playable=next(row for row in torrents if row.get('playable',{}).get('ready') and row['playable']['acked'] is False)
    profiles=[]
    def add(name, state, glyph, tone, text, *, counts=None, **values):
        item=dict(name=name,online=True,expectedState=state,expectedGlyph=glyph,expectedTone=tone,expectedText=text,summary=deepcopy(summary))
        item['summary']['counts']={key:0 for key in summary['counts']}
        item['summary']['counts'].update(counts or {})
        item['summary']['network'].update(state='ready')
        item['summary']['daemon'].update(state='running')
        item['summary']['daemon'].pop('idle_exit_in_s',None)
        item['summary'].update(dl_bps=0,ul_bps=0,alt=False,top=[])
        for key in ('dl_bps','ul_bps','alt','top','idle_exit_in_s'):
            if key in values:
                value=values.pop(key)
                if key=='idle_exit_in_s':item['summary']['daemon'][key]=value
                else:item['summary'][key]=value
        item.update(values);profiles.append(item)
    add('stopped','stopped','download','dim','not running',online=False)
    add('idle','idle','download','normal','Idle · 8 done',counts=dict(done=8,total=8))
    add('idle-countdown','idle','download','normal','stops in 4 min',idle_exit_in_s=240)
    add('seeding','seeding','upload','normal','↑ 120 KB/s · 1 uploading',counts=dict(seeding=1,total=1),ul_bps=120000)
    add('stopped-recently','stopped_recently','download','dim','stopped (idle)',online=False,stoppedRecently=True,byeReason='idle')
    add('downloading','downloading','download','normal','2 downloading · 1 queued',counts=dict(downloading=2,queued=1,total=3),dl_bps=4200000,ul_bps=32000)
    add('ready','ready','ready','normal','Ready to watch:',counts=dict(downloading=1,ready=1,total=1),caps=['watch'],rows=[playable],top=[playable],dl_bps=4200000)
    add('ready-no-cap','downloading','download','normal','1 downloading',counts=dict(downloading=1,ready=1,total=1),rows=[playable],top=[playable])
    acked=deepcopy(playable);acked['playable']['acked']=True
    add('ready-acked','downloading','download','normal','1 downloading',counts=dict(downloading=1,ready=1,total=1),caps=['watch'],rows=[acked],top=[acked])
    add('waiting-vpn','waiting_vpn','vpn','urgent','2 downloads waiting',counts=dict(waiting_vpn=2,total=2),backendState='parked')
    add('error','error','error','urgent',error['error'],counts=dict(error=1,total=1),rows=[error])
    hostile=deepcopy(error);hostile['error']='<b>Broken & waiting</b>'
    add('error-plain-text','error','error','urgent','Broken & waiting',counts=dict(error=1,total=1),rows=[hostile])
    add('stopped-by-user','stopped_recently','download','dim','fbittorrent stopped',online=False,stoppedRecently=True,byeReason='user')
    add('turtle','downloading','download','normal','turtle limits on',counts=dict(downloading=1,total=1),alt=True,dl_bps=4200000)
    add('reconnecting','reconnecting','checking','dim','Reconnecting',online=False,reconnecting=True)
    add('starting','starting','download','dim','Starting',online=False,startupRequested=True)
    (config/'Icons.json').write_text(json.dumps(profiles))
    palettes=[]
    for name,theme in [('dark','tokyo-night'),('light','catppuccin-latte')]:
        colors=tomllib.loads(Path('/usr/share/omarchy/themes',theme,'colors.toml').read_text())
        palettes.append(dict(name=name,**{key:colors[key] for key in ('foreground','background','red')}))
    (config/'Palettes.json').write_text(json.dumps(palettes))
