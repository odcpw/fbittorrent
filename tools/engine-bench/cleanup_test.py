"""Failure-injection controls for benchmark ownership, not engine acceptance."""
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from run import Run


class CleanupTests(unittest.TestCase):
    def test_setup_exception_and_keyboard_interrupt_reap_owned_children(self):
        for failure in (RuntimeError("injected setup failure"), KeyboardInterrupt()):
            with self.subTest(failure=type(failure).__name__), tempfile.TemporaryDirectory() as output:
                class Interrupted(Run):
                    def tool(self, *_args, **_kwargs):
                        self.spawn([sys.executable, "-c", "import time; time.sleep(60)"])
                        raise failure

                run = Interrupted(SimpleNamespace(output=Path(output), disk_root=Path("/tmp"), mib=1))
                with self.assertRaises(BaseExceptionGroup) as caught:
                    with run.setup("injected"):
                        self.fail("setup should not succeed")
                self.assertIs(caught.exception.exceptions[0], failure)
                self.assertFalse(run.owners.survivors())
                self.assertFalse(run.sandbox.root.exists())
                self.assertTrue(all(child.poll() is not None for child in run.children))
                self.assertFalse((run.output / "teardown.json").exists(), "emergency cleanup must fail qualification")


if __name__ == "__main__":
    unittest.main()
