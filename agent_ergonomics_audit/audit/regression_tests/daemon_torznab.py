"""P7-01: native private-indexer HTTP, credential hygiene and shared dedupe."""
from contextlib import contextmanager
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import urllib.parse

import daemon as d
import fixtures

KEY = 'Torznab-fixture VALUE&token=729'


def cli(*args, **kwargs):
    from agent_contract import cli as run
    return run(*args, **kwargs)


def configure(box, url, key=KEY, name='private_fixture'):
    folder = Path(box.env['XDG_CONFIG_HOME']) / 'fbittorrent'
    secret = folder / 'indexer.key'
    secret.write_text(key + '\n'); secret.chmod(0o600)
    path = folder / 'indexers.toml'
    path.write_text('[[torznab]]\nname = ' + json.dumps(name) + '\nurl = ' + json.dumps(url) + '\napi_key_file = ' + json.dumps(str(secret)) + '\n')
    path.chmod(0o600)
    return path, secret


@contextmanager
def owned(name, binary, systemd, endpoint='/api', *, unbound=False):
    with d.Case('daemon', name) as case, d.Sandbox() as box:
        # Exercise the P1-07 Torznab fixture bytes, in a mutable private copy.
        source = fixtures.HERE / 'fixtures/torznab'
        shutil.copyfile(source / 'query.json', box.root / 'query.json')
        routes = json.loads((source / 'routes.json').read_text())
        with fixtures.HttpFixture(box.root, routes) as http:
            config, secret = configure(box, http.base + endpoint)
            if unbound:
                d.select_replay_interface(Path(box.env['XDG_CONFIG_HOME']) / 'fbittorrent/config.toml', None)
            with d.Harness(case, binary, box, systemd) as h:
                process = h.start_serve(); h.receive_ready(); h.wait_engine()
                try:
                    yield case, box, h, http, config, secret
                finally:
                    if process.child.poll() is None: h.stop_serve(process)
                    h.lock_free()


def request_credentials(case, http):
    queries = [urllib.parse.parse_qs(urllib.parse.urlsplit(path).query) for path in http.requests]
    case.check(bool(queries) and all(query.get('apikey') == [KEY] for query in queries),
               'every owned indexer request has exactly the private key, decoded correctly',
               json.dumps(dict(request_count=len(queries), correct=all(query.get('apikey') == [KEY] for query in queries))))


def secret_free(case, *texts):
    texts = list(texts) + [case.path.read_text(), case.suite.stderr_path.read_text()]
    forms = [KEY, urllib.parse.quote(KEY, safe=''), urllib.parse.quote_plus(KEY)]
    case.check(all(form not in text for text in texts for form in forms),
               'public frames, CLI output, argv and daemon diagnostics contain neither literal nor URL-encoded key',
               'checked literal and percent/form encodings; credential values intentionally omitted')


