#!/usr/bin/env python3
"""Owned headless Quickshell process, isolated from the installed shell."""
import os
from pathlib import Path
import sys
import tempfile
import shutil
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'agent_ergonomics_audit/audit/regression_tests'))
import fixtures
import importlib.util
spec = importlib.util.spec_from_file_location('qs_log', Path(__file__).with_name('qs-log.py'))
qs_log = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qs_log)
with fixtures.Sandbox() as sandbox:
    os.environ.update(sandbox.env)
    os.environ['QT_QPA_PLATFORM'] = 'offscreen'
    os.environ['QT_QUICK_BACKEND'] = 'software'
    from qs_env import prepare
    config, env = prepare(sandbox, Path(sys.argv[2]).resolve())
    if sys.argv[1]=='icons':
        from icon_fixtures import write
        write(config)
    os.environ.clear()
    os.environ.update(env)
    command = ['timeout', '60s', 'qs', '--no-color', '-p', str(config)]
    sys.exit(qs_log.run('qs_' + sys.argv[1], command))
