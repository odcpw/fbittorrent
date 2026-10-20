#!/usr/bin/env python3
"""Actual installed Bar.qml host + capability facade in private Sway/XDG/network."""
import argparse,json,shutil,subprocess,time
from pathlib import Path
from dashboard import Session,ROOT
import fixtures,daemon
from fbtlog import Case,suite_status

def run(binary,baseline):
    with Case('qs_bar_host','baseline' if baseline else 'visible') as case,fixtures.Sandbox() as box,daemon.Harness(case,binary,box,False) as h:
        process=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
        q=Session(case,box,recipe='bar-host')
        try:
            q.wait(lambda:q.ipc('ready')=='true');current=q.state()
            assert current['service'],current
            if baseline:
                case.check(current['width']==0 and current['height']==0,'old widget connects to its service but occupies zero pixels in the installed host',json.dumps(current));return 0
            current=q.wait(lambda:s if (s:=q.state())['width']>0 and s['height']>0 and any(g['visible'] for g in s['geometry']) else None)
            case.check(current['visible'] and current['appearance']['tone']=='normal','actual installed bar gives the running widget nonzero visible geometry and lit glyph',json.dumps(current))
            image=case.path.with_suffix('.png');assert q.ipc('grab',image)=='true';q.wait(lambda:image.exists() and image.stat().st_size>0)
            link=Path(box.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents';link.parent.mkdir(parents=True);link.symlink_to(q.config/'Plugin',target_is_directory=True)
            fixtures.path_shim('omarchy-shell','exec /usr/share/omarchy/bin/omarchy-shell "$@"',box.root)
            for command in (['dashboard'],['up','--ui']):
                q.ipc('popup');q.wait(lambda:q.state()['opened'])
                result=subprocess.run([str(binary),*command,'--json'],env=q.env,capture_output=True,text=True,timeout=6)
                assert result.returncode==0,(result.stdout,result.stderr)
                current=q.wait(lambda:s if (s:=q.state()).get('view',{}).get('window',{}).get('visible') and not s['opened'] else None)
            h.stop_serve(process);q.wait(lambda:not q.state()['online']);assert q.state()['appearance']['tone']!='normal';q.wait(lambda:not q.state()['visible'],timeout=12)
            case.check(q.state()['appearance']['tone']!='normal','stopping the owned daemon unlights and hides the default widget',json.dumps(q.state()))
        finally:q.close()
    return suite_status('qs_bar_host')
if __name__=='__main__':
    import private_net
    private_net.enter()
    old=ROOT/'target/test-logs/qs_bar_host'
    if old.exists():shutil.copytree(old,ROOT/'target/test-logs/qs_bar_host_runs'/str(time.time_ns()))
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',required=True,type=Path);p.add_argument('--baseline',action='store_true');a=p.parse_args();raise SystemExit(run(a.binary.resolve(),a.baseline))