def torznab_results(binary, systemd):
    # This positive control runs both a local Nova script and a loopback indexer;
    # the dedicated bound-indexer cases exercise the VPN route separately.
    with owned('torznab_results', binary, systemd, unbound=True) as (case, box, h, http, config, secret):
        fixture = json.loads((box.root / 'query.json').read_text())
        fixture['xml'] = fixture['xml'].replace('1' * 40, 'a' * 40).replace('value="2"', 'value="7"').replace('262144', '1234')
        (box.root / 'query.json').write_text(json.dumps(fixture))
        engines = Path(box.env['XDG_DATA_HOME']) / 'fbittorrent/nova3/engines'
        engines.mkdir(parents=True)
        shutil.copyfile(fixtures.HERE / 'fixtures/providers/alpha.py', engines / 'alpha.py')
        installed, _ = cli(case, box, binary, 'search', '--install-torznab', http.base + '/api', '--key-file', secret, '--json')
        case.check(installed['name'] == 'private_fixture' and config.stat().st_mode & 0o777 == 0o600,
                   'CLI installs/updates the native provider while retaining a private config', json.dumps(installed))
        with d.ApiConn(h) as c:
            hello = c.hello()['result']
            providers = c.call('search.providers')['result']['providers']
            case.check('torznab' in hello['caps'] and {row['name']: row['kind'] for row in providers} == {'alpha': 'nova', 'private_fixture': 'torznab'},
                       'the API advertises Torznab and reports both native/Nova kinds', json.dumps(providers))
            value, result = cli(case, box, binary, 'search', 'native fixture', '--wait', '8', '--json')
            row = value['results'][0]
            case.check(value['complete'] and value['count'] == 1 and row['seeds'] == 7 and row['size'] == 1234
                       and row['providers'] == ['alpha', 'private_fixture'] and row['provider_url'] == http.base + '/api',
                       'real Torznab and Nova responses deduplicate by hash before sorting and preserve provenance', result.stdout)
            # search.install accepts native HTTPS endpoints without fetching a Python script.
            pending = c.call('search.install', dict(sources=['https://127.0.0.1:1/torznab/api']))['result']
            completed = d.mutation_outcome(c, pending['ref'])
            configured = c.call('search.providers')['result']['providers']
            case.check(completed['result']['installed'] == ['https://127.0.0.1:1/torznab/api'] and not completed['result']['failed']
                       and sum(row['kind'] == 'torznab' for row in configured) == 2,
                       'the pending API install configures a native endpoint and publishes its provider kind', json.dumps(completed))
        request_credentials(case, http); secret_free(case)


def torznab_http_key_refused(binary, systemd):
    with owned('torznab_http_key_refused', binary, systemd) as (case, box, h, http, config, secret), d.ApiConn(h) as c:
        c.hello()
        previous = config.read_bytes()
        configure(box, 'http://192.0.2.1/api')
        refusal = c.call('search.start', dict(query='no remote HTTP'), error='invalid_args')
        case.check(refusal['error'].get('details', {}).get('key') == 'url' and not http.requests,
                   'keyed non-loopback HTTP fails as typed invalid_args before any request', json.dumps(refusal))
        cli(case, box, binary, 'search', '--install-torznab', 'http://192.0.2.1/api', '--key-file', secret, '--json', code=2)
        config.write_bytes(previous); config.chmod(0o600)
        secret.chmod(0o644)
        c.call('search.providers', error='invalid_args')
        case.check(secret.stat().st_mode & 0o777 == 0o644, 'unsafe credential mode is refused without repair')
        secret.chmod(0o600)
        link = secret.with_suffix('.symlink'); link.symlink_to(secret)
        config.write_text(config.read_text().replace(str(secret), str(link)))
        c.call('search.providers', error='invalid_args')
        config.write_bytes(previous); config.chmod(0o644)
        c.call('search.providers', error='invalid_args')
        case.check(config.stat().st_mode & 0o777 == 0o644, 'unsafe indexer config is refused without repair')
        config.chmod(0o600)
        replacement = config.with_suffix('.real'); config.rename(replacement); config.symlink_to(replacement)
        c.call('search.providers', error='invalid_args')
        secret_free(case)


def torznab_redirect_refused(binary, systemd):
    with owned('torznab_redirect_refused', binary, systemd, endpoint='/redirect', unbound=True) as (case, box, h, http, config, secret), \
            fixtures.HttpFixture(box.root, {'/leak': dict(json={})}) as other, d.ApiConn(h) as c:
        c.hello()
        http.routes['/redirect']['headers']['Location'] = other.base + '/leak'
        cli(case, box, binary, 'search', 'redirect fixture', '--wait', '5', '--json', code=5)
        item = dict(key='redirect', name='Redirect fixture', url=http.base + '/redirect', providers=['private_fixture'], provider_url=http.base + '/redirect')
        pending = c.call('search.grab', dict(result=item, stopped=True), ref='redirect-grab')['result']
        outcome = next(frame for frame in c.events() if frame.get('ev') == 'mutation.result' and frame.get('ref') == pending['ref'])
        case.check(outcome['outcome'] == 'failed' and outcome['error']['code'] == 'upstream'
                   and outcome['error']['details']['source'] == 'torznab' and 'cross-origin' in outcome['error']['message'] and not other.requests,
                   'cross-origin query/download redirects are refused as upstream without hitting their target', json.dumps(outcome))
        http.routes['/redirect']['headers']['Location'] = '{base}/api'
        value, _ = cli(case, box, binary, 'search', 'same origin', '--wait', '5', '--json')
        case.check(value['count'] == 1 and value['complete'], 'same-origin redirects remain usable', json.dumps(value))
        request_credentials(case, http); secret_free(case)


