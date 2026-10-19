"""Require the executable Cargo named; never fall back to a different target tree."""
import json
import os
from pathlib import Path

IDS={'old':'p1-17-old','new':'p1-17-new'}


def executable(output, env, repo):
    named=[]
    for line in output.read_text(errors='replace').splitlines():
        try:value=json.loads(line)
        except ValueError:continue
        if value.get('reason')=='compiler-artifact' and value.get('target',{}).get('name')=='fbittorrent' and value.get('executable'):
            named.append(Path(value['executable']))
    if not named:raise RuntimeError('Cargo did not identify its fixture executable')
    target=Path(env.get('CARGO_TARGET_DIR',repo/'target'))
    if not target.is_absolute():target=repo/target
    # RCH reports the worker path and promotes the executable into this target.
    for path in [*reversed(named),target/'debug/fbittorrent']:
        if path.is_file() and os.access(path,os.X_OK):return path
    raise RuntimeError('Cargo fixture executable was not delivered into the configured target')


def valid_cache(saved, paths, digest):
    return (saved.get('ids')==IDS and saved.get('old')!=saved.get('new')
            and all(path.is_file() and digest(path)==saved.get(tag)
                    for path,tag in zip(paths,IDS)))
