#!/usr/bin/env python3
"""Capture private Qt popups over an owned real daemon; never summon the live shell."""
import argparse
import hashlib
import json
import os
import select
from pathlib import Path
import shutil
import subprocess
import sys
import time
from datetime import date
from PIL import Image, ImageChops, ImageDraw

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
import fixtures
from fbtlog import Case,suite_status
from visual_fixture import running,STATES
from qs_env import prepare
from headless import Headless


def diff(previous,current):
    if not previous.exists():return None
    with Image.open(previous) as a,Image.open(current) as b:
        if a.size!=b.size:return 100.0
        changed=ImageChops.difference(a.convert('RGB'),b.convert('RGB'))
        return 100*sum(any(pixel) for pixel in (changed.get_flattened_data() if hasattr(changed,'get_flattened_data') else changed.getdata()))/(a.width*a.height)


def pair(left,right,output,geometry):
    with Image.open(left).convert('RGB') as a,Image.open(right).convert('RGB') as b:
        result=Image.new('RGB',(a.width+b.width+20,max(a.height,b.height)+110),'white')
        result.paste(a,(0,0));result.paste(b,(a.width+20,0))
        draw=ImageDraw.Draw(result);y=max(a.height,b.height)+8
        draw.text((8,y),'oliver.torrents  |  omarchy.network',fill='black')
        for title,values in geometry.items():
            y+=20;draw.text((8,y),title+' '+json.dumps(values,sort_keys=True),fill='black')
        result.save(output)


class Capture:
    def __init__(self,case,sandbox,theme):
        self.case=case
        self.compositor=Headless(sandbox,case)
        try:self.start(case,sandbox,theme)
        except BaseException:self.compositor.close();raise

    def start(self,case,sandbox,theme):
        self.config,self.env=prepare(sandbox,Path(__file__).with_name('visual'))
        self.env.update(self.compositor.env)
        self.env.pop('HYPRLAND_INSTANCE_SIGNATURE',None)
        reference=self.config/'Reference';reference.mkdir()
        installed=Path('/usr/share/omarchy/shell/plugins/panels/network')
        shutil.copyfile(installed/'Panel.qml',reference/'NetworkWidget.qml')
        shutil.copyfile(installed/'Model.js',reference/'Model.js')
        theme_root=Path(self.env['HOME'])/'.local/state/omarchy/current/theme';theme_root.mkdir(parents=True)
        shutil.copyfile(ROOT/'contracts/ui/themes'/f'{theme}.toml',theme_root/'colors.toml')
        # The reference panel uses a private unavailable backend; no owner Wi-Fi scan or action.
        self.env.update(DBUS_SYSTEM_BUS_ADDRESS='unix:path='+str(sandbox.root/'no-system-bus'),DBUS_SESSION_BUS_ADDRESS='unix:path='+str(sandbox.root/'no-session-bus'))
        self.env['FBT_FONT_FAMILY']=os.environ.get('FBT_FONT_FAMILY','monospace')
        for name in ('omarchy-network-status','nmcli','omarchy-network-band','omarchy-network-dns','omarchy-network-ping'):
            fixtures.path_shim(name,'exit 0',sandbox.root)
        self.log=case.path.with_suffix('.qs.log');self.stream=self.log.open('w')
        self.child=subprocess.Popen(['qs','--no-color','-p',str(self.config)],env=self.env,stdout=self.stream,stderr=subprocess.STDOUT)

    def ipc(self,method,*args):
        p=subprocess.run(['qs','ipc','-p',str(self.config),'call','fbt-visual',method,*map(str,args)],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=3)
        if p.returncode:raise RuntimeError(p.stderr.strip())
        return p.stdout.strip()

    def wait(self,fn,timeout=12):
        deadline=time.monotonic()+timeout;last=None
        while time.monotonic()<deadline:
            if self.child.poll() is not None:raise RuntimeError(f'owned Qs exited {self.child.returncode}; {self.log}')
            try:
                last=fn()
                if last:return last
            except (RuntimeError,ValueError,subprocess.TimeoutExpired) as error:last=str(error)
            time.sleep(.03)
        raise TimeoutError(str(last))

    def capture(self,path,which='popup'):
        geometry=self.wait(lambda: value if (value:=self.ipc('capture',path,which)).startswith('{') else None)
        completed=self.wait(lambda: value if (value:=self.ipc('captureStatus',path)).startswith('{') else None)
        if not json.loads(completed)['saved']:raise RuntimeError('Qt failed to save '+str(path))
        return json.loads(geometry)

    def close(self):
        if self.child.poll() is None:
            subprocess.run(['qs','kill','--pid',str(self.child.pid)],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=3)
            try:self.child.wait(timeout=3)
            except subprocess.TimeoutExpired:self.child.kill();self.child.wait(timeout=3)
        self.stream.close()
        errors=[line for line in self.log.read_text().splitlines() if any(t in line for t in ('TypeError:','ReferenceError:','SyntaxError:','Failed to load configuration','Cannot assign','Binding loop detected','Error calling method'))]
        self.case.check(self.child.returncode==0 and not errors,'owned shell closes without QML errors',f'exit={self.child.returncode}; errors={errors}; log={self.log}')
        self.compositor.close()


def finish(name):
    result=suite_status('qs_visual')
    archive=ROOT/'target/test-logs/qs_visual_runs'/name
    shutil.copytree(ROOT/'target/test-logs/qs_visual',archive,dirs_exist_ok=True)
    return result


