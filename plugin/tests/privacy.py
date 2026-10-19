#!/usr/bin/env python3
"""Real plugin copy/open processes and RSS error text, private XDG only."""
import json
import os
from pathlib import Path
import subprocess
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'agent_ergonomics_audit/audit/regression_tests'))
import fixtures
from qs_env import prepare
from fbtlog import Case,suite_status

with Case('qs_privacy','clipboard_and_links') as case,fixtures.Sandbox() as box:
    fixtures.path_shim('wl-copy','cat > "$XDG_RUNTIME_DIR/copied"',box.root)
    fixtures.path_shim('xdg-open','exit 0',box.root)
    config,env=prepare(box,Path(__file__).with_name('privacy'))
    with case.path.with_suffix('.qs.log').open('w') as output:
        result=subprocess.run(['qs','--no-color','-p',str(config)],env=env,text=True,stdout=output,stderr=subprocess.STDOUT,timeout=10)
    result.stdout=case.path.with_suffix('.qs.log').read_text()
    case.check(result.returncode==0 and not any(s in result.stdout for s in ('TypeError:','ReferenceError:','Cannot assign','Unable to assign','Binding loop')),'owned shell loads and exits cleanly',result.stdout)
    copies=[json.loads(s) for s in (box.root/'shims/wl-copy.log').read_text().splitlines()]
    text=(Path(env['XDG_RUNTIME_DIR'])/'copied').read_text()
    case.check(copies==[['--']] and text.startswith('magnet:?xt=urn:btih:'+'a'*40) and 'synthetic-passkey' in text,'complete credential-bearing magnet is delivered only over stdin','argv contains only --; stdin content verified')
    opens=[json.loads(s) for s in (box.root/'shims/xdg-open.log').read_text().splitlines()]
    case.check(opens==[['https://example.invalid/title']],'scraped javascript URL is rejected and HTTPS opens',str(opens))
    records=[json.loads(s.split('FBTQS-STEP ',1)[1]) for s in result.stdout.splitlines() if 'FBTQS-STEP ' in s]
    case.check(len(records)==1 and records[0]['ok'],'real RSS row/footer retain the error while redacting credentials',str(records))
sys.exit(suite_status('qs_privacy'))
