"""Native desktop setup with only isolated prefixes, HOME/XDG and command shims."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from fbtlog import Case
import fixtures

ROOT=Path(__file__).resolve().parents[3]

def prepare(sandbox):
    for name in ('systemctl','omarchy','update-desktop-database'):
        fixtures.path_shim(name,'printf "external integration chatter\\n"\nexit 0',sandbox.root)
    # Keep the real xdg-mime's isolated output and association semantics; suppress
    # desktop-environment discovery so it uses its generic MIME writer.
    mime=Path('/usr/bin/xdg-mime')
    fixtures.path_shim('xdg-mime',f'exec {mime} "$@"',sandbox.root)
    sandbox.env.update(XDG_CURRENT_DESKTOP='generic',DE='generic')
    sandbox.env.pop('WAYLAND_DISPLAY',None);sandbox.env.pop('DISPLAY',None)
    sandbox.env['DBUS_SESSION_BUS_ADDRESS']='unix:path='+str(sandbox.root/'absent-bus')
    config=Path(sandbox.env['XDG_CONFIG_HOME'])
    extension=config/'omarchy/extensions/omarchy-menu.jsonc';extension.parent.mkdir(parents=True)
    extension.write_text('{\n// retain this unrelated comment\n"other":{"label":"Kept","action":"echo keep"},\n"torrents.stop":{"when":"systemctl --user is-active qbittorrent-nox"},\n}\n')
    return config

def call(case,binary,sandbox,*args,expected=0):
    command=[str(binary),'setup',*args,'--json']
    result=subprocess.run(command,env=sandbox.env,capture_output=True,text=True,timeout=12)
    path=case.path.with_suffix('.stdout.log')
    with path.open('a') as output:output.write(json.dumps(command)+'\n'+result.stdout+result.stderr+'\n')
    case.step('run '+' '.join(command),'setup exits '+str(expected)).close(result.returncode==expected,f'exit={result.returncode}; {result.stdout}; {result.stderr}',artifacts=[path])
    return json.loads(result.stdout)

def snapshot(sandbox):
    result={}
    for root in [sandbox.env[k] for k in sandbox.keys[:7]]:
        for path in Path(root).rglob('*'):
            if path.is_symlink():result[str(path)]=('link',os.readlink(path),path.lstat().st_mtime_ns)
            elif path.is_file():result[str(path)]=(hashlib.sha256(path.read_bytes()).hexdigest(),path.stat().st_mtime_ns)
    return result

def setup_dry_run_lists_actions(binary,systemd=False):
    with Case('daemon','setup_dry_run_lists_actions') as case,fixtures.Sandbox() as sandbox:
        config=prepare(sandbox);before=snapshot(sandbox)
        dry=call(case,binary,sandbox,'--magnet','--plugin=link','--dry-run')
        case.check(snapshot(sandbox)==before,'dry-run writes nothing and invokes no integration process',json.dumps(dry))
        applied=call(case,binary,sandbox,'--magnet','--plugin=link')
        case.check(dry['actions']==applied['actions'] and len(dry['actions'])>=8,'dry-run lists the exact executed action plan',json.dumps(dry['actions']))
        case.check('start' not in (sandbox.root/'shims/systemctl.log').read_text(),'setup never starts or enables a login unit',(sandbox.root/'shims/systemctl.log').read_text())

def setup_idempotent(binary,systemd=False):
    with Case('daemon','setup_idempotent') as case,fixtures.Sandbox() as sandbox:
        config=prepare(sandbox);first=call(case,binary,sandbox,'--magnet','--plugin=link');before=snapshot(sandbox)
        logs={p.name:p.read_bytes() for p in (sandbox.root/'shims').glob('*.log')}
        second=call(case,binary,sandbox,'--magnet','--plugin=link')
        case.check(second['actions']==[] and snapshot(sandbox)==before,'second setup has an empty plan and byte/mtime-identical tree',json.dumps(second))
        case.check(logs=={p.name:p.read_bytes() for p in (sandbox.root/'shims').glob('*.log')},'second setup invokes no commands',json.dumps(logs,default=lambda x:x.decode()))
        menu=(config/'omarchy/extensions/omarchy-menu.jsonc').read_text()
        case.check('"when":"fbittorrent status --quiet"' in menu and 'qbittorrent-nox' not in menu and '// retain this unrelated comment' in menu,'Stop gates on native status and unrelated JSONC bytes survive',menu)
        quiet=subprocess.run([str(binary),'status','--quiet'],env=sandbox.env,capture_output=True,timeout=5)
        case.check(quiet.returncode==4,'menu Stop condition is false while stopped and never starts a daemon',f'exit={quiet.returncode}')
        link=config/'omarchy/plugins/oliver.torrents';case.check(link.is_symlink(),'development install is a plugin symlink',str(link))
        call(case,binary,sandbox,'--plugin=copy')
        case.check(link.is_dir() and not link.is_symlink() and (link/'Dashboard.qml').is_file(),'stable copy replaces the link and contains actual plugin code',str(link))
        before=snapshot(sandbox);copy=call(case,binary,sandbox,'--plugin=copy');case.check(copy['actions']==[] and snapshot(sandbox)==before,'copy setup is also idempotent',json.dumps(copy))
        installed=sandbox.root/'prefix';command=['bash',str(ROOT/'scripts/install.sh'),'--prefix',str(installed),'--binary',str(binary),'--plugin','copy']
        result=subprocess.run(command,env=sandbox.env,capture_output=True,text=True,timeout=25)
        case.check(result.returncode==0 and (installed/'bin/fbittorrent').is_file(),'installer accepts an isolated prefix/prebuilt debug binary without a release build',result.stdout+result.stderr)
        case.check(not (installed/'bin/fbittorrent.tmp').exists(),'atomic installation leaves no temporary binary',str(installed))

def setup_uninstall_keeps_data(binary,systemd=False):
    with Case('daemon','setup_uninstall_keeps_data') as case,fixtures.Sandbox() as sandbox:
        config=prepare(sandbox);call(case,binary,sandbox,'--magnet','--plugin=link')
        data=Path(sandbox.env['XDG_DATA_HOME'])/'fbittorrent';keep={}
        for relative in ['engine/torrents.json','rss.json','shows.json']:
            path=data/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b'owner-shaped saved bytes\n');keep[path]=path.read_bytes()
        download=sandbox.downloads/'keep.bin';download.write_bytes(b'payload stays')
        call(case,binary,sandbox,'--uninstall')
        paths=[config/'systemd/user'/sandbox.unit,config/'omarchy/plugins/oliver.torrents',Path(sandbox.env['XDG_DATA_HOME'])/'icons/hicolor/scalable/apps/fbittorrent.svg']
        paths.extend(Path(sandbox.env['XDG_DATA_HOME'])/'applications'/name for name in ['fbittorrent.desktop','fbittorrent-magnet.desktop','fbittorrent-tui.desktop'])
        for path in paths:case.check(not path.exists() and not path.is_symlink(),'uninstall removes only integration: '+str(path),str(path))
        for path,body in keep.items():case.check(path.read_bytes()==body,'uninstall preserves '+path.name,str(path))
        menu=(config/'omarchy/extensions/omarchy-menu.jsonc').read_text();case.check('torrents.' not in menu and '"other"' in menu,'uninstall removes menu entries while preserving unrelated entries',menu)
        before=snapshot(sandbox);call(case,binary,sandbox,'--purge',expected=3);case.check(snapshot(sandbox)==before,'unconfirmed purge exits 3 and deletes nothing',json.dumps(before))
        inside=data/'downloads';inside.mkdir();nested=inside/'keep.bin';nested.write_bytes(b'nested payload stays')
        call(case,binary,sandbox,'--save',str(inside))
        call(case,binary,sandbox,'--purge','--yes')
        case.check(all(not path.exists() for path in keep) and download.read_bytes()==b'payload stays' and nested.read_bytes()==b'nested payload stays','confirmed purge removes records and preserves payloads even inside the app data directory',str(nested))

CASES=(setup_dry_run_lists_actions,setup_idempotent,setup_uninstall_keeps_data)
