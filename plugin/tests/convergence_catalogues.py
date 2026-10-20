#!/usr/bin/env python3
"""CLI + dashboard/popup + tmux attached to one real loopback app daemon."""
import argparse
from contextlib import contextmanager, ExitStack
import hashlib
import json
from pathlib import Path
import subprocess
import time

from dashboard import Session
from convergence import evidence
from fbtlog import Case, suite_status
from tui_smoke import PaneUi
from tui_journey_fixture import Fixture, IMDB


@contextmanager
def surfaces(binary, name):
    with Case('cross_surface', name) as case, ExitStack() as stack:
        f = stack.enter_context(Fixture(case, binary, 'file'))
        q = Session(case, f.box, recipe='bar-host')
        stack.callback(q.close)
        keyboard = subprocess.Popen(['wtype', '-s', '600000'], env=q.env)
        stack.callback(lambda: (keyboard.terminate(), keyboard.wait(timeout=3)))
        ui = PaneUi(case, str(binary), f.box, f.h, session='fbt-converge-'+name)
        stack.callback(ui.close)
        ui.start_owned()
        ui.tmux('resize-window', '-t', ui.target, '-x', '120', '-y', '40')
        stack.enter_context(evidence(case, q, ui))
        q.wait(lambda: q.ipc('ready') == 'true')
        yield case, f, q, ui


def observe(case, q, ui, label, predicate, timeout=1.25):
    started = time.monotonic()
    last = None
    while time.monotonic()-started < timeout:
        state, pane = q.state(), ui.capture(ui.target)
        last = dict(state=state, pane=pane)
        if predicate(state, pane):
            case.check(True, label, json.dumps(dict(after_action_ms=round((time.monotonic()-started)*1000), **last)))
            return state
        time.sleep(.03)
    case.check(False, label, json.dumps(last))
    raise AssertionError(label)


def view(state): return state.get('view') or {}
def rss(state): return view(state).get('rss') or {}
def shows(state): return view(state).get('shows') or {}
def add(state): return view(state).get('add') or {}


def torrents(binary):
    with surfaces(binary, 'torrents') as (case, f, q, ui):
        h, c, hash_ = f.h, f.client, f.torrent.info_hash
        q.shell('summon', '{"view":"torrents"}')
        q.wait(lambda: view(q.state()).get('window'))
        q.compositor.ipc('[title="^fbittorrent$"] focus')
        h.cli('add', str(f.torrent.torrent))
        observe(case, q, ui, 'CLI add appears once in the rendered dashboard and TUI',
                lambda s,p: len(s['torrents']) == view(s).get('rows') == 1 and f.name in p and any(f.name in t for t in s['rendered']))
        assert f.h.wait_for(lambda: ((row:=f.row()).get('completed_bytes',0)>0, json.dumps(row)), 'loopback fixture has real payload')
        q.ipc('popup')
        q.wait(lambda: len((q.state().get('popup') or {}).get('rows',[])) == 1)
        refs = q.state()['mutationRefs']
        assert q.ipc('popupPause') == 'true'
        observe(case, q, ui, 'popup pause converges to the daemon, dashboard, terminal and paused counter',
                lambda s,p: f.row()['state'] == 'paused' and s['torrents'][0]['state']=='paused' and s['status']['stopped']==1
                and 'Pause' in p and any(section['key']=='paused' and len(section['rows'])==1 for section in s['popup']['sections']))
        case.check(q.state()['mutationRefs']==refs+1, 'one popup action sends one mutation', str(refs))
        assert ui.contains('N', 'Rename to:')
        ui.press('C-a', 'replace the selected display name', literal=False)
        ui.press('Converged title', 'type the new display name')
        ui.press('Enter', 'confirm the TUI rename', literal=False)
        observe(case, q, ui, 'TUI rename updates every open row without renaming its payload',
                lambda s,p: f.row()['name']=='Converged title' and s['torrents'][0]['name']=='Converged title' and 'Converged title' in p
                and any('Converged title' in t for t in s['rendered']) and any(r['name']=='Converged title' for g in s['popup']['sections'] for r in g['rows']))
        h.cli('limit', hash_, '--down', '128K')
        q.shell('summon', json.dumps(dict(view='torrents', hash=hash_)))
        q.keys(['-k','Return'], lambda: (view(q.state()).get('details') or {}).get('visible'))
        assert ui.contains('Enter', '[Overview]', literal=False)
        observe(case, q, ui, 'CLI per-torrent limit reaches both open Details views',
                lambda s,p: s['torrents'][0].get('download_limit_bps')==131072 and '131072 B/s' in p
                and view(s)['details']['overview']['cap']==view(s)['details']['overview']['effective']
                and view(s)['details']['overview']['cap'] not in ('', 'Unlimited', '—'))
        moved=f.box.root/'moved';moved.mkdir()
        h.cli('move', hash_, str(moved))
        observe(case, q, ui, 'CLI move updates the open dashboard and TUI location',
                lambda s,p: s['torrents'][0]['save_path']==str(moved) and str(moved) in p
                and any(str(moved) in t for t in s['rendered']))
        before={str(path.relative_to(moved)):hashlib.sha256(path.read_bytes()).hexdigest() for path in moved.rglob('*') if path.is_file()}
        case.check(bool(before), 'move keeps the real partial payload under the private destination', json.dumps(before))
        ui.press('Escape','return to the torrent list',literal=False)
        q.keys(['-k','Escape'],lambda:not view(q.state())['details']['visible'])
        q.keys(['d'],lambda:(view(q.state()).get('dialog') or {}).get('kind')=='remove')
        q.keys(['y'],lambda:view(q.state()).get('rows')==0)
        q.ipc('popup')
        observe(case,q,ui,'dashboard keep-files removal clears the CLI, popup model and terminal',
                lambda s,p:not f.rows() and not s['torrents'] and s['status']['total']==0 and 'Converged title' not in p
                and not s['popup']['rows'] and not s['popup']['sections'])
        q.native_hook('close')
        q.keys(['U'],lambda:len(f.rows())==1 and view(q.state()).get('rows')==1)
        q.ipc('popup')
        observe(case,q,ui,'dashboard Undo restores the same record across CLI and TUI',
                lambda s,p:f.row()['state']=='paused' and len(s['torrents'])==1 and 'Converged title' in p
                and any(r['hash']==hash_ and r['name']=='Converged title' for g in s['popup']['sections'] for r in g['rows']))
        after={str(path.relative_to(moved)):hashlib.sha256(path.read_bytes()).hexdigest() for path in moved.rglob('*') if path.is_file()}
        case.check(before==after and not f.row()['upload_enabled'],'remove/Undo keeps every payload byte and zero-upload permission',json.dumps(after))
        ui.press('p','resume from the TUI')
        observe(case,q,ui,'TUI resume converges to the dashboard and CLI',
                lambda s,p:f.row()['state']=='downloading' and s['torrents'][0]['state']=='downloading' and s['status']['active']==1 and 'Down' in p
                and len(s['popup']['rows'])==1 and s['popup']['rows'][0]['hash']==hash_)


