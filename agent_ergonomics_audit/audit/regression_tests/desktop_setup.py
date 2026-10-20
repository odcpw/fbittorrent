"""Native desktop setup with only isolated prefixes, HOME/XDG and command shims."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from fbtlog import Case
import fixtures

ROOT=Path(__file__).resolve().parents[3]

def prepare(sandbox):
    for name in ('systemctl','omarchy','omarchy-shell','omarchy-restart-shell','update-desktop-database'):
        fixtures.path_shim(name,'printf "external integration chatter\\n"\nexit 0',sandbox.root)
    # Keep the real xdg-mime's isolated output and association semantics; suppress
    # desktop-environment discovery so it uses its generic MIME writer.
    mime=Path('/usr/bin/xdg-mime')
    fixtures.path_shim('xdg-mime',f'exec {mime} "$@"',sandbox.root)
    sandbox.env.update(XDG_CURRENT_DESKTOP='generic',DE='generic')
    for name in ('WAYLAND_DISPLAY','DISPLAY','HYPRLAND_INSTANCE_SIGNATURE','SWAYSOCK'):
        sandbox.env.pop(name,None)
    sandbox.env['DBUS_SESSION_BUS_ADDRESS']='unix:path='+str(sandbox.root/'absent-bus')
    config=Path(sandbox.env['XDG_CONFIG_HOME'])
    extension=config/'omarchy/extensions/omarchy-menu.jsonc';extension.parent.mkdir(parents=True)
    extension.write_text('{\n// retain this unrelated comment\n"other":{"label":"Kept","action":"echo keep"},\n"torrents.stop":{"when":"systemctl --user is-active qbittorrent-nox"},\n}\n')
    return config

def call(case,binary,sandbox,*args,expected=0,cwd=None,prefix=()):
    command=[*prefix,str(binary),'setup',*args,'--json']
    result=subprocess.run(command,env=sandbox.env,cwd=cwd,capture_output=True,text=True,timeout=12)
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
        config=prepare(sandbox)
        shell=config/'omarchy/shell.json'
        original='{\n  "bar" : { "layout" : { "right" : [\n    {"id":"oliver.torrents","settings":{"owner":true}},\n    {"id":"other.widget"}\n  ] } },\n  "unrelated" : { "spacing" : "keep me" }\n}\n'
        shell.write_text(original)
        call(case,binary,sandbox,'--plugin=copy')
        case.check(shell.read_text()==original,'copy setup recognizes an existing object widget and preserves all shell bytes',shell.read_text())
        before=snapshot(sandbox);copied=call(case,binary,sandbox,'--plugin=copy')
        case.check(copied['actions']==[] and snapshot(sandbox)==before,'second copy setup leaves the object placement and entire tree byte/mtime-identical',json.dumps(copied))
        duplicated=original.replace('    {"id":"other.widget"}\n','    {"id":"other.widget"},\n    "oliver.torrents"\n')
        shell.write_text(duplicated)
        call(case,binary,sandbox,'--plugin=copy')
        repaired=shell.read_text();right=json.loads(repaired)['bar']['layout']['right']
        expected=original.replace('    {"id":"other.widget"}\n','    {"id":"other.widget"}\n    \n')
        case.check(right==[dict(id='oliver.torrents',settings=dict(owner=True)),dict(id='other.widget')] and repaired==expected,'setup repairs its string duplicate, keeps the first object/settings and unrelated formatting',repaired)
        before=snapshot(sandbox);again=call(case,binary,sandbox,'--plugin=copy')
        case.check(again['actions']==[] and snapshot(sandbox)==before,'repaired layout stays unchanged on the next copy setup',json.dumps(again))
        first=call(case,binary,sandbox,'--magnet','--plugin=link');before=snapshot(sandbox)
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
        before=snapshot(sandbox);refused=call(case,binary,sandbox,'--purge','--yes',expected=1)
        case.check('refusing to purge' in refused.get('error','') and snapshot(sandbox)==before,
                   'even confirmed purge refuses unreadable torrent records without changing data',json.dumps(refused))
        # A saved torrent can live outside today's configured/default save path,
        # including inside the app tree that purge would otherwise remove.
        saved=data/'previous-save';saved.mkdir();saved_payload=saved/'keep.bin';saved_payload.write_bytes(b'saved payload stays')
        record=dict(hash='a'*40,name='Saved payload',source='magnet:?xt=urn:btih:'+'a'*40,
                    output_folder=str(saved),only_files=None,upload_limit=0,category='',tags='',
                    added_on=1,completion_on=0,paused=True)
        (data/'engine/torrents.json').write_text(json.dumps(dict(records=[record])))
        call(case,binary,sandbox,'--purge','--yes')
        case.check(all(not path.exists() for path in keep) and download.read_bytes()==b'payload stays'
                   and nested.read_bytes()==b'nested payload stays' and saved_payload.read_bytes()==b'saved payload stays',
                   'confirmed purge removes readable records but preserves configured and saved payload folders',str([download,nested,saved_payload]))

def setup_embedded_plugins(binary,systemd=False):
    with Case('daemon','setup_embedded_plugins') as case:
        for mode in ('copy','link'):
            with fixtures.Sandbox() as sandbox:
                config=prepare(sandbox)
                # A missing command must fail, never fall through to a desktop
                # executable later in the owner's PATH. Shim interpreters and
                # their logger use absolute paths.
                sandbox.env['PATH']=str(sandbox.root/'shims')
                relocated=sandbox.root/'bin/fbittorrent';relocated.parent.mkdir()
                relocated.touch()
                # Relocate the executable with a read-only file mount instead
                # of duplicating its large debug symbols into /tmp. All setup
                # writes still use the harness's usual private /tmp filesystem.
                # Hide the build checkout in a private mount namespace too:
                # an old CARGO_MANIFEST_DIR lookup must actually fail here.
                # The host filesystem is read-only except this sandbox.
                prefix=[shutil.which('bwrap') or 'bwrap','--die-with-parent','--unshare-user','--unshare-pid',
                        '--unshare-ipc','--unshare-net','--ro-bind','/','/',
                        '--tmpfs','/run/user','--tmpfs',str(Path.home()),
                        '--tmpfs',str(ROOT),'--bind',str(sandbox.root),str(sandbox.root),
                        '--ro-bind',str(binary),str(relocated),
                        '--dev','/dev','--proc','/proc',
                        '--chdir',str(sandbox.root),'--']
                hidden=subprocess.run([*prefix,'/bin/sh','-c','test ! -e "$1/Cargo.toml"',
                                       'source-check',str(ROOT)],env=sandbox.env,capture_output=True,text=True,timeout=5)
                case.check(hidden.returncode==0,'build checkout is absent inside the relocated process namespace',hidden.stderr)
                case.check(not (sandbox.root/'plugin').exists(),
                           'relocated binary and working directory contain no plugin source',str(relocated))
                first=call(case,relocated,sandbox,'--plugin='+mode,'--quick',cwd=sandbox.root,prefix=prefix)
                if mode=='link':
                    warnings=first.get('warnings',[])
                    case.check(len(warnings)==2 and all(w['code']=='plugin_link_copy' and 'embedded' in w['message'] for w in warnings),
                               'source-free link explicitly falls back to embedded copies for both plugins',str(warnings))
                for name in ('oliver.torrents','oliver.torrents-quick'):
                    source=ROOT/'plugin'/name;target=config/'omarchy/plugins'/name
                    expected={str(p.relative_to(source)):p.read_bytes() for p in source.rglob('*') if p.is_file()}
                    actual={str(p.relative_to(target)):p.read_bytes() for p in target.rglob('*') if p.is_file()}
                    case.check(not target.is_symlink() and actual==expected,
                               mode+' installs every shipping file byte-identically: '+name,
                               f'files={len(actual)}; bytes={sum(map(len,actual.values()))}')
                restart=sandbox.root/'shims/omarchy-restart-shell.log'
                case.check(len(restart.read_text().splitlines())==1,'changed install restarts only the private shim once',restart.read_text())
                before=snapshot(sandbox)
                commands={p.name:p.read_bytes() for p in (sandbox.root/'shims').glob('*.log')}
                second=call(case,relocated,sandbox,'--plugin='+mode,'--quick',cwd=sandbox.root,prefix=prefix)
                case.check(second['actions']==[] and snapshot(sandbox)==before,
                           'second relocated '+mode+' setup changes no bytes or mtimes',str(second))
                case.check(commands=={p.name:p.read_bytes() for p in (sandbox.root/'shims').glob('*.log')},
                           'second setup runs no integration commands or shell restart',str(commands.keys()))
                # A failed shell restart must not write the revision stamp:
                # the next successful invocation must retry even if files match.
                plugin=config/'omarchy/plugins/oliver.torrents/Dashboard.qml'
                plugin.write_text('stale UI')
                shim=sandbox.root/'shims/omarchy-restart-shell'
                shim.write_text('#!/missing-fbt-interpreter\n');shim.chmod(0o700)
                failed=call(case,relocated,sandbox,'--plugin='+mode,'--quick',expected=1,cwd=sandbox.root,prefix=prefix)
                case.check(failed['code_name']=='failure' and 'running omarchy-restart-shell' in failed['error']
                           and 'is closed' not in failed['error'] and 'hint' not in failed,
                           'missing setup program reports its real I/O failure, not a closed daemon',str(failed))
                fixtures.path_shim('omarchy-restart-shell','exit 0',sandbox.root)
                repaired=call(case,relocated,sandbox,'--plugin='+mode,'--quick',cwd=sandbox.root,prefix=prefix)
                case.check(any(a['path']=='omarchy-restart-shell' for a in repaired['actions'])
                           and plugin.read_bytes()==(ROOT/'plugin/oliver.torrents/Dashboard.qml').read_bytes(),
                           'retry restarts after the failed restart and retains the embedded UI',str(repaired))

CASES=(setup_dry_run_lists_actions,setup_idempotent,setup_uninstall_keeps_data,setup_embedded_plugins)