def children_argv(parent):
    children = set()
    for path in Path(f'/proc/{parent}/task').glob('*/children'):
        try: children.update(map(int, path.read_text().split()))
        except FileNotFoundError: pass
    result = []
    for child in children:
        try: result.append(Path(f'/proc/{child}/cmdline').read_bytes().decode().replace('\0', ' '))
        except FileNotFoundError: pass
    return result


def torznab_key_not_logged(binary, systemd):
    with owned('torznab_key_not_logged', binary, systemd) as (case, box, h, http, config, secret), d.ApiConn(h) as c:
        c.hello()
        fixtures.path_shim('python3', 'exit 99', box.root)
        fixture = d.recheck_local_fixture(box)
        token = urllib.parse.quote_plus(KEY)
        xml = '<rss><channel><item><title>Private '+KEY.replace('&', '&amp;')+'</title><enclosure url="{base}/payload.torrent?apikey='+token+'" length="262144"/><comments>{base}/details?apikey='+token+'</comments></item></channel></rss>'
        (box.root / 'query.json').write_text(json.dumps(dict(xml=xml)))
        http.routes['/payload.torrent'] = dict(file=str(fixture.torrent.relative_to(box.root)))
        entered, release = threading.Event(), threading.Event()
        original = http.server.RequestHandlerClass.do_GET
        def held(handler):
            if urllib.parse.urlsplit(handler.path).path == '/api':
                entered.set(); release.wait(5)
            original(handler)
        http.server.RequestHandlerClass.do_GET = held
        output = case.path.with_suffix('.cli.stdout.log')
        with output.open('w') as stdout:
            process = case.popen([str(binary), 'search', 'credential fixture', '--wait', '8', '--json'], env=box.env, cwd=box.root, stdout=stdout)
        try:
            case.check(entered.wait(5), 'native HTTP worker reaches the local credential endpoint')
            argv = children_argv(h.daemon_pid())
            case.check(bool(argv), 'native worker argv is observable while its real HTTP request is held', json.dumps(argv))
            secret_free(case, *argv)
        finally: release.set()
        code = process.wait(timeout=15); value = json.loads(output.read_text())
        case.check(code == 0 and value['count'] == 1 and value['complete'] and '__FBT_TORZNAB_KEY__' in value['results'][0]['url'],
                   'native-only search needs no Python and exposes a redacted usable download reference', output.read_text())
        item = value['results'][0]
        pending = c.call('search.grab', dict(result=item, stopped=True, save_path=str(fixture.files[0].parent)), ref='private-grab')['result']
        outcome = d.mutation_outcome(c, pending['ref'])
        case.check(outcome['result']['hash'] == fixture.info_hash, 'the native worker redeems the redacted reference and adds exact metainfo', json.dumps(outcome))
        case.check(not (box.root / 'shims/python3.log').exists(), 'Torznab-only query/grab never invokes Python')
        request_credentials(case, http)
        cli(case, box, binary, 'config', '--json')
        # Invalid inline credentials are deliberately sent on stdin/socket, never
        # printed by the request-logging helper or placed on a process argv.
        identifier=c.next_id; c.next_id+=1
        c.send_frame(dict(id=identifier,op='search.install',args=dict(sources=['https://indexer.invalid/api?apikey='+token]),ref='refused-inline'))
        pending=c.response(identifier)['result']
        refused=d.mutation_outcome(c,pending['ref'])
        case.check(not refused['result']['installed'] and len(refused['result']['failed'])==1
                   and '__FBT_TORZNAB_KEY__' in refused['result']['failed'][0]['source'],
                   'a refused inline credential is redacted from failure source and message',json.dumps(refused))
        secret_free(case, output.read_text(), config.read_text())


