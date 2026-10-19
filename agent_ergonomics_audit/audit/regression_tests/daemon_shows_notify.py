"""P6-10: real EZTV checks and exact desktop-notification argv in private state."""
from contextlib import contextmanager
import json
import time

import daemon as d
import daemon_notify as notify
import fixtures

IMDB = 'tt1234001'


def dataset(box, count):
    releases = []
    for index in range(count):
        key = f'S01E{index + 1:02}'
        releases.append(dict(id=index + 1, imdb_id=IMDB[2:],
            title='Local.Fixture.' + key + '.1080p', season='1', episode=str(index + 1),
            seeds=2, size_bytes='262144', date_released_unix=1792274400 + index,
            magnet_url='magnet:?xt=urn:btih:' + f'{index + 1:040x}'))
    (box.root / 'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],
        torrents_count=count, limit=100, page=1, torrents=releases)))


@contextmanager
def owned(name, binary, systemd):
    with d.Case('daemon', name) as case, d.Sandbox() as box, \
            fixtures.HttpFixture(box.root, {'/api/get-torrents': dict(file='episodes.json')}) as http:
        notify.config(box, notify=True)
        notify.shims(box)
        box.env['FBT_EZTV_BASE_URL'] = http.base
        box.keys.append('FBT_EZTV_BASE_URL')
        cache = d.Path(box.env['XDG_CACHE_HOME']) / 'fbittorrent/imdb/by-id'
        cache.mkdir(parents=True)
        title = dict(id=IMDB, name='Local Fixture', year=2026, kind='tvSeries', stars='',
            poster_url='', rating=None, votes=None, blurb='', wiki_url='')
        (cache / (IMDB + '.json')).write_text(json.dumps(title))
        dataset(box, 4)
        with d.Harness(case, binary, box, systemd) as h:
            process = h.start_serve(idle_exit_secs=0)
            if not (h.receive_ready() and h.wait_engine()):
                raise RuntimeError('private Shows notification daemon failed startup')
            try:
                with d.ApiConn(h) as client:
                    client.hello()
                    client.call('subscribe', dict(topics=['events']))
                    client.call('shows.follow', dict(imdb=IMDB, quality='1080p'))
                    # Today's private store is fresh so opening a UI does not race
                    # the explicit check with a stale-on-open check.
                    store = d.Path(box.env['XDG_DATA_HOME']) / 'fbittorrent/shows.json'
                    saved = json.loads(store.read_text())
                    saved['shows'][0]['last_checked'] = int(time.time())
                    store.write_text(json.dumps(saved))
                    yield case, box, h, client, http
            finally:
                if process.child.poll() is None:
                    h.stop_serve(process)
                h.lock_free()


def check(case, h, client, expected):
    result = client.call('shows.check', dict(imdb=IMDB))['result']
    if result.get('state') == 'pending':
        reference = result['ref']
        outcomes = []
        def finished():
            client.call('app.info')
            events = []
            for frame in client.pushed:
                events.append(frame)
                events.extend(frame.get('items', []))
                events.extend(frame.get('added', []))
            outcomes[:] = [event for event in events if event.get('ev') == 'mutation.result'
                           and event.get('ref') == reference]
            return bool(outcomes), json.dumps(outcomes)
        h.wait_for(finished, 'the real EZTV check finishes before notification assertions', timeout=5)
        if not outcomes:
            raise RuntimeError('Shows check produced no mutation result')
        case.check(outcomes[-1].get('outcome') == 'done', 'check succeeds', json.dumps(outcomes[-1]))
        result = outcomes[-1].get('result', {})
    case.check(result.get('new') == expected, 'check reports the actual new-release count', json.dumps(result))
    rows = client.call('torrents.list', {})['result']['torrents']
    case.check(rows == [], 'checks and notifications never add a torrent', json.dumps(rows))


def silent(case, box, expected):
    began = time.monotonic()
    observations = []
    while time.monotonic() - began < 1:
        rows = notify.calls(box)
        observations.append(dict(ms=round((time.monotonic() - began) * 1000), count=len(rows)))
        if len(rows) != expected:
            break
        time.sleep(.05)
    case.check(len(notify.calls(box)) == expected and observations[-1]['ms'] >= 900,
        'notification count stays unchanged throughout a one-second observation', json.dumps(observations))


def delivered(case, box, h, expected_count, title):
    h.wait_for(lambda: (len(notify.calls(box)) == expected_count, json.dumps(notify.calls(box))),
               'one notification arrives for the whole check', timeout=3)
    rows = notify.observe(case, box)
    if not rows:
        raise RuntimeError('notification shim recorded no argv')
    args = rows[-1]
    case.check(args[-2] == title and '-A' in args and 'window=Open Shows' in args,
        'notification title and Shows action match the check', json.dumps(args))
    return args[-1]


def shows_notify(binary, systemd):
    with owned('shows_notify', binary, systemd) as (case, box, h, client, http):
        check(case, h, client, 4)
        body = delivered(case, box, h, 1, '4 new episodes')
        case.check(body.count('Local Fixture S01E') == 3 and body.endswith('+ 1 more'),
            'one body lists at most three actual releases and the remaining count', body)
        silent(case, box, 1)
        case.check(len(http.requests) == 1, 'one explicit check performs one EZTV fixture request', str(http.requests))


def shows_notify_none_when_unchanged(binary, systemd):
    with owned('shows_notify_none_when_unchanged', binary, systemd) as (case, box, h, client, http):
        check(case, h, client, 4)
        delivered(case, box, h, 1, '4 new episodes')
        check(case, h, client, 0)
        silent(case, box, 1)
        notify.observe(case, box)
        case.check(len(http.requests) == 2, 'both checks really reach EZTV', str(http.requests))


def shows_notify_suppressed_interactive(binary, systemd):
    with owned('shows_notify_suppressed_interactive', binary, systemd) as (case, box, h, client, http):
        with d.ApiConn(h) as ui:
            ui.call('hello', dict(proto=1, client=dict(kind='tui', name='shows-notify-ui', version='0'), interactive=True))
            check(case, h, client, 4)
            silent(case, box, 0)
        h.wait_for(lambda: ((clients := client.call('app.stats')['result']['active_clients'])['tui'] == 0,
                           json.dumps(clients)), 'TUI presence closes')
        with d.ApiConn(h) as dashboard:
            dashboard.hello('bar')
            dashboard.call('app.client', dict(interactive=True, dashboard_visible=True))
            dataset(box, 5)
            check(case, h, client, 1)
            silent(case, box, 0)
        h.wait_for(lambda: ((clients := client.call('app.stats')['result']['active_clients'])['bar'] == 0,
                           json.dumps(clients)), 'dashboard presence closes')
        dataset(box, 6)
        check(case, h, client, 1)
        body = delivered(case, box, h, 1, '1 new episode')
        case.check(body == 'Local Fixture S01E06', 'background checks notify again after both UIs close', body)
        case.check(len(http.requests) == 3, 'fresh UI connects never add an implicit source request', str(http.requests))


CASES = (shows_notify, shows_notify_none_when_unchanged, shows_notify_suppressed_interactive)
