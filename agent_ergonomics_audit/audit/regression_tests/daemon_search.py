"""P3-01: real daemon/Nova sessions, local scripts, private XDG, no public requests."""
from contextlib import contextmanager
import json
from pathlib import Path
import shutil
import time

import daemon as d
from daemon_bus import RevisionStream


@contextmanager
def owned(name, binary, systemd):
    with d.api_owned(name, binary, systemd) as (case, sandbox, harness, process):
        runtime = Path(sandbox.env['XDG_DATA_HOME']) / 'fbittorrent/nova3'
        engines = runtime / 'engines'
        engines.mkdir(parents=True, exist_ok=True)
        for provider in ('alpha', 'beta'):
            shutil.copyfile(Path(__file__).parent / 'fixtures/providers' / (provider+'.py'), engines / (provider+'.py'))
        yield case, sandbox, harness, process, runtime


def start(client, query='hold', providers=None):
    args = dict(query=query)
    if providers is not None: args['providers'] = providers
    return client.call('search.start', args)['result']['search_id']


def snapshot(client, identifier):
    topic = f'search:{identifier}'
    sub = client.call('subscribe', dict(topics=[topic], interval_ms=500))['result']['sub']
    stream = RevisionStream(client)
    frames = stream.snapshot(sub, [topic])
    items = [item for frame in frames for item in frame.get('items', [])]
    return sub, stream, items


def results(client, harness, identifier):
    captured = []
    def ready():
        sub, _, items = snapshot(client, identifier)
        client.call('unsubscribe', dict(sub=sub))
        captured[:] = items
        return bool(items), json.dumps(items, ensure_ascii=False)
    if not harness.wait_for(ready, 'local Nova results reach a real keyed API snapshot'):
        raise RuntimeError('search never produced a result')
    return captured


def groups(runtime):
    directory = runtime / 'fixture-groups'
    return [json.loads(p.read_text()) for p in directory.glob('*.json')]


def members(pgid):
    rows = []
    for path in Path('/proc').glob('[0-9]*/stat'):
        try:
            fields = path.read_text().rsplit(') ', 1)[1].split()
            if int(fields[2]) == pgid:
                rows.append(dict(pid=int(path.parent.name), state=fields[0], pgid=pgid))
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            pass
    return rows


def live(pgid):
    return [row for row in members(pgid) if row['state'] != 'Z']


def wait_groups(harness, runtime):
    if not harness.wait_for(lambda: (bool(groups(runtime)), json.dumps(groups(runtime))),
                            'real provider writes its inherited process group and child PID'):
        raise RuntimeError('provider never started')
    return groups(runtime)


def search_dedupe_and_repair(binary, systemd):
    with owned('search_dedupe_and_repair', binary, systemd) as (case, sandbox, h, process, runtime), d.ApiConn(h) as client:
        hello = client.hello()['result']
        case.check('search' in hello['caps'], 'search capability is negotiated', json.dumps(hello))
        providers = client.call('search.providers')['result']['providers']
        case.check({p['name'] for p in providers} == {'alpha', 'beta'} and all(p['kind'] == 'nova' for p in providers),
                   'the daemon inspects both real local Nova scripts', json.dumps(providers))
        identifier = start(client, 'Sintel')
        observed = []
        def merged():
            observed[:] = results(client,h,identifier)
            return len(observed) == 1 and observed[0]['providers'] == ['alpha','beta'], json.dumps(observed,ensure_ascii=False)
        h.wait_for(merged,'duplicate btih results converge to one row listing both providers')
        raw = (runtime / 'alpha-raw.txt').read_bytes()
        artifact = case.path.with_suffix('.provider-raw.txt'); artifact.write_bytes(raw)
        case.step('compare raw provider bytes and normalized API result',
                  'repaired best-seeded name; unknown measurements absent').close(raw.decode() == 'è¾›ç‰¹å°”' and observed[0]['name'] == '辛特尔' and observed[0]['seeds'] == 9
                   and 'leechers' not in observed[0] and 'uploaded_at' not in observed[0],
                   json.dumps(dict(raw_hex=raw.hex(),items=observed),ensure_ascii=False),[artifact])
        # Provider preferences and filtering are exercised through the API too.
        client.call('search.set_provider',dict(name='beta',enabled=False))
        filtered=start(client,'Sintel',providers=['alpha'])
        items=results(client,h,filtered)
        case.check(items[0]['providers']==['alpha'],'provider selection runs only the requested enabled provider',json.dumps(items))
        client.call('search.stop',dict(search_id=filtered)); client.call('search.stop',dict(search_id=identifier))


def search_fifth_busy(binary, systemd):
    with owned('search_fifth_busy',binary,systemd) as (case,sandbox,h,process,runtime),d.ApiConn(h) as client:
        client.hello()
        identifiers=[start(client,f'hold-{n}',providers=['alpha']) for n in range(4)]
        failure=client.call('search.start',dict(query='fifth'),error='busy')
        case.check(failure['error']['retryable'] is True and failure['error']['details']['limit']=='sessions',
                   'fifth concurrent real provider session is retryable busy naming sessions',json.dumps(failure))
        client.call('search.stop',dict(search_id=identifiers.pop()))
        replacement=start(client,'hold-replacement',providers=['alpha'])
        case.check(replacement not in identifiers,'stopping a session immediately releases one admission',str(replacement))
        for identifier in identifiers+[replacement]: client.call('search.stop',dict(search_id=identifier))


