#!/usr/bin/env python3
"""Route CLI queries into a real, already-open private dashboard."""
import argparse
import json
from pathlib import Path
import subprocess
import traceback
from journey_support import JourneySession
import daemon
import fixtures
from fbtlog import Case, suite_status


def run(binary):
    with Case('qs_search_route', 'open_dashboard') as case, fixtures.Sandbox() as box:
        runtime = Path(box.env['XDG_DATA_HOME']) / 'fbittorrent/nova3'
        engines = runtime / 'engines'
        engines.mkdir(parents=True)
        queries = runtime / 'queries.jsonl'
        (engines / 'route.py').write_text('''class route:
 name="Route fixture"
 url="http://127.0.0.1"
 def search(self,query):
  import hashlib,json,pathlib,urllib.parse
  from novaprinter import prettyPrinter
  query=urllib.parse.unquote(query)
  with pathlib.Path("queries.jsonl").open("a") as output:output.write(json.dumps(query)+"\\n")
  prettyPrinter(dict(link="magnet:?xt=urn:btih:"+hashlib.sha1(query.encode()).hexdigest(),name="Result "+query,size=4096,seeds=1,leech=0,engine_url=self.url,desc_link=self.url+"/source"))
''')
        with daemon.Harness(case, binary, box, False) as harness:
            harness.start_serve(idle_exit_secs=0)
            if not (harness.receive_ready() and harness.wait_engine()):
                raise RuntimeError('private engine startup failed')
            q = JourneySession(case, box)
            try:
                installed = Path(box.env['XDG_CONFIG_HOME']) / 'omarchy/plugins/oliver.torrents'
                installed.parent.mkdir(parents=True)
                installed.symlink_to(q.config / 'Plugin', target_is_directory=True)
                fixtures.path_shim('omarchy-shell', 'exec /usr/share/omarchy/bin/omarchy-shell "$@"', box.root)
                q.wait(lambda: q.ipc('ready') == 'true')

                def view():
                    return q.state().get('view') or {}

                def received():
                    return [json.loads(line) for line in queries.read_text().splitlines()] if queries.exists() else []

                def route(query, label):
                    command = [str(binary), '--no-start', '--json', 'dashboard', '--view', 'search', '--query', query]
                    child = subprocess.run(command, env=q.env, capture_output=True, text=True, timeout=5)
                    case.check(child.returncode == 0, 'CLI routes ' + label, repr((child.returncode, child.stdout, child.stderr)))
                    def matches():
                        state = view()
                        return (state.get('query') == query and state.get('search', {}).get('mode') == 'query'
                                and state.get('search', {}).get('phase') == 'done'
                                and [row['name'] for row in state.get('rows', [])] == ['Result ' + query]
                                and received().count(query) == 1)
                    try:
                        q.wait(matches)
                        ok = True
                    except TimeoutError:
                        ok = False
                    case.check(ok, label + ' preserves the query and searches it once without typing Enter',
                               json.dumps(dict(view=view(), received=received()), ensure_ascii=False))

                q.shell('summon', '{"view":"torrents"}')
                q.wait(lambda: view().get('rows') == 0)
                route('Routed first', 'from an open Torrents window')
                route('Routed second', 'into the already-mounted Search view')
                q.shell('summon', '{"view":"torrents"}')
                q.wait(lambda: view().get('window', {}).get('view') == 'torrents')
                route('Routed third', 'after returning from a previous Search')
                q.shell('hide')
                q.wait(lambda: q.state().get('view') is None)
                route('Routed café', 'after closing and reopening the dashboard')
                image = case.path.with_suffix('.png')
                q.ipc('grab', image)
                q.wait(lambda: image.exists() and image.stat().st_size > 0)
                with daemon.ApiConn(harness) as client:
                    client.hello()
                    case.check(not client.call('torrents.list', {})['result']['torrents'] and not list(box.downloads.iterdir()),
                               'routing searches adds no torrents or payloads', 'private ledger and Downloads remain empty')
            except Exception:
                print('Search route state at failure: ' + json.dumps(q.state()), flush=True)
                traceback.print_exc()
                raise
            finally:
                q.close()
    return suite_status('qs_search_route')


if __name__ == '__main__':
    import private_net
    private_net.enter()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
