#!/usr/bin/env python3
"""Stale target and identical-binary negative controls for upgrade qualification."""
import json
from pathlib import Path
import tempfile
import unittest
from upgrade_artifacts import IDS,executable,valid_cache


class UpgradeArtifacts(unittest.TestCase):
    def test_remote_identity_uses_only_the_configured_delivery_target(self):
        with tempfile.TemporaryDirectory(prefix='fbt-upgrade-control-') as value:
            root=Path(value);stale=root/'target/debug/fbittorrent';stale.parent.mkdir(parents=True);stale.write_bytes(b'old');stale.chmod(0o700)
            current=root/'qualified';current.mkdir();output=root/'cargo.json'
            output.write_text(json.dumps(dict(reason='compiler-artifact',target=dict(name='fbittorrent'),executable='/missing-worker/debug/fbittorrent'))+'\n')
            with self.assertRaisesRegex(RuntimeError,'not delivered'):
                executable(output,dict(CARGO_TARGET_DIR=str(current)),root)
            delivered=current/'debug/fbittorrent';delivered.parent.mkdir();delivered.write_bytes(b'new');delivered.chmod(0o700)
            self.assertEqual(executable(output,dict(CARGO_TARGET_DIR=str(current)),root),delivered)
            output.write_text('compiler transport lost the artifact record\n')
            with self.assertRaisesRegex(RuntimeError,'did not identify'):executable(output,dict(CARGO_TARGET_DIR=str(current)),root)

    def test_cache_requires_two_different_verified_builds(self):
        with tempfile.TemporaryDirectory(prefix='fbt-upgrade-control-') as value:
            paths=[Path(value)/tag for tag in IDS]
            for path in paths:path.write_text('identical stale binary')
            digest=lambda p:p.read_text()
            self.assertFalse(valid_cache(dict(ids=IDS,old=digest(paths[0]),new=digest(paths[1])),paths,digest))
            paths[1].write_text('verified new binary')
            saved=dict(ids=IDS,old=digest(paths[0]),new=digest(paths[1]))
            self.assertTrue(valid_cache(saved,paths,digest))
            self.assertFalse(valid_cache(dict(saved,ids={}),paths,digest))
            paths[0].write_text('tampered')
            self.assertFalse(valid_cache(saved,paths,digest))


if __name__=='__main__':unittest.main()
