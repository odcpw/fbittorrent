"""Real stripped-ELF control for avoiding nearest-export misattribution."""
from pathlib import Path
import subprocess
import tempfile
import unittest

from elf_symbols import FunctionRanges


class SymbolControls(unittest.TestCase):
    def test_hidden_code_is_unresolved_after_stripping(self):
        with tempfile.TemporaryDirectory(prefix='fbt-symbol-') as directory:
            root = Path(directory)
            source = root/'sample.c'
            library = root/'sample.so'
            source.write_text('''
__attribute__((noinline)) int exported(int x) { return x + 1; }
__attribute__((visibility("hidden"), noinline)) int hidden(int x) { return x * 3; }
int caller(int x) { return hidden(exported(x)); }
''')
            subprocess.run(['cc', '-g', '-O0', '-shared', '-fPIC', source, '-o', library], check=True)
            raw = subprocess.check_output(['nm', '-S', '--defined-only', library], text=True)
            positions = {fields[3]: (int(fields[0], 16), int(fields[1], 16))
                         for line in raw.splitlines() if len(fields := line.split()) == 4}
            address, size = positions['hidden']
            self.assertEqual(FunctionRanges(library).containing(address+size//2), 'hidden')
            subprocess.run(['strip', '--strip-all', library], check=True)
            bounds = FunctionRanges(library)
            self.assertIsNone(bounds.containing(address+size//2), 'hidden code must not inherit a nearby export')
            address, size = positions['exported']
            self.assertEqual(bounds.containing(address+size//2), 'exported')
            self.assertIsNone(bounds.containing(0))
            self.assertIsNone(bounds.containing(2**63))


if __name__ == '__main__':
    unittest.main()
