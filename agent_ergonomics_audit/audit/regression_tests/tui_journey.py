#!/usr/bin/env python3
"""M3 owner journeys: each source drives real TUI input and loopback payload.

Run at each integrated release point: tui_journey.py target/release/fbittorrent.
No owner data or public network is used. --case permits a focused replay.
"""
import argparse
import json
from pathlib import Path
import sys
from fbtlog import Case,REPO,suite_status
import fbtlog
from tui_smoke import PaneUi
from tui_journey_fixture import Fixture,IMDB
from tui_journey_steps import exercise
sys.path.insert(0,str(REPO/'plugin/tests'))
import private_net

SOURCES=('magnet','file','search','rss','shows')


def add(ui,fixture,source):
    def contains(key,text,literal=True):
        if not ui.contains(key,text,literal=literal):raise RuntimeError('TUI source step failed: '+text)
    if source in ('magnet','file'):
        contains('a','streaming automatic')
        ui.press(fixture.torrent.magnet if source=='magnet' else str(fixture.torrent.torrent),'type a real '+source+' source')
        ui.press('Enter','submit the source to the daemon',literal=False)
    elif source=='search':
        contains('2','Search');ui.press('local journey','type the search query');contains('Enter',fixture.name,False)
        ui.press('Escape','leave query editing',literal=False)
        contains('v','IMDb · v/Esc Back')
        if not ui.observe('inspect poster card','real cached poster, title, rating and cast are readable',lambda p:'Journey Fixture' in p and '7.9' in p and 'Local cast' in p and any(c in p for c in ('▀','▄'))):raise RuntimeError('search poster is not usable')
        ui.press('Escape','close the search card',lambda p:'IMDb · v/Esc Back' not in p,literal=False)
        ui.press('Enter','download the selected deduplicated search result',literal=False)
    elif source=='rss':
        contains('4','RSS');contains('n','feed')
        ui.press(fixture.http.base+'/feed.xml','type the private feed URL');contains('Enter','127.0.0.1',False)
        if not ui.observe('select the newly added feed','feed row is selected before opening its items',lambda p:any('▌' in line and '127.0.0.1' in line for line in p.splitlines())):raise RuntimeError('new feed is not selected')
        contains('Enter','[Items]',False)
        if not ui.observe('read the fetched enclosure','real feed item appears',lambda p:fixture.name in p):raise RuntimeError('RSS item did not load')
        contains('m','✓ Read');contains('m','● Unread')
        if fixture.rows():raise RuntimeError('RSS read action unexpectedly created a transfer')
        ui.press('Enter','download the selected real feed enclosure',literal=False)
    else:
        contains('3','Shows');contains('n','Follow')
        ui.press(IMDB,'type the local catalogue IMDb id');contains('Enter','Journey Fixture',False)
        ui.press('R','explicitly check the followed show')
        if not fixture.h.wait_for(lambda:((rows:=fixture.client.call('shows.releases',dict(inbox=True))['result']['releases']) and rows[0]['title']==fixture.name,json.dumps(rows)),'explicit check populates the daemon inbox'):raise RuntimeError('Shows check did not finish')
        if fixture.rows():raise RuntimeError('Shows follow/check unexpectedly created a transfer')
        contains('Home','All unread',False);contains('Enter','[Unread]',False)
        if not ui.observe('select the discovered release','the fetched unread release is a visible keyboard target',lambda p:any('▌' in line and 'S01E01' in line for line in p.splitlines())):raise RuntimeError('Shows release did not become actionable')
        contains('m','Undo read')
        if not fixture.h.wait_for(lambda:(not fixture.client.call('shows.releases',dict(inbox=True))['result']['releases'],'unread inbox is empty'),'read removes the daemon inbox row'):raise RuntimeError('Shows read did not persist')
        if not ui.observe('read removes the visible inbox row','the consumed release disappears before Undo',lambda p:not any('▌' in line and 'S01E01' in line for line in p.splitlines())):raise RuntimeError('Shows read row stayed visible')
        contains('u','release restored to unread')
        if not ui.observe('select restored unread release','Undo restores the visible release target',lambda p:any('▌' in line and 'S01E01' in line for line in p.splitlines())):raise RuntimeError('restored Shows release is not actionable')
        ui.press('Enter','download the selected real Shows release',literal=False)
    if not fixture.h.wait_for(lambda:(any(r['hash']==fixture.torrent.info_hash for r in fixture.rows()),ui.capture(ui.target)),'TUI '+source+' action admits the real fixture',timeout=15):raise RuntimeError('source did not add a torrent')
    row=fixture.row();expected=source if source in ('search','rss','shows')else'manual'
    fixture.case.check(row['origin']==expected,'source origin is retained',json.dumps(row))
    fixture.case.check(row['upload_enabled'] is False and row['upload_policy']=='off' and row.get('uploaded_bytes',0)==0,
                       'every add source defaults to zero payload upload',json.dumps(row))
    ui.press('1','return to Torrents',lambda p:'Torrents' in p)


def run(binary,source):
    private_net.require()
    with Case('tui_journey',source) as case,Fixture(case,binary,source) as fixture:
        ui=PaneUi(case,str(binary),fixture.box,fixture.h,session='fbt-m3-'+source)
        try:
            ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','80','-y','24')
            add(ui,fixture,source);exercise(ui,fixture)
        finally:ui.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('binary',type=Path);parser.add_argument('--case',choices=SOURCES,action='append');parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();private_net.enter();binary=args.binary.resolve();names=args.case or SOURCES
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    for source in names:
        run(binary,source)
    return suite_status('tui_journey')


if __name__=='__main__':sys.exit(main())
