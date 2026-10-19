#!/usr/bin/env python3
"""Disposable process controls: safety retries never turn the gate green."""
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

import gate
from gate_safety import strict_check, strict_rust_test


class SafetyTests(unittest.TestCase):
    def test_successful_retry_keeps_every_safety_family_red(self):
        for name in ('daemon/zero_upload', 'daemon/delete_files_safety_live',
                     'tracker_privacy', 'native/q_confirms', 'daemon/idle_exit_glance_pauses'):
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix='fbt-strict-') as directory:
                root=Path(directory); marker=root/'ran'
                script=('import json,sys;from pathlib import Path;'
                        'p=Path(sys.argv[1]);again=p.exists();p.touch();'
                        'print("successful diagnostic retry" if again else "safety failure");'
                        'sys.exit(0 if again else 1)')
                check=gate.Check(name,[sys.executable,'-c',script,str(marker)])
                self.assertFalse(gate.run_checks([check],root/'logs',dict(os.environ)))
                self.assertEqual(check.status,'FAIL')
                self.assertIn('GATE-EXIT: 1',(root/'logs'/name/'first/output.log').read_text())
                self.assertIn('GATE-EXIT: 0',(root/'logs'/name/'retry/output.log').read_text())
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.summary([check],0,root,'fast'),1)
                self.assertFalse(json.loads((root/'summary.json').read_text())['ok'])

    def test_summary_also_rejects_a_strict_flaky_verdict(self):
        with tempfile.TemporaryDirectory(prefix='fbt-strict-') as directory, contextlib.redirect_stdout(io.StringIO()):
            check=gate.Check('daemon/zero_upload',[],status='FLAKY')
            self.assertEqual(gate.summary([check],0,Path(directory),'fast'),1)
            self.assertEqual(check.status,'FAIL')

    def test_safety_rust_failure_is_sticky_and_timing_retry_stays_flaky(self):
        for name,safety in [('engine::tests::upload::permission',True),
                            ('engine::dormant_delete::tests::parent_symlinks',True),
                            ('api::trackers::tests::redaction',True),
                            ('native_runtime::shutdown_tests::deadline',True),
                            ('engine::tests::timing_only',False)]:
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix='fbt-strict-') as directory:
                root=Path(directory)
                cargo=root/'cargo'
                cargo.write_text('#!/usr/bin/env python3\nimport sys\n'
                    'if "--exact" in sys.argv:\n print("test result: ok. 1 passed; 0 failed;")\n'
                    'else:\n print("failures:\\n    '+name+'\\n\\ntest result: FAILED. 0 passed; 1 failed;");sys.exit(1)\n')
                cargo.chmod(0o700)
                rustc=root/'rustc';rustc.write_text("#!/bin/sh\necho 'host: test-host'\n");rustc.chmod(0o700)
                check=gate.rust_tests('fast',root/'logs',dict(os.environ,PATH=str(root)+os.pathsep+os.environ['PATH']))
                self.assertEqual(check.status,'FAIL' if safety else 'FLAKY')
                self.assertEqual(strict_rust_test(name),safety)

    def test_skip_is_not_pass_and_safety_skip_fails(self):
        for name, expected in [('daemon/rev_soak_10min','SKIP'),('daemon/zero_upload','FAIL')]:
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix='fbt-skip-') as directory:
                root=Path(directory)
                script=('import json,sys;from pathlib import Path;'
                        'p=Path(sys.argv[1]);(p/"daemon").mkdir();'
                        '(p/"daemon/case.jsonl").write_text(json.dumps(dict(suite="daemon",final=True,ok=None,status="skip"))+"\\n")')
                check=gate.Check(name,[sys.executable,'-c',script,'{logs}'],case=True)
                gate.run_checks([check],root,dict(os.environ))
                self.assertEqual(check.status,expected)

    def test_fast_gate_always_exercises_zero_upload(self):
        self.assertIn('zero_upload',gate.FAST['daemon'])
        self.assertTrue(strict_check('daemon/zero_upload'))
        self.assertFalse(strict_check('daemon/search_dedupe_and_repair'))


if __name__=='__main__':unittest.main()
