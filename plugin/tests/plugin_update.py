#!/usr/bin/env python3
"""Setup replaces cached dashboard code by restarting only an owned private shell."""
import argparse,http.server,json,os,shlex,shutil,subprocess,threading
from pathlib import Path
from dashboard import Session
import fixtures,daemon,desktop_setup
from fbtlog import Case,suite_status

def run(binary):
    with Case('qs_plugin_update','reload') as case,fixtures.Sandbox() as box,daemon.Harness(case,binary,box,False) as h:
        desktop_setup.prepare(box)
        def setup(mode='copy',expected=0):
            return desktop_setup.call(case,binary,box,'--plugin='+mode,expected=expected)
        setup();config=Path(box.env['XDG_CONFIG_HOME']);installed=config/'omarchy/plugins/oliver.torrents'
        stamp=config/'fbittorrent/shell-plugin.revision'
        dashboard=installed/'Dashboard.qml';current=dashboard.read_text()
        dashboard.write_text(current.replace('  id:root','  id:root;objectName:"stale-installed-code"',1))
        process=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
        q=Session(case,box,recipe='bar-host',source_link=installed)
        restarts=[]
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self,*_):pass
            def do_POST(self):
                try:
                    old=q.child.pid
                    subprocess.run(['qs','kill','--pid',str(old)],env=q.env,check=True,capture_output=True,timeout=3);q.child.wait(timeout=3)
                    q.child=subprocess.Popen(['qs','--no-color','-p',str(q.config)],env=q.env,stdout=q.output,stderr=subprocess.STDOUT)
                    q.wait(lambda:q.ipc('ready')=='true');restarts.append((old,q.child.pid));self.send_response(200);self.end_headers();self.wfile.write(b'ok')
                except Exception as error:self.send_error(500,str(error))
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        url='http://127.0.0.1:'+str(server.server_port)
        script='import urllib.request;urllib.request.urlopen('+repr(url)+',data=b"restart",timeout=8).read()'
        def restore_restart():fixtures.path_shim('omarchy-restart-shell','exec python3 -c '+shlex.quote(script),box.root)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{}');q.wait(lambda:q.state()['code']=='stale-installed-code')
            with daemon.ApiConn(h) as client:
                client.hello();leftover='f'*40
                client.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+leftover),stage=True))
            restore_restart();setup();assert dashboard.read_text()==current and stamp.exists()
            q.shell('summon','{}');fresh=q.wait(lambda:s if (s:=q.state())['code']=='' and s.get('view') else None)
            case.check(len(restarts)==1 and restarts[0][0]!=restarts[0][1],'copy repairs changed files, restarts the owned shell, and opens new dashboard code',json.dumps(dict(restarts=restarts,state=fresh)))
            source='magnet:?xt=urn:btih:'+leftover+'&dn=Review%20after%20reload'
            q.shell('summon',json.dumps(dict(view='torrents',add=dict(source=dict(magnet=source),origin='handler'))))
            q.wait(lambda:a if (a:=((q.state().get('view') or {}).get('add') or {})).get('owned') and a.get('canDownload') and a.get('field')=='sheet' else None)
            q.compositor.ipc('[title="fbittorrent"] focus')
            q.keys(['-k','Return'],lambda:not ((q.state().get('view') or {}).get('add') or {}).get('visible'))
            with daemon.ApiConn(h) as client:
                client.hello();row=client.call('torrents.get',dict(hash=leftover))['result']['torrent']
                case.check(not row['staged'] and row['intent']=='run','a stage left by the old shell accepts one new review and Enter commits it',json.dumps(row))
            pid=q.child.pid;assert setup()['actions']==[] and q.child.pid==pid
            dashboard.write_text(current+'\n// changed installed copy\n');fixtures.path_shim('omarchy-restart-shell','exit 7',box.root)
            setup(expected=1);assert not stamp.exists()
            restore_restart();setup();assert stamp.exists() and len(restarts)==2
            setup('link');assert installed.is_symlink() and len(restarts)==3
            q.shell('summon','{}');q.wait(lambda:q.state()['code']=='')
            assert setup('link')['actions']==[]
            assert process.child.poll() is None,'plugin reload never restarts the torrent daemon'
        finally:
            server.shutdown();server.server_close();thread.join(timeout=2);q.close()
    return suite_status('qs_plugin_update')
if __name__=='__main__':
    import private_net
    private_net.enter();p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',required=True,type=Path);a=p.parse_args();raise SystemExit(run(a.binary.resolve()))
