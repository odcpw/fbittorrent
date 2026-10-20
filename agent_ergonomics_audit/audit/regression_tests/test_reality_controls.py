#!/usr/bin/env python3
"""Coverage accounting cannot turn an opt-out or a literal True into proof."""
import ast
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
import fbtlog

class RealityControls(unittest.TestCase):
    def test_skip_has_no_passed_assertion_and_a_failure_cannot_be_skipped_away(self):
        previous_root,previous_suites=fbtlog.LOG_ROOT,fbtlog._suites
        try:
            with tempfile.TemporaryDirectory(prefix='fbt-outcome-') as directory,contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
                fbtlog.LOG_ROOT=Path(directory);fbtlog._suites={}
                with fbtlog.Case('controls','opt_out') as case:case.skip('explicit measurement opt-in absent')
                row=json.loads(case.path.read_text().splitlines()[-1])
                self.assertIsNone(row['ok']);self.assertEqual(row['status'],'skip');self.assertEqual(row['steps'],0)
                summary=json.loads((case.suite.directory/'summary.json').read_text())
                self.assertEqual((summary['passed'],summary['failed'],summary['skipped']),(0,0,1))
                with fbtlog.Case('controls','failed_then_skip') as failed:
                    failed.check(False,'real failure');failed.skip('must not erase failure')
                row=json.loads(failed.path.read_text().splitlines()[-1])
                self.assertIs(row['ok'],False);self.assertEqual(row['status'],'fail')
                self.assertEqual(fbtlog.suite_status('controls'),1)
        finally:fbtlog.LOG_ROOT,fbtlog._suites=previous_root,previous_suites

    def test_no_unconditional_success_receipts_in_product_journeys(self):
        root=Path(__file__).resolve().parents[3];bad=[]
        for directory in [Path(__file__).parent,root/'plugin/tests']:
            for path in directory.rglob('*.py'):
                if 'target' in path.parts:continue
                for node in ast.walk(ast.parse(path.read_bytes())):
                    if isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute) and node.func.attr=='check' and node.args and isinstance(node.args[0],ast.Constant) and node.args[0].value is True:
                        bad.append(f'{path.relative_to(root)}:{node.lineno}')
        self.assertEqual(bad,[],'Use a real predicate or a raising event wait; do not count a note as proof.')

if __name__=='__main__':unittest.main()
