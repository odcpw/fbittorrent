"""Isolated Quickshell config used by client, store and UI smoke tests."""
import os
from pathlib import Path
import shutil
import hashlib
import json

ROOT = Path(__file__).resolve().parents[2]


def prepare(sandbox, recipe, source=None, wayland=False):
    config = sandbox.root / 'qs-config'
    shutil.copytree(recipe, config)
    source = Path(source or os.environ.get('FBT_PLUGIN_SOURCE', ROOT / 'plugin/oliver.torrents')).resolve()
    shutil.copytree(source, config / 'Plugin')
    fixtures = ROOT / 'contracts/ipc/v1'
    shutil.copytree(fixtures, config / 'Fixtures')
    states=ROOT/'contracts/ui/states.json'
    if states.exists(): shutil.copy2(states,config/'States.json')
    for name in ('Commons', 'Ui'):
        (config / name).symlink_to('/usr/share/omarchy/shell/' + name, target_is_directory=True)
    env = dict(sandbox.env, FBT_FIXTURE_NAMES=json.dumps(sorted(p.name for p in fixtures.glob('*.ndjson'))), QT_QPA_PLATFORM='offscreen', QT_QUICK_BACKEND='software', FBT_TEST_HOOKS='1')
    env['FBT_FIXTURE_METADATA']=json.dumps({p.name:dict(md5=hashlib.md5(p.read_bytes()).hexdigest(),lines=len(p.read_text().splitlines())) for p in fixtures.glob('*.ndjson')})
    if wayland:
        display = Path(os.environ.get('WAYLAND_DISPLAY', 'wayland-1'))
        if not display.is_absolute(): display = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/' + str(os.getuid()))) / display
        if not display.is_socket(): raise RuntimeError('isolated UI smoke needs an existing Wayland display socket')
        env.update(QT_QPA_PLATFORM='wayland', WAYLAND_DISPLAY=str(display))
    else:
        env.pop('WAYLAND_DISPLAY', None)
    return config, env