def torznab_boundaries(binary, systemd):
    with owned('torznab_boundaries', binary, systemd) as (case, box, h, http, config, secret), d.ApiConn(h) as c:
        c.hello()
        entered, release = threading.Event(), threading.Event()
        original = http.server.RequestHandlerClass.do_GET
        def held(handler):
            entered.set(); release.wait(5); original(handler)
        http.server.RequestHandlerClass.do_GET = held
        identifier = c.call('search.start', dict(query='held native request'))['result']['search_id']
        try:
            case.check(entered.wait(5), 'real native request is blocked at the owned fixture')
            parent = h.daemon_pid()
            workers = set()
            for path in Path(f'/proc/{parent}/task').glob('*/children'):
                try: workers.update(map(int, path.read_text().split()))
                except FileNotFoundError: pass
            began = time.monotonic(); c.call('app.info')
            case.check(time.monotonic()-began < 2, 'held native HTTP does not block API dispatch')
            began = time.monotonic(); c.call('search.stop', dict(search_id=identifier))
            case.check(bool(workers) and all(not d.process_alive(pid) for pid in workers) and time.monotonic()-began < 2,
                       'search.stop kills and reaps every owned native worker within two seconds', json.dumps(dict(workers=sorted(workers), elapsed_s=time.monotonic()-began)))
        finally: release.set()
        http.server.RequestHandlerClass.do_GET = original
        oversized = b'<rss><channel>' + b' '*(4*1024*1024) + b'</channel></rss>'
        http.routes['/api'] = dict(base64=base64.b64encode(oversized).decode())
        cli(case, box, binary, 'search', 'bounded XML body', '--wait', '5', '--json', code=5)
        case.check(len(oversized)>4*1024*1024, 'real HTTP response over the 4 MiB cap is refused')
        before=len(http.requests)
        c.call('app.settings.set',dict(network_interface='fbmissing0'))
        h.wait_for(lambda:((info:=c.call('app.info')['result'])['state']=='parked',json.dumps(info)),'missing selected interface parks the isolated owner')
        cli(case,box,binary,'search','must not fall back to ordinary HTTP','--wait','5','--json',code=5)
        case.check(len(http.requests)==before,'native query never retries on an ordinary socket when its selected interface is absent',json.dumps(dict(before=before,after=len(http.requests))))
        secret_free(case)


def torznab_tui_kind(binary, systemd):
    import native
    with owned('torznab_tui_kind', binary, systemd) as (case, box, h, http, config, secret):
        ui=native.AttachedTui(case,str(binary),box,h,session='fbt-fbt-p7-01-cje')
        try:
            target,done=ui.start()
            ui.keys(target,'2');ui.keys(target,'Escape',literal=False);ui.keys(target,'P')
            ui.assert_pane(target,'Search providers')
            ui.assert_pane(target,'torznab ·')
            capture=case.path.with_suffix('.pane.txt');capture.write_text(ui.capture(target))
            case.step('retain the real providers modal','Torznab kind appears in the attached terminal').ok(str(capture),[capture])
            ui.keys(target,'Escape',literal=False);ui.keys(target,'q')
            h.wait_for(lambda:(done.exists() and done.read_text()=='0',done.read_text() if done.exists() else 'running'),'q closes only this TUI')
            with d.ApiConn(h) as c:
                c.hello();case.check(c.call('app.info')['result']['state']=='running','closing providers TUI leaves its daemon owner running')
            secret_free(case,capture.read_text())
        finally:ui.close()


def torznab_localhost(binary, systemd):
    with owned('torznab_localhost', binary, systemd) as (case, box, h, http, config, secret):
        configure(box,http.base.replace('127.0.0.1','localhost')+'/api')
        value,_=cli(case,box,binary,'search','localhost fixture','--wait','5','--json')
        case.check(value['count']==1 and value['complete'],'keyed localhost HTTP resolves directly to loopback with interface binding',json.dumps(value))
        request_credentials(case,http);secret_free(case)


CASES = (torznab_results, torznab_http_key_refused, torznab_redirect_refused, torznab_key_not_logged, torznab_boundaries, torznab_tui_kind, torznab_localhost)
