#!/usr/bin/env python3
"""The timing adapter may start once; never mask an unexpected daemon exit."""
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from shell_latency import start_requested_daemon


class ColdStartAdapter(unittest.TestCase):
    def test_only_recorded_initial_up_starts_the_daemon(self):
        with TemporaryDirectory(prefix='fbt-latency-adapter-') as root:
            path=Path(root);(path/'shims').mkdir()
            swarm=SimpleNamespace(box=SimpleNamespace(root=path),p=SimpleNamespace(child=SimpleNamespace(poll=lambda:0)))
            launched=[]
            def start():
                launched.append(True)
                swarm.p.child=SimpleNamespace(poll=lambda:None)
            swarm.start=start
            observed=[]
            start_requested_daemon(swarm,observed)
            self.assertFalse(launched)
            (path/'shims/fbittorrent.log').write_text('["up"]\n')
            start_requested_daemon(swarm,observed)
            start_requested_daemon(swarm,observed)
            self.assertEqual(len(launched),1)
            self.assertEqual(len(observed),1)
            swarm.p.child=SimpleNamespace(poll=lambda:-9)
            with self.assertRaisesRegex(RuntimeError,'exited -9 during warm session'):
                start_requested_daemon(swarm,observed)
            self.assertEqual(len(launched),1, 'unexpected exits must not be restarted or hidden')


if __name__=='__main__':unittest.main()
