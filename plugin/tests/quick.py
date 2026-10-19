#!/usr/bin/env python3
"""Optional palette installation and a real search/add/review journey in private XDG."""
import argparse,json,os,shutil,subprocess,sys,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
import daemon as d
import fixtures,fbtlog
from daemon_bus import RevisionStream
from qs_env import prepare
from headless import Headless


def install(case,box,binary):
    d.private_environment(box)
    cfg=Path(box.env['XDG_CONFIG_HOME']);bindings=cfg/'hypr/bindings.lua';bindings.parent.mkdir()
    original='-- private fixture binding\no.bind("SUPER + B", "Browser", "browser")\n';bindings.write_text(original)
    shell=cfg/'omarchy/shell.json';shell.parent.mkdir();shell.write_text('{"bar":{"layout":{"left":["oliver.torrents"]}},"plugins":[{"id":"other.overlay"}]}\n')
    sentinel=box.downloads/'keep.bin';sentinel.write_bytes(b'owned payload')
    # Commands are recorded in this private fixture; never contact the live shell/manager.
    for command in ['systemctl','omarchy','omarchy-shell']:fixtures.path_shim(command,'exit 0',box.root)
    def setup(*args):
        step=case.step('private setup '+' '.join(args),'exit 0 with one JSON object; private paths only')
        result=subprocess.run([str(binary),'--no-start','setup',*args,'--json'],env=box.env,capture_output=True,text=True,timeout=15)
        value=json.loads(result.stdout);step.close(result.returncode==0 and value.get('ok') is True,json.dumps(dict(exit=result.returncode,result=value,stderr=result.stderr)))
        assert result.returncode==0;return value
    planned=setup('--plugin','copy','--dry-run')
    case.check(not any('torrents-quick' in a['path'] or 'bindings.lua' in a['path'] for a in planned['actions']) and bindings.read_text()==original,'normal setup never opts into the palette or shortcut',json.dumps(planned))
    planned=setup('--plugin','copy','--quick','SUPER + CTRL + ALT + T','--dry-run')
    main=cfg/'omarchy/plugins/oliver.torrents';quick=cfg/'omarchy/plugins/oliver.torrents-quick'
    case.check(not quick.exists() and bindings.read_text()==original,'dry-run shows overlay and binding without writing them',json.dumps(planned))
    setup('--plugin','copy','--quick','SUPER + CTRL + ALT + T')
    value=json.loads(shell.read_text());case.check(json.loads((quick/'manifest.json').read_text())['kinds']==['overlay'] and value['bar']['layout']['left']==['oliver.torrents'] and value['plugins']==[dict(id='other.overlay'),dict(id='oliver.torrents-quick')],'opt-in installs/enables separate id and preserves bar placement',json.dumps(value))
    lua='o={bind=function(chord,label,route) if type(route)=="table" and route.panel=="oliver.torrents-quick" then assert(chord=="SUPER + CTRL + ALT + T");found=true end end};\n'+bindings.read_text()+'\nassert(found)'
    parsed=subprocess.run(['lua','-'],input=lua,env=box.env,capture_output=True,text=True,timeout=3)
    case.check(parsed.returncode==0,'generated opt-in Lua binding invokes the separate overlay route',parsed.stderr)
    case.check(setup('--plugin','copy','--quick','SUPER + CTRL + ALT + T')['actions']==[],'second install is idempotent','no planned writes or commands')
    return main,quick,lambda:setup('--uninstall'),lambda:bindings.read_text()==original and sentinel.read_bytes()==b'owned payload' and json.loads(shell.read_text())['plugins']==[dict(id='other.overlay')]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('binary',type=Path);parser.add_argument('--log-root',type=Path)
    args=parser.parse_args();binary=args.binary.resolve()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    with fbtlog.Case('quick_ui','palette_live') as case,fixtures.daemon_pair(case,binary,False,262144) as pair,d.Sandbox() as box:
        source=pair.box.root/'source';source.mkdir();items=[]
        for i in range(12):
            path=source/f'Result-{i:02}.bin';path.write_bytes(f'owned search result {i}'.encode())
            item=fixtures.torrent_fixture(source,[path],name=f'result-{i:02}');items.append(item)
        direct_payload=source/'Direct.bin';direct_payload.write_bytes(b'owned explicit path')
        direct_item=fixtures.torrent_fixture(source,[direct_payload],name='direct')
        with fixtures.HttpFixture(source) as http:
            runtime=Path(pair.box.env['XDG_DATA_HOME'])/'fbittorrent/nova3';engines=runtime/'engines';engines.mkdir(parents=True,exist_ok=True)
            (runtime/'fixture-items.json').write_text(json.dumps([dict(link=http.base+'/'+item.torrent.name,name=item.files[0].name,size=item.files[0].stat().st_size,seeds=20-i,leech=0,engine_url=http.base,desc_link=http.base+'/source') for i,item in enumerate(items)]))
            (engines/'quickfixture.py').write_text('class quickfixture:\n name="Owned palette provider"\n url="https://fixture.invalid"\n def search(self,query):\n  import json,pathlib\n  from novaprinter import prettyPrinter\n  for row in json.loads(pathlib.Path("fixture-items.json").read_text()):prettyPrinter(row)\n')
            installed,quick_path,uninstall,clean=install(case,box,binary)
            compositor=Headless(box,case)
            config,env=prepare(box,Path(__file__).with_name('quick-live'),source=installed)
            shutil.rmtree(config/'QuickPlugin');shutil.copytree(quick_path,config/'QuickPlugin')
            host=config/'Host';host.mkdir();shutil.copyfile('/usr/share/omarchy/shell/services/PluginRegistry.qml',host/'PluginRegistry.qml')
            config.rename(box.root/'shell');config=box.root/'shell'
            env.update(compositor.env,FBT_TEST_BINARY=str(binary),OMARCHY_PATH=str(box.root),XDG_RUNTIME_DIR=pair.box.env['XDG_RUNTIME_DIR'])
            log=case.path.with_suffix('.qs.log');stream=log.open('w');child=subprocess.Popen(['qs','--no-color','-p',str(config)],env=env,stdout=stream,stderr=subprocess.STDOUT)
            def ipc(target,method,*values):
                r=subprocess.run(['qs','ipc','-p',str(config),'call',target,method,*map(str,values)],env=env,capture_output=True,text=True,timeout=3)
                if r.returncode:raise RuntimeError(f'qs IPC {r.returncode}: {r.stderr} {r.stdout}')
                return r.stdout.strip()
            def state():return json.loads(ipc('quick-test','state'))
            def route(method,id='oliver.torrents-quick',payload='{}'):
                r=subprocess.run(['/usr/share/omarchy/bin/omarchy-shell','shell',method,id,payload],env=env,capture_output=True,text=True,timeout=3)
                case.check(r.returncode==0 and r.stdout.strip()=='true','actual shell CLI '+method+' '+id,r.stdout+r.stderr)
            def keys(args,predicate,label):
                keyboard=subprocess.Popen(['wtype','-s','80',*args,'-s','150'],env=env)
                try:assert pair.h.wait_for(lambda:(predicate(state()),json.dumps(state())),label,timeout=10)
                finally:keyboard.wait(timeout=4)
            def open_palette():
                route('summon');assert pair.h.wait_for(lambda:(state()['visible'],json.dumps(state())),'separate palette route maps its native overlay')
            try:
                assert pair.h.wait_for(lambda:('Configuration Loaded' in log.read_text(),log.read_text()[-1800:]),'private shell loads installed plugins')
                assert pair.h.wait_for(lambda:(ipc('quick-test','ready')=='true',ipc('quick-test','state')),'shared daemon service connects')
                open_palette();keys(['Local search'],lambda s:len(s['palette']['rows'])==10,'ordinary native typing streams ten real Nova results')
                counts=pair.client.call('app.stats')['result']['active_clients'];case.check(counts['bar']==1,'palette reuses exactly one shared bar connection',json.dumps(counts))
                sub=pair.client.call('subscribe',dict(topics=['summary'],interval_ms=250))['result']['sub']
                summary=next(f['data'] for f in RevisionStream(pair.client).snapshot(sub,['summary']) if f.get('topic')=='summary')
                case.check('interactive' in summary['daemon']['busy_reasons'],'visible palette counts as interactive presence',json.dumps(summary['daemon']))
                pair.client.call('unsubscribe',dict(sub=sub))
                before=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']['completed_bytes']
                keys(['-k','Escape'],lambda s:not s['visible'],'Esc closes only the palette')
                route('toggle','oliver.torrents');assert pair.h.wait_for(lambda:(state()['bar'] and not state()['visible'] and not state()['dashboard'],json.dumps(state())),'normal bar toggle retains its popup route')
                route('toggle','oliver.torrents');assert pair.h.wait_for(lambda:(not state()['bar'],json.dumps(state())),'bar popup closes separately')
                open_palette();keys([str(direct_item.torrent),'-k','Return'],lambda s:not s['visible'],'explicit local torrent path adds immediately on Enter')
                direct=pair.client.call('torrents.get',dict(hash=direct_item.info_hash))['result']['torrent'];case.check(not direct['staged'],'explicit path skips review',json.dumps(direct))
                open_palette();magnet='magnet:?xt=urn:btih:'+'b'*40+'&dn=Quick+magnet&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce'
                keys([magnet,'-k','Return'],lambda s:not s['visible'],'explicit magnet adds without dashboard review')
                case.check(not pair.client.call('torrents.get',dict(hash='b'*40))['result']['torrent']['staged'],'magnet has a durable non-staged record','owned unresolved magnet')
                open_palette();keys(['Local search'],lambda s:len(s['palette']['rows'])==10,'fresh search session reuses shared services')
                keys(['-k','Down'],lambda s:s['palette']['cursor']==1,'native Down selects a search result');chosen=state()['palette']['rows'][1]
                keys(['-k','Return'],lambda s:not s['visible'] and (s['add'] or {}).get('visible') and (s['add'] or {}).get('phase')=='ready','Enter stages selected result and opens dashboard review')
                staged_hash=state()['add']['hash'];record=pair.client.call('torrents.get',dict(hash=staged_hash))['result']['torrent']
                case.check(record['staged'] and record['name']==chosen['name'] and not (pair.box.downloads/chosen['name']).exists(),'search selection remains staged without creating a payload before review',json.dumps(record))
                keys(['-k','Escape'],lambda s:not (s['add'] or {}).get('visible'),'dashboard Cancel discards the staged selection')
                pair.client.call('torrents.get',dict(hash=staged_hash),error='not_found')
                ipc('quick-test','closeDashboard')
                after=pair.client.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
                case.check(after['completed_bytes']>before and pair.h.daemon_pid()>0,'Esc, bar toggles and review cancellation leave the real transfer running',json.dumps(dict(before=before,after=after)))
            finally:
                subprocess.run(['qs','kill','--pid',str(child.pid)],env=env,capture_output=True,timeout=3)
                try:child.wait(timeout=3)
                except subprocess.TimeoutExpired:child.kill();child.wait()
                stream.close();compositor.close()
                errors=[line for line in log.read_text().splitlines() if any(x in line for x in ['TypeError:','ReferenceError:','SyntaxError:','Cannot assign','Binding loop detected','Failed to load configuration','Error calling method'])]
                case.check(child.returncode==0 and not errors,'private overlay/dashboard journey exits cleanly',json.dumps(dict(exit=child.returncode,errors=errors)))
            case.check('ERROR api subscription failed' not in case.suite.stderr_path.read_text(),'subscription handoff produces no API sampling errors','owned daemon stderr checked')
            uninstall();case.check(clean() and not quick_path.exists(),'uninstall removes only owned palette integration and keeps the payload sentinel','original binding, other plugin and payload preserved')
    return fbtlog.suite_status('quick_ui')
if __name__=='__main__':sys.exit(main())
