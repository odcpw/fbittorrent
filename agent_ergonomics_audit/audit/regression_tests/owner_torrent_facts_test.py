#!/usr/bin/env python3
"""A matching wrong CLI/API value must still fail the independent oracle."""
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from owner_torrent_facts import assert_fixture_rows

class OwnerFacts(unittest.TestCase):
    def test_each_owner_fact_has_an_independent_negative_control(self):
        with tempfile.TemporaryDirectory(prefix='fbt-owner-facts-') as directory:
            root=Path(directory);payload=root/'payload';payload.write_bytes(b'actual fixture bytes')
            item=SimpleNamespace(info_hash='a'*40,info={'name':'Owned name'},files=[payload])
            row=dict(hash='a'*40,name='Owned name',size=20,save_path=str(root))
            assert_fixture_rows([row],item,root)
            for key,value in dict(hash='b'*40,name='Wrong name',size=21,save_path='/wrong').items():
                with self.subTest(key=key),self.assertRaises(AssertionError):
                    assert_fixture_rows([dict(row,**{key:value})],item,root)
            for rows in ([],[row,row]):
                with self.assertRaises(AssertionError):assert_fixture_rows(rows,item,root)

if __name__=='__main__':unittest.main()
