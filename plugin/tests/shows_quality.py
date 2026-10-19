#!/usr/bin/env python3
"""S02: real quality/filter controls and notification-to-inbox in private state."""
import argparse,json,os,shlex,shutil,subprocess,time,traceback
from pathlib import Path
from dashboard import Session
import daemon,fixtures,daemon_notify as notify
from fbtlog import Case,suite_status
IMDB='tt1234001'

def run(binary):
    with Case('qs_shows_quality','journey') as case,fixtures.Sandbox() as box:
        notify.config(box,notify=True);notify.shims(box,action='window')
        fixtures.path_shim('omarchy-shell','exec /usr/share/omarchy/bin/omarchy-shell "$@"',box.root)
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        (cache/(IMDB+'.json')).write_text(json.dumps(dict(id=IMDB,name='Owned Quality Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')))
        rows=[dict(id=i,imdb_id=IMDB[2:],title=f'Owned.Show.S01E{episode:02}.{quality}',season='1',episode=str(episode),seeds=seeds,size_bytes='262144',date_released_unix=1792274400+episode,magnet_url='magnet:?xt=urn:btih:'+f'{i:040x}') for i,(episode,quality,seeds) in enumerate([(1,'1080p',99),(1,'2160p',1),(2,'720p',1),(3,'480p',1)],1)]
        def dataset(): (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=len(rows),limit=100,page=1,torrents=rows)))
        dataset()
        with fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
            box.env.update(FBT_EZTV_BASE_URL=http.base,FBT_IMDB_BASE_URL=http.base)
            box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
            q=Session(case,box);keyboard=subprocess.Popen(['wtype','-s','600000'],env=q.env)
            box.env.update(OMARCHY_PATH=str(box.root),WAYLAND_DISPLAY=q.env['WAYLAND_DISPLAY']);box.keys+=['OMARCHY_PATH','WAYLAND_DISPLAY']
            manifest=Path(box.env['XDG_CONFIG_HOME'])/'omarchy/plugins/oliver.torrents/manifest.json';manifest.parent.mkdir(parents=True);shutil.copyfile(q.config/'Plugin/manifest.json',manifest)
            try:
                with daemon.Harness(case,binary,box,False) as h:
                    process=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
                    with daemon.ApiConn(h) as c:
                        c.hello();c.call('subscribe',dict(topics=['events']))
                        def call(op,args):
                            result=c.call(op,args)['result']
                            if result.get('state')=='pending':return daemon.mutation_outcome(c,result['ref']).get('result',{})
                            return result
                        def show():return c.call('shows.list')['result']['shows'][0]
                        def view():return q.state().get('view') or {}
                        def state():return view().get('shows') or {}
                        def keys(args,predicate):q.compositor.ipc('[title="fbittorrent"] focus');return q.keys(args,predicate)
                        def dropdown(name,args,predicate):
                            assert q.ipc('showsDropdown',name)=='true'
                            q.wait(lambda:view().get('controls',{}).get(name+'Open'))
                            keys(args,predicate)
                        call('shows.follow',dict(imdb=IMDB));q.wait(lambda:q.ipc('ready')=='true')
                        # The actual notification action invokes the real binary and
                        # installed shell CLI, addressed only at this owned shell.
                        call('shows.check',dict(imdb=IMDB));q.wait(lambda:view().get('window',{}).get('view')=='shows',timeout=10)
                        q.wait(lambda:len(state().get('rows',[]))==3)
                        assert len(notify.calls(box))==1 and 'window=Open Shows' in notify.calls(box)[0]
                        case.check(True,'new-release notification action opens the actual populated Shows inbox',json.dumps(state()))
                        keys(['='],lambda:view().get('controls',{}).get('preferenceOpen'))
                        keys(['j','-k','Return'],lambda:show().get('quality')=='2160p')
                        keys(['r'],lambda:any(row['key']=='S01E01' and row['quality']=='2160p' for row in state().get('rows',[])))
                        call('shows.set_read',dict(imdb=IMDB,key='S01E02',read=True));q.wait(lambda:len(state().get('rows',[]))==2)
                        dropdown('read',['j','-k','Return'],lambda:state().get('readFilter')=='all' and len(state().get('rows',[]))==3)
                        dropdown('read',['j','-k','Return'],lambda:state().get('readFilter')=='read' and len(state().get('rows',[]))==1)
                        dropdown('quality',['jjj','-k','Return'],lambda:state().get('qualityFilter')=='720p' and len(state().get('rows',[]))==1)
                        dropdown('quality',['kk','-k','Return'],lambda:state().get('qualityFilter')=='2160p' and not state().get('rows'))
                        dropdown('read',['k','-k','Return'],lambda:state().get('readFilter')=='all' and len(state().get('rows',[]))==1)
                        case.check(True,'actual dropdowns filter read state and release quality independently',json.dumps(state()))
                        keys(['='],lambda:view().get('controls',{}).get('preferenceOpen'));keys(['k','-k','Return'],lambda:'quality' not in show())
                        assert c.call('torrents.list',{})['result']['torrents']==[], 'quality, filters and checks never download'
                        # An explicit visible check with a new key is silent.
                        rows.append(dict(rows[-1],id=5,title='Owned.Show.S01E04.1080p',episode='4',date_released_unix=1792274404));dataset()
                        call('shows.check',dict(imdb=IMDB));q.wait(lambda:show()['counts']['episodes']==4)
                        assert len(notify.calls(box))==1
                        q.shell('hide');q.wait(lambda:q.ipc('pendingRequests')=='0')
                        call('shows.check',dict(imdb=IMDB));time.sleep(1)
                        assert len(notify.calls(box))==1,'unchanged hidden check must not notify'
                        cli=subprocess.run([str(binary),'shows','quality',IMDB,'720p','--json'],env=box.env,capture_output=True,text=True,timeout=8)
                        assert cli.returncode==0 and show().get('quality')=='720p',cli.stderr
                        q.shell('summon','{"view":"shows"}')
                        q.wait(lambda:'shows' in view())
                        keys(['hj'],lambda:state().get('selected')==IMDB)
                        q.wait(lambda:view().get('controls',{}).get('preference')=='720p')
                        q.shell('hide')
                        socket='fbt-fbt-p3-18-3pe';session=socket;env=dict(box.env,FBITTORRENT_NO_START='1')
                        def tmux(*args,check=True):return subprocess.run(['tmux','-L',socket,*args],env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=check,timeout=4)
                        def capture():return tmux('capture-pane','-p','-t',session).stdout
                        def wait(predicate):
                            deadline=time.monotonic()+10
                            while time.monotonic()<deadline:
                                if predicate():return
                                time.sleep(.03)
                            raise TimeoutError(capture())
                        def text(value):tmux('send-keys','-t',session,'-l',value)
                        try:
                            tmux('new-session','-d','-s',session,'-x','120','-y','30',str(binary),'tui','--shows')
                            wait(lambda:'(y) Copy magnet' in capture());text('=');wait(lambda:'Show quality:' in capture())
                            tmux('send-keys','-t',session,'C-a');text('480p');tmux('send-keys','-t',session,'Enter');wait(lambda:show().get('quality')=='480p')
                            text('F');wait(lambda:'Inbox filter:' in capture());tmux('send-keys','-t',session,'C-a');text('read 720p');tmux('send-keys','-t',session,'Enter');wait(lambda:'read 720p' in capture() and 'S01E02' in capture())
                            case.check(True,'TUI = saves quality and F filters the real read history',capture())
                            text('q');wait(lambda:tmux('has-session','-t',session,check=False).returncode!=0)
                        finally:tmux('kill-server',check=False)
                        c.close();h.stop_serve(process);h.notify.close();process=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
                        with daemon.ApiConn(h) as restored:
                            restored.hello();saved=restored.call('shows.list')['result']['shows'][0]
                            assert saved.get('quality')=='480p' and restored.call('torrents.list',{})['result']['torrents']==[]
                            case.check(True,'dashboard, CLI and TUI preference survives daemon restart without downloading',json.dumps(saved))
                    h.stop_serve(process);h.lock_free()
            except Exception:
                print(json.dumps(q.state()),flush=True);traceback.print_exc();raise
            finally:keyboard.terminate();keyboard.wait(timeout=3);q.close()
    return suite_status('qs_shows_quality')
if __name__=='__main__':
    import private_net
    private_net.enter();parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True,type=Path)
    raise SystemExit(run(parser.parse_args().binary.resolve()))