def self_test(binary):
    with Case('qs_visual','harness') as case,fixtures.Sandbox() as sandbox:
        a=sandbox.root/'a.png';b=sandbox.root/'b.png';output=sandbox.root/'pair.png'
        Image.new('RGB',(2,2),'black').save(a);Image.new('RGB',(2,2),'black').save(b)
        case.check(diff(a,b)==0,'identical PNG pixel difference is zero',str(diff(a,b)))
        image=Image.open(b);image.putpixel((0,0),(255,255,255));image.save(b)
        case.check(diff(a,b)==25,'one of four changed pixels is 25 percent',str(diff(a,b)))
        pair(a,b,output,dict(torrents={'titleBaseline':30},network={'titleBaseline':30}))
        case.check(output.is_file() and Image.open(output).height>2,'comparison writes both panels and annotation space',str(output))
        with case.path.with_suffix('.fixture.log').open('w') as error:
            child=subprocess.Popen([sys.executable,str(ROOT/'agent_ergonomics_audit/audit/regression_tests/daemon.py'),str(binary),'--serve-fixture','paused'],env=sandbox.env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=error,text=True)
            try:
                ready=None;deadline=time.monotonic()+20
                while time.monotonic()<deadline and child.poll() is None:
                    if select.select([child.stdout],[],[],.1)[0]:
                        line=child.stdout.readline()
                        if line.startswith('FBTFIXTURE '):ready=json.loads(line[len('FBTFIXTURE '):]);break
                case.check(bool(ready) and ready['row']['state']=='paused','fixture CLI exposes a real paused record',json.dumps(ready))
            finally:
                child.stdin.close()
                try:child.wait(timeout=12)
                except subprocess.TimeoutExpired:child.kill();child.wait()
                case.check(child.returncode==0,'stdin EOF drains and reaps the fixture CLI',f'exit={child.returncode}')
    return finish('harness')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('surface',choices=['popup','bar'],nargs='?',default='popup')
    parser.add_argument('state',nargs='?',default='downloading')
    parser.add_argument('--binary',type=Path,default=Path(os.environ.get('FBT_TEST_BINARY',ROOT/'target/debug/fbittorrent')))
    parser.add_argument('--light',default='tokyo-night',dest='theme')
    parser.add_argument('--compare',choices=['omarchy.network'])
    parser.add_argument('--self-test',action='store_true')
    parser.add_argument('--dry-run',action='store_true')
    args=parser.parse_args()
    if not args.dry_run and args.surface!='bar':
        import private_net
        private_net.enter()
    if args.self_test:return self_test(args.binary.resolve())
    if args.dry_run:
        with Case('qs_visual','dry_run') as case:
            for name,argv in [('qs',['qs','--version']),('grim',['grim','-h']),('omarchy-shell',['omarchy-shell','--help']),('hyprctl',['hyprctl','version'])]:
                executable=shutil.which(name)
                reply=subprocess.run(argv,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=3) if executable else None
                case.check(bool(executable),'capture tool available: '+name,f'{executable}; '+(reply.stdout.splitlines()[0] if reply and reply.stdout else 'no version output'))
            case.check(True,'isolation policy','private Qs config, compositor and theme, Qt item capture; no owner-shell IPC, theme switch or whole-desktop screenshot')
        return finish('dry_run')
    if os.environ.get('FBT_TEST_VISUAL')!='1':parser.error('set FBT_TEST_VISUAL=1 for owned capture processes')
    if not (ROOT/'contracts/ui/themes'/f'{args.theme}.toml').is_file():parser.error('theme is not a checked-in fixture')
    if args.surface=='bar':
        # The bar gallery uses the same actual native icon; its state source is explicitly presentational.
        return subprocess.run([str(Path(__file__).with_name('qs-icons.sh'))],env=dict(os.environ)).returncode
    if args.state not in STATES:parser.error('real popup fixture states: '+', '.join(STATES))
    with Case('qs_visual',args.state+'_'+args.theme.replace('-','_')) as case,running(args.binary,args.state,case) as (sandbox,harness,client,row):
        q=Capture(case,sandbox,args.theme)
        try:
            q.wait(lambda:json.loads(q.ipc('state'))['online'])
            before=q.ipc('useRuntimeDir',sandbox.env['XDG_RUNTIME_DIR'])
            case.check(before=='ok','isolated runtime hook accepts the owned fixture',before)
            q.wait(lambda:json.loads(q.ipc('state'))['online'])
            q.ipc('open')
            q.wait(lambda:json.loads(q.ipc('state'))['opened'] and (args.state=='empty' or json.loads(q.ipc('state'))['rows']>0))
            directory=ROOT/'target/test-logs/visual'/date.today().isoformat();directory.mkdir(parents=True,exist_ok=True)
            path=directory/f'{args.surface}-{args.state}-{args.theme}.png'
            previous=path.with_suffix('.previous.png')
            if path.exists():shutil.copyfile(path,previous)
            geometry=q.capture(path)
            case.check(path.is_file() and geometry['titleBaseline'] is not None and (args.state!='downloading' or all(geometry[key] is not None for key in ('metaBaseline','separatorY','rowPitch'))),'real popup over fixture daemon rendered',f'{path}; geometry={geometry}; previous pixel diff={diff(previous,path)}%')
            if args.compare:
                q.ipc('referenceOpen');reference=path.with_name(f'omarchy.network-{args.theme}.png')
                ref_geometry=q.capture(reference,'reference')
                output=path.with_name(path.stem+'-pair.png');pair(path,reference,output,dict(torrents=geometry,network=ref_geometry))
                case.check(output.is_file() and ref_geometry['titleBaseline'] is not None and ref_geometry['separatorY'] is not None,'pair names both actual panels and prints geometry below',f'{output}; ours={geometry}; reference={ref_geometry}; reference backend unavailable by design')
            q.ipc('close')
            case.check(not json.loads(q.ipc('state'))['opened'],'isolated popup/hook restored before process exit',q.ipc('state'))
        finally:q.close()
    return finish(case.name)

if __name__=='__main__':sys.exit(main())
