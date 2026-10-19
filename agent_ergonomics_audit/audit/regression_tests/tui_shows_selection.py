#!/usr/bin/env python3
"""S05: follow/check/open/mark/undo real Shows releases in an 80-column terminal."""
import argparse
import json
from pathlib import Path
import daemon as d
import fbtlog
import fixtures
import native

IMDB = 'tt1234001'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--log-root', type=Path)
    args = parser.parse_args()
    if args.log_root:
        fbtlog.LOG_ROOT = args.log_root.resolve()
    with fbtlog.Case('tui_shows_selection', 'follow_check_mark') as case, d.Sandbox() as box:
        case.check(bool(d.private_environment(box)), 'prove private XDG and downloads before any product starts', str(box.root))
        cache = Path(box.env['XDG_CACHE_HOME']) / 'fbittorrent/imdb/by-id'
        cache.mkdir(parents=True, exist_ok=True)
        (cache / (IMDB + '.json')).write_text(json.dumps(dict(id=IMDB, name='Local Fixture',
            kind='tvSeries', year=2026, stars='', poster_url='', rating=None, votes=None, blurb='', wiki_url='')))
        names = cache.parent / 'by-name-v2'
        names.mkdir(exist_ok=True)
        (names / 'local_fixture.json').write_text(json.dumps(dict(id=IMDB)))
        release = dict(id=1, imdb_id=IMDB[2:], title='Local.Fixture.S01E01.1080p',
            season='1', episode='1', seeds=1, size_bytes='262144', date_released_unix=1792274401,
            magnet_url='magnet:?xt=urn:btih:' + '1' * 40)
        (box.root / 'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],
            torrents_count=1, limit=100, page=1, torrents=[release])))
        with fixtures.HttpFixture(box.root, {'/api/get-torrents': dict(file='episodes.json')}) as http:
            box.env.update(FBT_EZTV_BASE_URL=http.base, FBT_IMDB_BASE_URL=http.base)
            box.keys += ['FBT_EZTV_BASE_URL', 'FBT_IMDB_BASE_URL']
            with d.Harness(case, args.binary.resolve(), box, False) as h:
                process = h.start_serve(idle_exit_secs=0)
                h.receive_ready()
                h.wait_engine()
                with d.ApiConn(h) as api:
                    api.hello()
                    ui = native.AttachedTui(case, str(args.binary.resolve()), box, h, 'fbt-fbt-p4-07-swt-shows')
                    try:
                        target, done = ui.start()
                        result = ui.tmux('resize-window', '-t', target, '-x', '80', '-y', '24')
                        case.check(result.returncode == 0, 'terminal is 80 columns', result.stderr or '80x24')
                        ui.keys(target, '3')
                        ui.keys(target, 'n')
                        assert ui.assert_pane(target, 'Follow show')
                        ui.keys(target, IMDB)
                        ui.keys(target, 'Enter', literal=False)
                        assert ui.assert_pane(target, 'show followed')
                        assert ui.assert_pane(target, 'Local Fixture')
                        ui.keys(target, 'R')

                        def unread(count):
                            rows = api.call('shows.releases', dict(inbox=True))['result']['releases']
                            return len(rows) == count, json.dumps(rows)

                        assert h.wait_for(lambda: unread(1), 'real EZTV check produces one unread release')
                        ui.keys(target, 'Home', literal=False)
                        ui.keys(target, 'Enter', literal=False)
                        assert ui.assert_pane(target, 'S01E01')
                        assert ui.assert_pane(target, '(m) Mark read')
                        pane = ui.capture(target)
                        case.path.with_suffix('.visible.pane.txt').write_text(pane)
                        case.check(any('▌' in line and 'S01E01' in line for line in pane.splitlines()),
                            'inbox has a visible selected release, not only its unread header', pane)
                        ui.keys(target, 'm')
                        assert h.wait_for(lambda: unread(0), 'm marks the visible release read in the daemon')
                        assert ui.assert_pane(target, 'All caught up')
                        ui.keys(target, 'u')
                        assert h.wait_for(lambda: unread(1), 'u restores that release to the real unread inbox')
                        assert ui.assert_pane(target, 'S01E01')
                        assert ui.assert_pane(target, '(m) Mark read')
                        # Switch between full history and inbox without stale-topic actions.
                        ui.keys(target, 'Left', literal=False)
                        ui.keys(target, 'Down', literal=False)
                        ui.keys(target, 'Enter', literal=False)
                        assert ui.assert_pane(target, 'S01E01')
                        assert ui.assert_pane(target, '[Releases]')
                        ui.keys(target, 'Left', literal=False)
                        ui.keys(target, 'Home', literal=False)
                        ui.keys(target, 'Enter', literal=False)
                        assert ui.assert_pane(target, '[Unread]')
                        assert ui.assert_pane(target, 'S01E01')
                        assert ui.assert_pane(target, '(m) Mark read')
                        case.check(api.call('torrents.list', {})['result']['torrents'] == [],
                            'following checking marking and undo never start a download', 'no torrents added')
                        ui.keys(target, 'q')
                        assert h.wait_for(lambda: (done.exists(), ui.capture(target)), 'q detaches the terminal')
                        case.check(api.call('shows.releases', dict(inbox=True))['result']['releases'][0]['key'] == 'S01E01',
                            'the daemon stays alive with the same unread release after q', 'S01E01')
                    finally:
                        ui.close()
                h.stop_serve(process)
    return fbtlog.suite_status('tui_shows_selection')


if __name__ == '__main__':
    raise SystemExit(main())
