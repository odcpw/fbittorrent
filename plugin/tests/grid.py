#!/usr/bin/env python3
"""Native browse grids over real YTS/TPB parsers and daemon-local poster files."""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from PIL import Image, ImageDraw

from dashboard import Session, ROOT
import daemon
import fixtures
from fbtlog import Case, suite_status


def content(box):
    cache = Path(box.env['XDG_CACHE_HOME']) / 'fbittorrent'
    titles, posters = cache / 'imdb/by-id', cache / 'posters'
    titles.mkdir(parents=True, exist_ok=True, mode=0o700)
    posters.mkdir(mode=0o700)
    yts, tpb, ratings = [], [], ['tconst\taverageRating\tnumVotes\n']
    for index in range(70):
        ident, info_hash = 'tt' + str(8700000 + index), f'{index + 1000:040x}'
        name = f'Owned grid film {index:02d}'
        card = dict(id=ident, name=name, year=2000 + index % 26, kind='feature', stars='',
                    poster_url='https://poster.invalid/' + ident + '.jpg', rating=7.4,
                    votes=1000, blurb='Owned local poster fixture.', wiki_url='')
        (titles / (ident + '.json')).write_text(json.dumps(card))
        # Distinct image URLs and bytes exercise sixty asynchronous JPEG decodes,
        # rather than reusing Qt's image cache for sixty copies of one source.
        image = Image.new('RGB', (120, 180), (30 + index * 3, 40 + index * 2, 70 + index))
        ImageDraw.Draw(image).text((8, 80), f'Film {index:02d}', fill=(255, 255, 255))
        image.save(posters / (ident + '.jpg'))
        ratings.append(f'{ident}\t7.4\t1000\n')
        yts.append(dict(title_long=name + ' (2026)', imdb_code=ident, rating=7.4,
                        torrents=[dict(hash=info_hash, quality='1080p', size_bytes=1048576,
                                       seeds=1000-index, peers=0)]))
        tpb.append(dict(id=index, info_hash=info_hash, name=name, size=1048576,
                        seeders=1000-index, leechers=0, imdb=ident, added=1000+index))
    (cache / 'title.ratings.tsv').write_text(''.join(ratings))
    return {'/api/v2/list_movies.json': {'json': {'data': {'movies': yts}}},
            '/precompiled/data_top100_207.json': {'json': tpb}}


def state(q):
    return json.loads(q.ipc('gridState'))


def ready(q):
    result = state(q)
    return result if (result.get('grid') or {}).get('ready') == 60 else None


def capture(case, q):
    image = case.path.with_suffix('.png')
    q.ipc('grab', image)
    q.wait(lambda: image.exists() and image.stat().st_size > 0)
    return image