def search_session_grace(binary, systemd):
    with owned('search_session_grace',binary,systemd) as (case,sandbox,h,process,runtime):
        with d.ApiConn(h) as first:
            first.hello(); identifier=start(first,providers=['alpha']); before=results(first,h,identifier)
            original=wait_groups(h,runtime)
        time.sleep(.25)
        with d.ApiConn(h) as second:
            second.hello(); sub,stream,after=snapshot(second,identifier)
            case.check(after==before and groups(runtime)==original and all(live(g['pgid']) for g in original),
                       'reconnect within 30 seconds reclaims the same session and retained keyed results without a provider restart',
                       json.dumps(dict(before=before,after=after,groups=original)))
            # Read an authoritative summary, not a local session count.
            summary=second.call('subscribe',dict(topics=['summary']))['result']['sub']
            frames=stream.snapshot(summary,['summary'])
            busy=[f['data']['daemon']['busy_reasons'] for f in frames if f.get('topic')=='summary']
            case.check(any('search' in reasons for reasons in busy),'retained/reclaimed session keeps the daemon busy as search',json.dumps(busy))
            second.call('search.stop',dict(search_id=identifier))


def search_process_group_gone(binary, systemd):
    with owned('search_process_group_gone',binary,systemd) as (case,sandbox,h,process,runtime):
        with d.ApiConn(h) as first:
            first.hello(); identifier=start(first,providers=['alpha']); results(first,h,identifier)
            original=wait_groups(h,runtime)
            case.check(all(live(g['pgid']) and any(m['pid']==g['child'] for m in live(g['pgid'])) for g in original),
                       'provider-created child is a real live member of the Nova process group',json.dumps([members(g['pgid']) for g in original]))
        began=time.monotonic()
        time.sleep(28)
        case.check(all(live(g['pgid']) for g in original),'disconnect grace does not kill providers early',
                   json.dumps(dict(elapsed=time.monotonic()-began,members=[members(g['pgid']) for g in original])))
        with d.ApiConn(h) as observer:
            observer.hello()
            h.wait_for(lambda:(all(not live(g['pgid']) for g in original),json.dumps([members(g['pgid']) for g in original])),
                       '30-second lease expiry terminates every provider process-group member',timeout=5)
            elapsed=time.monotonic()-began
            failure=observer.call('subscribe',dict(topics=[f'search:{identifier}']),error='not_found')
            case.check(29.5<=elapsed<=33 and failure['error']['details']['search_id']==identifier,
                       'expired session is not found after 30 seconds and no live group member survives',
                       json.dumps(dict(elapsed=elapsed,error=failure,members=[members(g['pgid']) for g in original])))


CASES=(search_dedupe_and_repair,search_session_grace,search_fifth_busy,search_process_group_gone)


def search_grab_and_ledger(binary, systemd):
    with owned('search_grab_and_ledger',binary,systemd) as (case,sandbox,h,process,runtime),d.ApiConn(h) as client:
        client.hello(); identifier=start(client,'Sintel'); results(client,h,identifier)
        client.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+'a'*40),stopped=True))
        captured=[]
        def added():
            captured[:]=results(client,h,identifier)
            return captured[0].get('added')=='a'*40,json.dumps(captured)
        h.wait_for(added,'streamed search results annotate the daemon ledger without client file access')
        source=sandbox.root/'selected';source.mkdir(); payload=source/'selected.bin';payload.write_bytes(b'local search grab\n'*1024)
        fixture=d.fixtures.torrent_fixture(source,[payload],name='Selected search')
        with d.fixtures.HttpFixture(sandbox.root) as http:
            result=dict(key=fixture.info_hash,name='Selected search',providers=['alpha'],url=http.base+'/selected/'+fixture.torrent.name)
            ref='search-real-url'
            response=client.call('search.grab',dict(result=result,save_path=str(source)),ref=ref)
            case.check(response['result']==dict(state='pending',ref=ref),'grab returns its exact pending ref',json.dumps(response))
            outcome=d.mutation_outcome(client,ref)
            row=client.call('torrents.get',dict(hash=fixture.info_hash))['result']['torrent']
            case.check(outcome['outcome']=='done' and outcome['result']['hash']==fixture.info_hash and row['origin']=='search',
                       'real loopback metainfo is admitted and confirmed with search origin',json.dumps(dict(outcome=outcome,row=row)))
        sources=client.call('search.browse_sources')['result']['sources']
        site=next(s for s in sources if not s['in_app'])
        empty=client.call('search.browse',dict(source=site['id']))
        case.check(empty['result']==dict(items=[]) and site['url'].startswith('https://'),'site-only browse sources are catalogued without fetching a public site',json.dumps(dict(site=site,response=empty)))
        client.call('app.settings.set',dict(network_interface='fbtmissing0'))
        h.wait_for(lambda:((value:=client.call('app.info')['result'])['state']=='parked',json.dumps(value)), 'park the real daemon engine before a search grab')
        failure=client.call('search.grab',dict(search_id=identifier,key='a'*40),error='engine_unavailable')
        case.check(failure['error']['details']['state']=='parked' and 'VPN' in failure['error']['hint'],
                   'parked grab is refused before pending with the VPN hint',json.dumps(failure))
        client.call('search.stop',dict(search_id=identifier))


CASES=CASES+(search_grab_and_ledger,)