def feeds(binary):
    with surfaces(binary, 'rss') as (case, f, q, ui):
        h,c=f.h,f.client
        q.shell('summon','{"view":"rss"}')
        q.wait(lambda:'rss' in view(q.state()))
        ui.press('4','keep RSS open in the TUI')
        h.cli('rss','add',f.http.base+'/feed.xml','--name','Shared feed')
        h.cli('rss','refresh','Shared feed')
        observe(case,q,ui,'CLI feed appears in both already-open catalogue views with one unread item',
                lambda s,p:len(rss(s).get('feeds',[]))==1 and rss(s)['feeds'][0]['unread']==1 and 'Shared feed' in p)
        assert ui.contains('Enter','[Items]',literal=False)
        q.compositor.ipc('[title="^fbittorrent$"] focus')
        q.keys(['-k','Return'],lambda:rss(q.state()).get('pane')=='items' and len(rss(q.state()).get('items',[]))==1)
        observe(case,q,ui,'the same unread RSS article is actionable in dashboard and TUI',
                lambda s,p:f.name in p and '● Unread' in p and rss(s)['items'][0]['title']==f.name and any(f.name in t for t in s['rendered']))
        q.keys(['m'],lambda:rss(q.state())['items'][0]['read'])
        observe(case,q,ui,'dashboard marking read updates the open terminal and CLI unread count',
                lambda s,p:'✓ Read' in p and h.cli('rss','list')['feeds'][0]['unread']==0 and rss(s)['feeds'][0]['unread']==0)
        ui.press('m','mark the item unread from the terminal')
        observe(case,q,ui,'TUI marking unread updates the dashboard and CLI item',
                lambda s,p:not rss(s)['items'][0]['read'] and rss(s)['feeds'][0]['unread']==1
                and h.cli('rss','items','Shared feed','--unread')['count']==1 and '● Unread' in p)
        refs=q.state()['mutationRefs']
        q.keys(['-k','Return'],lambda:add(q.state()).get('phase')=='ready')
        case.check(len(f.rows())==1 and f.row()['staged'] and not rss(q.state())['items'][0]['read'] and not list(f.box.downloads.iterdir()),
                   'review stages exactly one torrent without marking read or creating payload',json.dumps(f.row()))
        # An unrelated catalogue write while review is open must not be lost
        # when the review commits its source association.
        h.cli('rss','add',f.http.base+'/feed.xml?second=1','--name','Second feed')
        q.wait(lambda:len(rss(q.state())['feeds'])==2)
        q.keys(['-k','Return'],lambda:not add(q.state()).get('visible') and not f.row()['staged'])
        q.shell('summon','{"view":"rss"}')
        observe(case,q,ui,'review commit updates read/transfer state without losing the concurrent feed',
                lambda s,p:len(rss(s).get('feeds',[]))==2 and rss(s).get('items') and rss(s)['items'][0]['read'] and rss(s)['items'][0]['grabbed']
                and '✓ Read' in p and h.cli('rss','items','Shared feed')['items'][0]['grabbed'])
        case.check(len(f.rows())==1 and f.row()['origin']=='rss' and not f.row()['upload_enabled'] and q.state()['mutationRefs']==refs+2,
                   'RSS review and commit send one action each and retain one zero-upload transfer',json.dumps(f.rows()))
        h.cli('rss','remove','Second feed')
        observe(case,q,ui,'CLI removal leaves the shared feed and article selected on both open views',
                lambda s,p:len(rss(s)['feeds'])==1 and rss(s)['feeds'][0]['name']=='Shared feed' and f.name in p)
        ui.press('1','inspect the admitted RSS transfer')
        q.shell('summon','{"view":"torrents"}')
        q.ipc('popup')
        observe(case,q,ui,'the RSS commit reaches torrent rows and popup counters without duplicates',
                lambda s,p:len(s['torrents'])==1 and s['status']['total']==1 and f.name in p
                and any(r['hash']==f.torrent.info_hash for g in s['popup']['sections'] for r in g['rows']))


