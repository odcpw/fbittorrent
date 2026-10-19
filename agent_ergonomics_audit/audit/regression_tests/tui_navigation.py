#!/usr/bin/env python3
"""Repeat real Files navigation across RPC/snapshot arrival and progress pushes.

Private XDG, loopback payload and tmux only. Optional CPU work widens the sampler
race without changing input waits, resending keys or touching the owner's data.
"""
import argparse
from contextlib import contextmanager
from pathlib import Path
import re
import subprocess
import sys

import fbtlog
from fbtlog import Case, REPO, suite_status
from tui_journey import add
from tui_journey_fixture import Fixture
from tui_smoke import PaneUi
sys.path.insert(0, str(REPO / 'plugin/tests'))
import private_net


@contextmanager
def cpu_load(count):
    workers = []
    try:
        for _ in range(count):
            workers.append(subprocess.Popen([sys.executable, '-c',
                'import hashlib,time; end=time.monotonic()+600; data=b"x"*65536\n'
                'while time.monotonic()<end: hashlib.sha256(data).digest()'],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        yield
    finally:
        for worker in workers:
            if worker.poll() is None: worker.terminate()
        for worker in workers:
            try: worker.wait(timeout=5)
            except subprocess.TimeoutExpired: worker.kill(); worker.wait(timeout=5)


def selected(pane, name):
    return any('▌' in line and name in line for line in pane.splitlines())


def progress(pane):
    line = next((line for line in pane.splitlines() if 'Episode.mkv' in line), '')
    match = re.search(r'\d+%', line)
    return match.group() if match else None


def run(binary, number, cycles):
    with Case('tui_navigation', f'files_{number:02}') as case, Fixture(case, binary, 'file') as fixture:
        ui = PaneUi(case, str(binary), fixture.box, fixture.h, session=f'fbt-m3-end-{number}')
        def need(ok, label):
            if not ok: raise RuntimeError(label)
        try:
            ui.start_owned(); ui.tmux('resize-window', '-t', ui.target, '-x', '80', '-y', '24')
            add(ui, fixture, 'file')
            need(fixture.h.wait_for(lambda: ((row := fixture.row()).get('completed_bytes', 0) > 0,
                str(row)), 'real loopback payload begins'), 'payload did not begin')
            need(ui.observe('wait for the torrent row', 'real torrent is a visible keyboard target',
                lambda p: fixture.name in p), 'torrent absent')
            for cycle in range(cycles):
                need(ui.contains('Enter', '[Overview]', literal=False), 'details absent')
                need(ui.details_tab('Files'), 'Files absent')
                need(ui.observe('wait for actionable Files', 'three real file rows and controls are visible',
                    lambda p: all(s in p for s in ('English.srt', 'Episode.mkv', 'sample.mkv', 'include/skip'))), 'Files not ready')
                before = progress(ui.capture(ui.target))
                need(ui.press('End', f'End selects the actual final file on open {cycle}',
                    lambda p: selected(p, 'sample.mkv'), literal=False), 'End selection lost')
                if cycle == 0:
                    need(ui.observe('wait for a real progress push', 'End selection survives a newer Files update',
                        lambda p: selected(p, 'sample.mkv') and progress(p) not in (None, before)), 'progress reset selection')
                need(ui.press('Home', 'Home selects the actual first file',
                    lambda p: selected(p, 'English.srt'), literal=False), 'Home selection lost')
                need(ui.press('Escape', 'close Files to the torrent table',
                    lambda p: '[Torrents]' in p and 'include/skip' not in p, literal=False), 'close lost')
        finally:
            ui.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--repeat', type=int, choices=range(1, 31), default=10)
    parser.add_argument('--cycles', type=int, choices=range(1, 21), default=10)
    parser.add_argument('--cpu-load', type=int, choices=range(5), default=0)
    parser.add_argument('--log-root', type=Path)
    args = parser.parse_args(); private_net.enter()
    if args.log_root: fbtlog.LOG_ROOT = args.log_root.resolve()
    with cpu_load(args.cpu_load):
        for number in range(args.repeat): run(args.binary.resolve(), number, args.cycles)
    return suite_status('tui_navigation')


if __name__ == '__main__': sys.exit(main())