def run(binary):
    previous = ROOT / 'target/test-logs/qs_grid'
    if previous.exists():
        shutil.copytree(previous, ROOT / 'target/test-logs/fbt-p3-17-71l/runs' /
                        datetime.datetime.now().strftime('%H%M%S'))
    with Case('qs_grid', 'fixture') as setup, fixtures.Sandbox() as box:
        with fixtures.HttpFixture(box.root, content(box)) as http:
            box.env['FBT_TEST_BROWSE_ROOT'] = http.base
            box.keys.append('FBT_TEST_BROWSE_ROOT')
            with daemon.Harness(setup, binary, box, False) as harness:
                process = harness.start_serve(idle_exit_secs=0)
                if not (harness.receive_ready() and harness.wait_engine()):
                    raise RuntimeError('owned browse daemon did not start')
                with daemon.ApiConn(harness) as client:
                    hello = client.hello()['result']
                    os.environ['FBT_TEST_RENDER_TIMING'] = '1'
                    q = Session(setup, box)
                    try:
                        q.compositor.ipc('output * mode 1920x1080')
                        q.wait(lambda: q.ipc('ready') == 'true')
                        with Case('qs_grid', 'grid_renders') as case:
                            offset = q.log.stat().st_size
                            tpb_ms = None
                            began = time.monotonic()
                            q.shell('summon', '{"view":"search"}')
                            yts = q.wait(lambda: ready(q), timeout=5)
                            yts_ms = (time.monotonic() - began) * 1000
                            case.check(yts_ms <= 1000 and yts['grid']['cells'] == 60
                                       and yts['total'] == 70 and yts['label'] == 'YTS · Latest movies'
                                       and yts['query'] == '',
                                       'empty Search renders the YTS grid and sixty unique local posters within 1 s',
                                       json.dumps(dict(ms=yts_ms, state=yts)))
                            capture(case, q)
                            # Exercise the actual Browse sheet navigation, not a
                            # test-only callback that bypasses its selection.
                            q.keys(['-k','Down'],lambda:(q.state().get('view') or {}).get('focus')=='results')
                            try:
                                q.keys(['b'], lambda: (q.state().get('view') or {}).get('panel') == 'browse')
                            except TimeoutError as error:
                                raise RuntimeError(q.ipc('searchFocus')) from error
                            q.keys(['jjjjjjj'], lambda: True)
                            began = time.monotonic()
                            keyboard = subprocess.Popen(['wtype', '-s', '80', '-k', 'Return', '-s', '700'], env=q.env)
                            try:
                                q.wait(lambda: (value := ready(q)) and value['label'] == 'The Pirate Bay · Top 100 · Movies (HD)', timeout=5)
                                tpb = state(q)
                                tpb_ms = (time.monotonic() - began) * 1000
                            finally:
                                keyboard.wait(timeout=3)
                            case.check(tpb_ms <= 1000 and tpb['query'] == tpb['label']
                                       and tpb['grid']['cells'] == 60 and tpb['total'] == 70,
                                       'TPB selection renders its bounded grid within 1 s and shows the list name',
                                       json.dumps(dict(ms=tpb_ms, state=tpb, requests=http.requests)))
                            image = case.path.with_name('grid_renders_tpb.png')
                            q.ipc('grab', image)
                            q.wait(lambda: image.exists() and image.stat().st_size > 0)
                            cursor = tpb['cursor']
                            q.keys(['v'], lambda: state(q).get('grid') is None)
                            case.check(state(q)['cursor'] == cursor, 'v changes to rows without moving the cursor', json.dumps(state(q)))
                            q.keys(['v'], lambda: ready(q))
                            case.check(state(q)['cursor'] == cursor, 'v restores the grid and keeps the cursor', json.dumps(state(q)))
                            q.keys(['G'], lambda: state(q).get('cursor') == f'{1059:040x}')
                            case.check(state(q)['grid']['cursor'] == f'{1059:040x}',
                                       'End stays inside the sixty visible poster cells, even for seventy source rows', json.dumps(state(q)))
                        with Case('qs_grid', 'grid_frame_budget') as case:
                            text = q.log.read_bytes()[offset:].decode(errors='replace')
                            timings = [float(value) for value in re.findall(
                                r"(?:time in renderer:\s*|Frame rendered with 'software' renderloop in )([\d.]+)\s*ms", text)]
                            artifact = case.path.with_suffix('.frames.json')
                            artifact.write_text(json.dumps(dict(frames=timings, yts_ms=yts_ms, tpb_ms=tpb_ms), indent=2))
                            case.check(bool(timings) and max(timings, default=float('inf')) <= 32,
                                       'every renderer frame while sixty local posters load is <= 32 ms; missing samples fail',
                                       json.dumps(dict(frames=len(timings), slowest=max(timings, default=None), artifact=str(artifact))))
                        if 'staging' in hello['caps']:
                            with Case('qs_grid', 'grid_enter_stages') as case:
                                q.keys(['-k', 'Return'], lambda: (q.state().get('view') or {}).get('add', {}).get('visible'))
                                # Browse supplies a magnet, not a fabricated file
                                # list. Review must work before peers answer.
                                try:
                                    q.wait(lambda: (a := (q.state().get('view') or {}).get('add', {})).get('owned')
                                           and a.get('hash') == state(q)['cursor'] and a.get('canDownload'), timeout=12)
                                except TimeoutError as error:
                                    raise RuntimeError(json.dumps(q.state())) from error
                                rows = client.call('torrents.list', {})['result']['torrents']
                                case.check(len(rows) == 1 and rows[0]['staged'] and not rows[0]['checked']
                                           and not rows[0]['upload_enabled'] and not list(box.downloads.iterdir()),
                                           'Enter stages exactly one selected magnet before Download without payload transfer or upload', json.dumps(rows))
                        else:
                            print('UNMET: grid_enter_stages needs P3-06 staging on this branch; no passing staging case is recorded.', flush=True)
                    finally:
                        q.close()
                        os.environ.pop('FBT_TEST_RENDER_TIMING', None)
                harness.stop_serve(process)
                harness.lock_free()
    return suite_status('qs_grid')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
