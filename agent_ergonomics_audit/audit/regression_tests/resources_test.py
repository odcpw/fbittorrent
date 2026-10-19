#!/usr/bin/env python3
"""Resource entry-point controls: current lifetime and unchanged measured budgets."""
from pathlib import Path
import unittest
from unittest.mock import patch
import resources


class ResourceEntry(unittest.TestCase):
    def test_default_qualifies_all_current_daemon_workloads(self):
        with patch('resources_daemon.main', return_value=17) as measured:
            self.assertEqual(resources.main(['/tmp/owned-binary','--baseline','recorded.json','--output','observed.json']),17)
        args=measured.call_args.args[0]
        self.assertTrue(args.matrix)
        self.assertEqual((args.seconds,args.runs),(60,3))
        self.assertEqual(args.baseline,Path('recorded.json'))
        self.assertEqual(args.output,Path('observed.json'))

    def test_explicit_single_configuration_keeps_the_requested_workload(self):
        with patch('resources_daemon.main', return_value=0) as measured:
            self.assertEqual(resources.main(['/tmp/owned-binary','--daemon','--torrents','100','--subscribers','8','--seconds','60','--runs','3','--output','observed.json']),0)
        args=measured.call_args.args[0]
        self.assertFalse(args.matrix)
        self.assertEqual((args.torrents,args.subscribers,args.seconds,args.runs),(100,8,60,3))


if __name__=='__main__':unittest.main()
