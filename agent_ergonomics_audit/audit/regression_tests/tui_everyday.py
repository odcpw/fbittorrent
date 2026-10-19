#!/usr/bin/env python3
"""S05: actual everyday TUI keys against one private daemon, payload and services."""
import argparse
import hashlib
import json
import shlex
import sys
import time
import tomllib
from pathlib import Path
import daemon as d
import fbtlog
import fixtures
import native

IMDB='tt1234001'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary',type=Path)
    parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();binary=args.binary.resolve()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    with fbtlog.Case('tui_everyday','everyday_live') as case,d.Sandbox() as box:
        case.check(bool(d.private_environment(box)),'prove private XDG and download paths before any product starts',str(box.root))
        data=Path(box.env['XDG_DATA_HOME'])/'fbittorrent';data.mkdir(parents=True,exist_ok=True)
        # Fresh persisted Shows avoid both external requests and synthetic API responses.
        episodes=[dict(key=f'S01E{i:02}',season=1,episode=i,title=f'Owned.Show.S01E{i:02}.1080p',magnet='magnet:?xt=urn:btih:'+f'{i+10:040x}',quality='1080p',size=size,seeds=3-i,released=int(time.time())-i,candidates=1) for i,size in [(1,900),(2,1100)]]
        (data/'shows.json').write_text(json.dumps(dict(shows=[dict(imdb=IMDB,title='Owned Show',quality=None,seen=[],grabbed={},last_checked=int(time.time()),catalogue_complete=True,episodes=episodes)])))
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True,exist_ok=True)
        (cache/(IMDB+'.json')).write_text(json.dumps(dict(id=IMDB,name='Owned Show',kind='tvSeries',year=None,stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')))
        engines=data/'nova3/engines';engines.mkdir(parents=True)
        (engines/'everydayfixture.py').write_text('class everydayfixture:\n name="Owned everyday provider"\n url="https://fixture.invalid"\n def search(self,query):\n  from novaprinter import prettyPrinter\n  for i,size in [(1,900),(2,1100)]:prettyPrinter(dict(link="magnet:?xt=urn:btih:"+str(i)*40,name="Owned.Result."+str(i),size=size,seeds=3-i,leech=-1,engine_url=self.url))\n')
        # Never let hovered names trigger a public lookup during this local journey.
        names=cache.parent/'by-name-v2';names.mkdir(exist_ok=True)
        for name in ['everyday_bin','owned_result_1','owned_result_2','owned_show']:
            (names/(name+'.json')).write_text(json.dumps(dict(id=IMDB)) if name=='owned_show' else '{"missing":true}')
        box.env['FBT_IMDB_BASE_URL']='http://127.0.0.1:9';box.env['FBT_EZTV_BASE_URL']='http://127.0.0.1:9'
        box.keys+=['FBT_IMDB_BASE_URL','FBT_EZTV_BASE_URL']
        copied=box.root/'copied.txt'
        fixtures.path_shim('wl-copy','cat > '+shlex.quote(str(copied)),box.root)
        payload=box.downloads/'everyday.bin';payload.write_bytes(b'owned payload '*16384)
        item=fixtures.torrent_fixture(box.root,[payload])
        def fingerprint():return (payload.stat().st_ino,payload.stat().st_size,hashlib.sha256(payload.read_bytes()).hexdigest())
        before=fingerprint()
        with d.Harness(case,binary,box,False) as h:
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as c:
                c.hello();case.check(len(c.call('shows.list')['result']['shows'])==1,'persisted Shows fixture loads before UI connects','one real stored show');c.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(box.downloads),stopped=True))
                ui=native.AttachedTui(case,str(binary),box,h,'fbt-fbt-p4-07-swt')
                try:
                    target,done=ui.start();assert ui.assert_pane(target,'everyday.bin')
                    ui.tmux('resize-window','-t',target,'-x','80','-y','24')
                    assert ui.assert_pane(target,'Idle · UI open')
                    assert ui.assert_pane(target,'3 Shows (2)')
                    ui.keys(target,'z')
                    on=ui.assert_pane(target,'󰢃 Turtle',timeout=1)
                    assert on
                    ui.keys(target,'z')
                    assert h.wait_for(lambda:('󰢃 Turtle' not in ui.capture(target).splitlines()[0],ui.capture(target)),'existing turtle indicator clears by push within one second',timeout=1)
                    ui.keys(target,'s');assert ui.assert_pane(target,'Sorted by name')
                    ui.keys(target,'S');assert ui.assert_pane(target,'Sorted by name ↓')
                    ui.keys(target,'y')
                    assert h.wait_for(lambda:(copied.exists() and item.info_hash in copied.read_text(),copied.read_text() if copied.exists() else 'not written'),'torrent y copies its magnet to owned clipboard shim')
                    ui.keys(target,'d');assert ui.assert_pane(target,'keep files')
                    ui.keys(target,'Enter',literal=False)
                    case.check(len(c.call('torrents.list',{})['result']['torrents'])==1,'Enter on destructive confirmation keeps the safe No default','one torrent still present')
                    ui.keys(target,'d');ui.keys(target,'y')
                    assert h.wait_for(lambda:((rows:=c.call('torrents.list',{})['result']['torrents'])==[],json.dumps(rows)),'confirmed removal disappears from daemon')
                    assert ui.assert_pane(target,'(U) Undo removal')
                    ui.keys(target,'U')
                    assert h.wait_for(lambda:(len(rows:=c.call('torrents.list',{})['result']['torrents'])==1 and rows[0]['hash']==item.info_hash,json.dumps(rows)),'U restores the exact removed hash')
                    case.check(fingerprint()==before,'remove and undo preserve actual payload inode length and SHA256',repr(dict(before=before,after=fingerprint())))
                    ui.keys(target,'2');ui.keys(target,'C-a',literal=False);ui.keys(target,'everyday');ui.keys(target,'Enter',literal=False)
                    assert ui.assert_pane(target,'Owned.Result.1')
                    ui.keys(target,'Escape',literal=False);ui.keys(target,'g')
                    ui.keys(target,'y')
                    assert h.wait_for(lambda:('1'*40 in copied.read_text(),copied.read_text()),'Search y copies the selected provider magnet')
                    # Find Size through the available fields; their cycle order
                    # is not the behavior under test.
                    for _ in range(7):
                        ui.keys(target,'s')
                        if 'SIZE↓' in ui.capture(target):break
                    assert ui.assert_pane(target,'SIZE↓')
                    pane=ui.capture(target);case.check(pane.find('Owned.Result.2')<pane.find('Owned.Result.1'),'Search size sort uses actual numeric sizes',pane)
                    ui.keys(target,'S');assert ui.assert_pane(target,'SIZE↑')
                    pane=ui.capture(target);case.check(pane.find('Owned.Result.1')<pane.find('Owned.Result.2'),'Search reverse changes actual row order',pane)
                    ui.keys(target,'3');assert ui.assert_pane(target,'S01E01')
                    ui.keys(target,'s');ui.keys(target,'s');assert ui.assert_pane(target,'Sorted releases by size ↓')
                    pane=ui.capture(target);case.check(pane.find('S01E02')<pane.find('S01E01'),'release sort compares actual sizes in the narrow list',pane)
                    ui.keys(target,'S');assert ui.assert_pane(target,'Sorted releases by size ↑')
                    pane=ui.capture(target);case.check(pane.find('S01E01')<pane.find('S01E02'),'release reverse changes actual row order',pane)
                    ui.keys(target,'y')
                    assert h.wait_for(lambda:(copied.read_text()==episodes[0]['magnet'],copied.read_text()),'Shows y copies the selected release magnet')
                    ui.keys(target,'=');assert ui.assert_pane(target,'Show quality')
                    ui.keys(target,'C-a',literal=False);ui.keys(target,'2160p');ui.keys(target,'Enter',literal=False)
                    def quality():
                        rows=c.call('shows.list')['result']['shows'];return rows[0].get('quality')=='2160p',json.dumps(rows)
                    assert h.wait_for(quality,'TUI quality prompt persists the selected show override')
                    ui.keys(target,',');assert ui.assert_pane(target,'Download settings')
                    # Select destination (row 9), edit a draft then cancel the entire form.
                    for _ in range(9):ui.keys(target,'Down',literal=False)
                    ui.keys(target,'Right',literal=False);ui.keys(target,'C-a',literal=False)
                    destination=box.root/'new downloads';destination.mkdir()
                    ui.keys(target,str(destination));ui.keys(target,'Enter',literal=False)
                    ui.keys(target,'Escape',literal=False)
                    case.check(c.call('app.settings.get')['result']['save_path']==str(box.downloads),'Esc discards destination draft in the real daemon','original destination retained')
                    ui.keys(target,'F6',literal=False);assert ui.assert_pane(target,'Download settings')
                    for _ in range(9):ui.keys(target,'Down',literal=False)
                    ui.keys(target,'Right',literal=False);ui.keys(target,'C-a',literal=False);ui.keys(target,str(destination));ui.keys(target,'Enter',literal=False)
                    ui.keys(target,'Down',literal=False);ui.keys(target,'Right',literal=False);ui.keys(target,'C-a',literal=False);ui.keys(target,'300');ui.keys(target,'Enter',literal=False)
                    ui.keys(target,'Enter',literal=False)
                    def saved():
                        s=c.call('app.settings.get')['result'];return s['save_path']==str(destination) and not s['seed_by_default'] and s['idle_exit_secs']==300,json.dumps(s)
                    assert h.wait_for(saved,'Enter commits destination and idle exit without changing upload opt-in')
                    ui.keys(target,'1')
                    h.stop_serve(p);assert ui.assert_pane(target,'fbittorrent stopped')
                    h.notify.close();p=h.start_serve(idle_exit_secs=300);h.receive_ready();h.wait_engine()
                    assert h.wait_for(lambda:(ui.clients()==1,ui.capture(target)),'the same terminal attaches to the replacement daemon')
                    with d.ApiConn(h) as fresh:
                        fresh.hello();settings=fresh.call('app.settings.get')['result'];shows=fresh.call('shows.list')['result']['shows']
                        case.check(settings['save_path']==str(destination) and not settings['seed_by_default'] and shows[0]['quality']=='2160p','settings and per-show quality survive a real daemon restart without opting into upload',json.dumps(dict(settings=settings,shows=shows)))
                        fresh.call('torrents.rename',dict(hash=item.info_hash,name='After restart'))
                        assert ui.assert_pane(target,'After restart')
                        fresh.call('torrents.rename',dict(hash=item.info_hash,name=''))
                        assert ui.assert_pane(target,'everyday.bin')
                    ui.keys(target,'q');assert h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches without stopping the daemon')
                    case.check(fingerprint()==before,'entire everyday journey leaves payload unchanged',repr(fingerprint()))
                finally:ui.close()
            h.stop_serve(p)
    return fbtlog.suite_status('tui_everyday')

if __name__=='__main__':raise SystemExit(main())
