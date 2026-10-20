#!/usr/bin/env python3
"""A real terminal must remain actionable while 500 paused rows restore.

The zero-filled pieces are real legal payload. Their first check on restore
also exercises completion persistence, while a separate loopback seed supplies
an active download. No external network or owner data is available.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import daemon as d
import fbtlog
import fixtures
from tui_journey_fixture import Fixture
from tui_smoke import PaneUi, private_net


def run(binary):
    with fbtlog.Case('tui_restore_load', 'restart_500') as case, Fixture(case, binary, 'file') as f:
        f.client.call('torrents.add', dict(source=dict(path=str(f.torrent.torrent)), save_path=str(f.box.downloads)))
        directory = f.box.root / 'many'
        directory.mkdir()
        for index in range(499):
            name = f'Row {index:03} · 日本語 🧲 e\u0301 ' + 'long name ' * 14 + '.bin'
            info = {'name': name, 'length': 16384, 'piece length': 16384,
                    'pieces': hashlib.sha1(bytes(16384)).digest(), 'private': 1}
            path = directory / f'{index}.torrent'
            path.write_bytes(fixtures.bencode(dict(info=info)))
            reply = f.client.call('torrents.add', dict(source=dict(path=str(path)),
                                  save_path=str(f.box.downloads), stopped=True))
            assert reply['ok'], reply
        rows = f.rows()
        assert len(rows) == 500
        # Added times have one-second resolution: use the real hash tie-break
        # instead of assuming the first admitted torrent is the last row.
        target_hash = sorted(rows, key=lambda row: (-row['added_at'], row['hash']))[-1]['hash']
        ui = PaneUi(case, str(binary), f.box, f.h, session='fbt-restore-500')
        try:
            ui.start_owned()
            assert ui.observe('load the real list', 'all 500 rows reach the terminal',
                              lambda pane: 'Torrents (500)' in pane)
            f.process.child.kill()
            f.process.child.wait(timeout=5)
            assert ui.observe('kill the private daemon', 'loss is visible without losing this terminal',
                              lambda pane: 'fbittorrent stopped' in pane, timeout=10)
            f.h.notify.close()
            f.process = f.h.start_serve(idle_exit_secs=0)
            assert f.h.receive_ready() and f.h.wait_engine()
            assert f.h.wait_for(lambda: ((n := ui.clients()) == 1, f'interactive clients={n}'),
                                'same terminal completes its new API hello', timeout=20)
            # A cosmetic rename should not queue behind 499 repetitions of a
            # full-ledger reconciliation. Require a one-second foreground
            # response on this isolated fixture, while restores continue.
            with d.ApiConn(f.h, timeout=1) as fresh:
                fresh.hello()
                reply = fresh.call('torrents.rename', dict(hash=target_hash, name='Restored Café 日本語'))
                assert reply['ok'], reply
                fresh.socket.settimeout(10)
                # Hello is counted before Attached/initial rows reach the model.
                # Navigation starts when this terminal actually draws online data.
                assert ui.observe('consume restored rows', 'fresh online rows are actionable after hello',
                    lambda pane: 'Torrents (500)' in pane and '● offline' not in pane.splitlines()[0]
                    and 'Connecting' not in pane.splitlines()[0], timeout=15)
                assert ui.press('End', 'new pushed name reaches the existing terminal',
                                lambda pane: 'Restored Café 日本語' in pane, literal=False)
                assert len(fresh.call('torrents.list', {})['result']['torrents']) == 500
                assert f.h.wait_for(lambda: ((row := fresh.call('torrents.get',
                    dict(hash=f.torrent.info_hash))['result']['torrent']).get('completed_bytes', 0) > 0
                    and row['upload_enabled'] is False and row['uploaded_bytes'] == 0, json.dumps(row)),
                    'restored loopback download remains real and upload stays off')
            assert ui.press('q', 'terminal detaches after the restored mutation', lambda pane: ui.done.exists())
        finally:
            ui.close()
    return fbtlog.suite_status('tui_restore_load')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--log-root', type=Path)
    args = parser.parse_args()
    private_net.enter()
    if args.log_root:
        fbtlog.LOG_ROOT = args.log_root.resolve()
    sys.exit(run(args.binary.resolve()))