def television(binary):
    with surfaces(binary,'shows') as (case,f,q,ui):
        h,c=f.h,f.client
        q.shell('summon','{"view":"shows"}')
        q.wait(lambda:'shows' in view(q.state()))
        ui.press('3','keep Shows open in the TUI')
        h.cli('shows','follow',IMDB)
        observe(case,q,ui,'CLI follow/check pushes the new show and unread release into both open views',
                lambda s,p:len(shows(s).get('shows',[]))==len(shows(s).get('rows',[]))==1 and 'Journey Fixture' in p and shows(s)['shows'][0]['unread']==1)
        ui.press('Left','focus the followed-shows navigation before Enter',literal=False)
        assert ui.contains('Home','All unread',literal=False)
        assert ui.contains('Enter','[Unread]',literal=False)
        q.compositor.ipc('[title="^fbittorrent$"] focus')
        q.keys(['l'],lambda:shows(q.state()).get('focus')=='releases')
        observe(case,q,ui,'the discovered release is a real selected target on both surfaces',
                lambda s,p:'S01E01' in p and len(shows(s)['rows'])==1 and any(f.name in t for t in s['rendered']))
        ui.press('m','mark the selected release read from the terminal')
        observe(case,q,ui,'TUI mark read removes the same inbox row and unread counter from the dashboard and CLI',
                lambda s,p:not shows(s)['rows'] and shows(s)['shows'][0]['unread']==0 and h.cli('shows','episodes',IMDB,'--new')['count']==0
                and not any(f.name in t for t in s['rendered']))
        ui.press('u','undo the read mark from the terminal')
        observe(case,q,ui,'TUI Undo restores the identical dashboard inbox row and CLI unread episode',
                lambda s,p:len(shows(s)['rows'])==1 and 'S01E01' in p and h.cli('shows','episodes',IMDB,'--new')['count']==1
                and any(f.name in t for t in s['rendered']))
        h.cli('shows','quality',IMDB,'2160p')
        q.wait(lambda:shows(q.state())['shows'][0].get('quality')=='2160p')
        # Select the show so its preference is actually rendered in the TUI.
        ui.press('Left','return to followed shows',literal=False)
        ui.press('j','select the followed show')
        observe(case,q,ui,'CLI quality is rendered on both clients without changing inbox read intent',
                lambda s,p:'2160p' in p and shows(s)['shows'][0].get('quality')=='2160p' and not shows(s)['rows'][0]['read'])
        ui.press('Home','return to the shared unread inbox',literal=False)
        ui.press('Enter','open unread releases',literal=False)
        q.keys(['m'],lambda:not shows(q.state())['rows'])
        observe(case,q,ui,'dashboard mark read removes the terminal inbox target too',
                lambda s,p:not shows(s)['rows'] and not any('▌' in line and 'S01E01' in line for line in p.splitlines()))
        q.keys(['u'],lambda:len(shows(q.state())['rows'])==1)
        observe(case,q,ui,'dashboard Undo restores the terminal target without creating a transfer',
                lambda s,p:'S01E01' in p and not f.rows() and not shows(s)['rows'][0]['read'])
        h.cli('shows','unfollow',IMDB)
        observe(case,q,ui,'CLI unfollow removes the show and release from both open views',
                lambda s,p:not shows(s)['shows'] and not shows(s)['rows'] and 'Journey Fixture' not in p and 'S01E01' not in p)


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--case',choices=['torrents','rss','shows'],default='torrents')
    parser.add_argument('--log-root',type=Path)
    args=parser.parse_args()
    if args.log_root:
        import fbtlog
        fbtlog.LOG_ROOT=args.log_root.resolve()
    {'torrents':torrents,'rss':feeds,'shows':television}[args.case](args.binary.resolve())
    raise SystemExit(suite_status('cross_surface'))
