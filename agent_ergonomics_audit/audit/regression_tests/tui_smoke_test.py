#!/usr/bin/env python3
"""Force an actual terminal check red and verify pane evidence and final status."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

import fbtlog
from fbtlog import Case,REPO,suite_status


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary',nargs='?',type=Path,default=REPO/'target/debug/fbittorrent')
    parser.add_argument('--log-root',type=Path)
    args=parser.parse_args()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    with Case('tui_smoke_control','forced_pane')as case:
        root=case.suite.directory/'fixtures/forced'
        command=[sys.executable,str(Path(__file__).with_name('tui_smoke.py')),str(args.binary.resolve()),
                 '--case','layout_80x24','--log-root',str(root)]
        result=subprocess.run(command,env=dict(os.environ,FBT_TEST_FORCE_FAIL='tui_smoke/layout_80x24'),
                              capture_output=True,text=True,timeout=90)
        stdout=case.path.with_suffix('.child.stdout.log');stdout.write_text(result.stdout)
        stderr=case.path.with_suffix('.child.stderr.log');stderr.write_text(result.stderr)
        case.step('run native terminal with its first pane check forced red','child exits 1 and keeps both logs').close(
            result.returncode==1,f'exit={result.returncode}',[stdout,stderr])
        path=root/'tui_smoke/layout_80x24.jsonl'
        rows=[json.loads(line)for line in path.read_text().splitlines()]
        failed=[row for row in rows if not row.get('ok',True)and not row.get('final')]
        case.check(len(failed)==1 and failed[0]['step']==1 and rows[-1]['failed']==[1] and not rows[-1]['ok'],
                   'one forced UI failure remains red in the final record',json.dumps(rows[-1]))
        first=failed[0]
        artifacts=list(map(Path,first['artifacts']))
        panes=[p for p in artifacts if p.name.endswith('.pane.txt')]
        case.step('inspect failed step pane','observed and artifact retain the real list, state cells and full frame').close(
            len(panes)==1 and panes[0].exists()and panes[0].read_text()in first['observed']and
            all(text in first['observed']for text in('Torrents (3)','STATE','↓ Down','✓ Done','◌ Meta'))and
            len(panes[0].read_text().splitlines())==24,first['observed'],[path,*panes])
        case.check(len(rows)>25 and all(row.get('artifacts')and row['observed']for row in rows[:-1])and
                   all(row['ok']for row in rows[1:-1]),
                   'later steps continue through q and confirmed Q, each retaining a pane',f'{len(rows)-1} UI steps; final={rows[-1]}')
        setup=json.loads((root/'tui_smoke_setup/summary.json').read_text())
        case.check(setup['failed']==0,'real seed/daemon setup and cleanup remain green',json.dumps(setup))
        summary=subprocess.run([sys.executable,str(REPO/'scripts/testlog-summary.py'),str(root)],
                               capture_output=True,text=True,timeout=5)
        case.check(summary.returncode==1 and 'tui_smoke' in summary.stdout,
                   'summary rejects the failed step and exits 1',summary.stdout+summary.stderr)
    return suite_status('tui_smoke_control')


if __name__=='__main__':sys.exit(main())
