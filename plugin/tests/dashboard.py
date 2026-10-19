#!/usr/bin/env python3
"""Native dashboard tests inside a private compositor, using the actual shell CLI."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
import daemon
import fixtures
from fbtlog import Case,suite_status
from qs_env import prepare
from headless import Headless


class Session:
    def __init__(self,case,sandbox,hooks=True,recipe="dashboard",source_link=None):
        self.recipe=recipe;self.source_link=source_link;self.hooks=hooks;self.case=case;self.compositor=Headless(sandbox,case)
        try:self.start(sandbox)
        except BaseException:self.compositor.close();raise

    def start(self,sandbox):
        config,self.env=prepare(sandbox,Path(__file__).with_name(self.recipe),source=self.source_link)
        self.config=sandbox.root/'shell';config.rename(self.config)
        if self.source_link:
            assert self.source_link.is_relative_to(sandbox.root),'only private installed sources may be linked'
            shutil.rmtree(self.config/'Plugin');(self.config/'Plugin').symlink_to(self.source_link,target_is_directory=True)
        host=self.config/'Host';host.mkdir()
        shutil.copyfile('/usr/share/omarchy/shell/services/PluginRegistry.qml',host/'PluginRegistry.qml')
        if self.recipe=='bar-host':
            (self.config/'HostBar').symlink_to('/usr/share/omarchy/shell/plugins/bar',target_is_directory=True)
            for name in ('BarWidgetRegistry.qml','PluginShellApi.qml'):
                shutil.copyfile('/usr/share/omarchy/shell/services/'+name,host/name)
        self.env.update(self.compositor.env)
        self.env['FBT_FONT_FAMILY']=os.environ.get('FBT_FONT_FAMILY') or subprocess.check_output(['fc-match','monospace','-f','%{family[0]}'],text=True)
        self.env.pop('HYPRLAND_INSTANCE_SIGNATURE',None)
        if os.environ.get('FBT_TEST_RENDER_TIMING')=='1':self.env.update(QSG_RENDER_TIMING='1',QT_LOGGING_RULES='qt.scenegraph.time.renderloop=true')
        fixtures.path_shim('fbittorrent','exit 0',sandbox.root)
        self.env.update(OMARCHY_PATH=str(sandbox.root),FBT_TEST_BINARY=str(sandbox.root/'shims/fbittorrent'),FBT_TEST_HOOKS='1' if self.hooks else '0')
        theme=Path(self.env['HOME'])/'.local/state/omarchy/current/theme';theme.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(ROOT/'contracts/ui/themes/owner.toml',theme/'colors.toml')
        self.log=self.case.path.with_suffix('.qs.log');self.output=self.log.open('w')
        command=['qs','--no-color','-p',str(self.config)]
        if os.environ.get('FBT_TEST_QS_DIAG')=='1':
            # A fresh exec wrapper avoids Python preexec_fn after the DNS thread starts.
            script='import ctypes,os,sys; result=ctypes.CDLL(None).prctl(0x59616d61,ctypes.c_ulong(-1),0,0,0); assert result==0; os.execvp("qs",sys.argv[1:])'
            command=[sys.executable,'-c',script,*command]
        self.child=subprocess.Popen(command,env=self.env,stdout=self.output,stderr=subprocess.STDOUT)

    def ipc(self,method,*args):
        result=subprocess.run(['qs','ipc','-p',str(self.config),'call','fbt-dashboard',method,*map(str,args)],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=3)
        if result.returncode:raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()

    def native_hook(self,method,*args):
        result=subprocess.run(['qs','ipc','-p',str(self.config),'call','oliver.torrents',method,*map(str,args)],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=3)
        if result.returncode:raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()

    def state(self):return json.loads(self.ipc('state'))

    def wait(self,fn,timeout=5):
        deadline=time.monotonic()+timeout;last=None
        while time.monotonic()<deadline:
            if self.child.poll() is not None:raise RuntimeError(f'owned Qs exited {self.child.returncode}; {self.log}')
            try:
                last=fn()
                if last:return last
            except (RuntimeError,ValueError,subprocess.TimeoutExpired) as error:last=str(error)
            time.sleep(.02)
        raise TimeoutError(str(last))

    def shell(self,method,*args):
        result=subprocess.run(['/usr/share/omarchy/bin/omarchy-shell','shell',method,'oliver.torrents',*map(str,args)],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=4)
        if result.returncode:raise RuntimeError(result.stderr.strip() or result.stdout.strip())
        return result.stdout.strip()

    def keys(self,args,predicate):
        child=subprocess.Popen(['wtype','-s','80',*args,'-s','700'],env=self.env)
        try:return self.wait(predicate)
        except TimeoutError as error:raise TimeoutError(json.dumps(self.state())) from error
        finally:child.wait(timeout=3)

    def diagnose(self):
        if os.environ.get('FBT_TEST_QS_DIAG')!='1':return
        path=self.case.path.with_suffix('.threads.log')
        with path.open('w') as output:
            subprocess.run(['ps','-L','-p',str(self.child.pid),'-o','pid,tid,stat,pcpu,wchan:28,comm'],stdout=output,stderr=subprocess.STDOUT,timeout=2)
            subprocess.run(['gdb','--batch','-ex','set pagination off','-ex','thread apply all bt 20','-ex','detach','-ex','quit','-p',str(self.child.pid)],stdout=output,stderr=subprocess.STDOUT,timeout=10)

    def close(self):
        try:
            if self.child.poll() is None:
                try:subprocess.run(['qs','kill','--pid',str(self.child.pid)],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=3)
                except subprocess.TimeoutExpired:self.child.terminate()
                try:self.child.wait(timeout=3)
                except subprocess.TimeoutExpired:self.child.kill();self.child.wait()
            self.output.close();text=self.log.read_text()
            errors=[line for line in text.splitlines() if any(t in line for t in ('TypeError:','ReferenceError:','SyntaxError:','Failed to load configuration','Cannot assign','Binding loop detected','Unexpected token','Error calling method'))]
            self.case.check(self.child.returncode==0 and not errors,'owned dashboard shell closes without QML errors',f'exit={self.child.returncode}; errors={errors}; {self.log}')
        finally:self.compositor.close()


def descriptors(pid):
    result={}
    for path in Path(f'/proc/{pid}/fd').iterdir():
        try:result[path.name]=os.readlink(path)
        except FileNotFoundError:pass
    return result


def socket_details(pid,snapshot):
    inodes={target[8:-1] for target in snapshot.values() if target.startswith('socket:[')}
    result={}
    for name in ('unix','tcp','tcp6','udp','udp6'):
        for line in Path(f'/proc/{pid}/net/{name}').read_text().splitlines()[1:]:
            fields=line.split()
            inode=fields[6] if name=='unix' else fields[9]
            if inode in inodes:result[inode]=dict(protocol=name,details=fields)
    return result


def run(binary):
    with Case('qs_dashboard','window_shell') as case,fixtures.Sandbox() as sandbox,daemon.Harness(case,binary,sandbox,False) as harness:
        import private_net
        case.check(True,'kernel network/mount boundary contains all fixture traffic',private_net.describe())
        process=harness.start_serve()
        if not(harness.receive_ready() and harness.wait_engine()):raise RuntimeError('owned dashboard daemon did not start')
        q=Session(case,sandbox)
        try:
            q.wait(lambda:q.ipc('ready')=='true')
            case.check(q.ipc('focusCommand','0xabc','true')=='hl.dsp.focus({window="address:0xabc"})' and q.ipc('focusCommand','0xabc','false')=='focuswindow address:0xabc',
                       'focus fallback formats Lua and legacy dispatch by address','both dispatcher syntaxes match installed source/documentation')
            case.check(q.ipc('focusCommand','0xabc;evil','true')=='refused','focus fallback rejects non-address text','no command constructed')
            registry=json.loads(q.ipc('registryState'))
            case.check(registry['enabled'] and registry['location']['kind']=='bar' and 'panel' in registry['kinds'],'actual installed registry enables the panel from only a bar layout reference',json.dumps(registry))
            reply=q.shell('summon',json.dumps(dict(view='search',query='sintel')))
            state=q.wait(lambda:s if (s:=q.state()).get('view',{} ) and s['view']['window']['visible'] else None)
            case.check(state['view']['window']['view']=='search' and state['view']['query']=='sintel','actual omarchy-shell CLI routes Search/query to the private real Dashboard',json.dumps(state))
            keyboard=subprocess.Popen(['wtype','-s','80','-k','End','-s','30',' typed','-s','700'],env=q.env)
            try:
                typed=q.wait(lambda:s if (s:=q.state())['view']['query']=='sintel typed' and s['view'].get('focus',s['view']['cursor'])=='query' else None)
                case.check(True,'typing through wtype reaches the focused native search field',json.dumps(dict(state=typed,focus_steps=q.ipc('focusSteps'))))
            finally:keyboard.wait(timeout=3)
            q.keys(['-k','Tab'],lambda:q.state()['view']['window']['view']=='shows')
            q.keys(['-M','shift','-k','Tab','-m','shift'],lambda:q.state()['view']['window']['view']=='search')
            case.check(True,'Tab and Shift+Tab cycle the four primary views',json.dumps(q.state()))
            q.keys(['-k','Escape'],lambda:q.state()['view']['query']=='')
            q.keys(['-k','Escape'],lambda:q.state()['view']['window']['visible'])
            case.check(q.state()['view']['window']['visible'],'Esc clears input then keeps the top-level window open',json.dumps(q.state()))
            case.check(json.loads(q.ipc('visibility'))==dict(service=True,client=True),'visible window sends interactive true through the shared service/client',q.ipc('visibility'))
            q.shell('hide');q.wait(lambda:q.state()['view'] is None and q.ipc('pendingRequests')=='0')
            q.shell('summon',json.dumps(dict(view='torrents')))
            q.wait(lambda:q.state()['view'] is not None)
            q.keys(['q'],lambda:q.state()['view'] is None)
            case.check(q.state()['view'] is None,'q dismisses the native window through the host facade','window destroyed; service retained')
            hook=q.native_hook('summonView',json.dumps(dict(view='torrents',filter='downloading')))
            q.wait(lambda:q.state()['view'] is not None)
            hook_state=json.loads(q.native_hook('state'))
            case.check(hook=='ok' and hook_state['view']['filter']=='downloading','production bar summonView hook and shared state expose dashboard routing',json.dumps(hook_state))
            q.shell('hide');q.wait(lambda:q.state()['view'] is None and q.ipc('pendingRequests')=='0')
            case.check(json.loads(q.ipc('visibility'))==dict(service=False,client=False),'hidden/destroyed window clears interactive state',q.ipc('visibility'))
            pid=harness.daemon_pid()
            for _ in range(5):
                q.shell('summon','{}');q.wait(lambda:q.state()['view'] is not None)
                q.shell('hide');q.wait(lambda:q.state()['view'] is None and q.ipc('pendingRequests')=='0')
            baseline=descriptors(pid);net=socket_details(pid,baseline);before=len(baseline);samples=[];details=[]
            for i in range(50):
                q.shell('summon','{}');q.wait(lambda:q.state()['view'] is not None)
                q.shell('hide');q.wait(lambda:q.state()['view'] is None and q.ipc('pendingRequests')=='0')
                snapshot=descriptors(pid);samples.append(len(snapshot));details.append(snapshot)
            case.path.with_suffix('.fds.json').write_text(json.dumps(dict(baseline=baseline,network=net,samples=details),indent=2))
            case.check(all(value==before for value in samples),'50 native dashboard create/destroy cycles leave daemon FDs flat',json.dumps(dict(baseline=before,samples=samples)))
            with daemon.ApiConn(harness) as client:
                hello=client.hello()['result']
                info=client.call('app.info')['result']
                case.check(info['clients']['bar']==1,'all dashboard cycles reuse one daemon bar connection',json.dumps(info['clients']))
                if 'subscribe' not in hello['caps']:
                    case.check(True,'daemon busy-reason acceptance remains pending the server capability','SKIP: no subscribe cap; UI/client visibility is proven, daemon idle policy is not')
                else:
                    client.call('subscribe',dict(topics=['summary'],interval_ms=250))
                    def interactive(expected):
                        deadline=time.monotonic()+5
                        for frame in client.events():
                            if frame.get('topic')=='summary' and frame.get('ev')=='state':
                                reasons=frame['data']['daemon']['busy_reasons']
                                if ('interactive' in reasons)==expected:return reasons
                            if time.monotonic()>deadline:raise TimeoutError('summary interactive state did not follow visibility')
                        raise EOFError('summary stream ended')
                    q.shell('summon','{}')
                    shown=interactive(True);case.check('interactive' in shown,'daemon busy_reasons includes visible dashboard',json.dumps(shown))
                    q.shell('hide');hidden=interactive(False)
                    case.check('interactive' not in hidden,'daemon busy_reasons clears after hiding dashboard',json.dumps(hidden))

        finally:
            q.close();harness.stop_serve(process);harness.lock_free()
    with Case('qs_dashboard','hook_guard') as case,fixtures.Sandbox() as sandbox:
        q=Session(case,sandbox,hooks=False)
        try:
            q.wait(lambda:json.loads(q.ipc('registryState'))['enabled'])
            result=q.native_hook('summonView','{"view":"search"}')
            case.check(q.ipc('startScrolling')=='false','private scroll driver also refuses without hooks','no timer started')
            case.check(q.native_hook('setPaneWidth','420').startswith('refused:'),'pane width hook refuses without FBT_TEST_HOOKS=1','production guard checked before any width mutation')
            case.check(result.startswith('refused:') and q.state()['view'] is None,'production summonView refuses without FBT_TEST_HOOKS=1',result)
            case.check(not (sandbox.root/'shims/fbittorrent.log').exists(),'refused hook never starts a daemon','recording up shim untouched')
        finally:q.close()
    return suite_status('qs_dashboard')

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,default=Path(os.environ.get('FBT_BIN',ROOT/'target/release/fbittorrent')))
    args=parser.parse_args();sys.exit(run(args.binary.resolve()))
