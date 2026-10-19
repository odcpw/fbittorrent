#!/usr/bin/env bash
# Enforce the 900-line ceiling on explicit directories; --baseline ratchets legacy oversize files (they may only shrink).
set -euo pipefail
exec python3 - "$@" <<'PY'
import argparse
from pathlib import Path
import tempfile
import unittest

LIMIT = 900


def load_baseline(path):
    """Legacy oversize files and the line counts they may not exceed (ratchet): `<lines> <path>` per line."""
    baseline = {}
    if path is None:
        return baseline
    for raw in Path(path).read_text().splitlines():
        raw = raw.strip()
        if not raw or raw.startswith("#"):
            continue
        count, _, name = raw.partition(" ")
        baseline[name.strip()] = int(count)
    return baseline


def check_sizes(directories, emit=print, baseline=None):
    baseline = baseline or {}
    failed = False
    files = set()
    for directory in directories:
        if not directory.is_dir():
            emit(f"lint-sizes: ERROR {directory}: expected a directory")
            failed = True
            continue
        files.update(path for path in directory.rglob("*") if path.is_file())
    for path in sorted(files):
        try:
            with path.open("rb") as source:
                count = sum(1 for _ in source)
        except OSError as error:
            emit(f"lint-sizes: ERROR {path}: {error}")
            failed = True
            continue
        allowed = max(LIMIT, baseline.get(str(path), 0))
        verdict = "ERROR" if count > allowed else "ok"
        note = f" (legacy baseline {baseline[str(path)]}, may only shrink)" if str(path) in baseline and allowed > LIMIT else ""
        emit(f"lint-sizes: {verdict} {path}: {count} lines (limit {allowed}){note}")
        failed |= count > allowed
    emit(f"lint-sizes: {len(files)} files checked; {'FAIL' if failed else 'PASS'}")
    return int(failed)


class SizeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fbt-lint-sizes-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = []

    def check(self, *roots):
        return check_sizes(roots or [self.root], self.output.append)

    def test_exactly_900_lines_pass_in_nested_directory_with_spaces(self):
        nested = self.root / "nested directory"
        nested.mkdir()
        (nested / "boundary.rs").write_bytes(b"line\n" * 900)
        self.assertEqual(self.check(), 0)
        self.assertTrue(any("900 lines" in line for line in self.output))

    def test_901_lines_fail_and_all_roots_are_checked(self):
        (self.root / "large.rs").write_bytes(b"line\n" * 901)
        other = self.root / "other"
        other.mkdir()
        (other / "small.rs").write_bytes(b"line\n")
        self.assertEqual(self.check(self.root, other), 1)
        self.assertTrue(any("ERROR" in line and "901 lines" in line for line in self.output))
        self.assertTrue(any("small.rs" in line for line in self.output))
        self.assertIn("2 files checked", self.output[-1])

    def test_final_line_without_newline_counts(self):
        (self.root / "unterminated.rs").write_bytes(b"line\n" * 900 + b"last")
        self.assertEqual(self.check(), 1)

    def test_baseline_allows_legacy_size_but_not_growth(self):
        legacy = self.root / "legacy.rs"
        legacy.write_bytes(b"line\n" * 1000)
        base = {str(legacy): 1000}
        self.assertEqual(check_sizes([self.root], self.output.append, base), 0)
        legacy.write_bytes(b"line\n" * 1001)
        self.assertEqual(check_sizes([self.root], self.output.append, base), 1)
        (self.root / "new.rs").write_bytes(b"line\n" * 901)
        legacy.write_bytes(b"line\n" * 1000)
        self.assertEqual(check_sizes([self.root], self.output.append, base), 1)

    def test_missing_directory_and_file_argument_fail(self):
        self.assertEqual(self.check(self.root / "missing"), 1)
        file = self.root / "file.rs"
        file.touch()
        self.assertEqual(self.check(file), 1)


parser = argparse.ArgumentParser(description="Fail when a file exceeds 900 physical lines.")
parser.add_argument("--self-test", action="store_true", help="test the ceiling and error handling before checking directories")
parser.add_argument("--baseline", type=Path, help="ratchet file: `<lines> <path>` per line; listed files may not grow past that count, all others obey the limit")
parser.add_argument("directories", nargs="*", type=Path)
args = parser.parse_args()
if not args.directories and not args.self_test:
    parser.error("at least one directory is required")
if args.self_test:
    tests = unittest.defaultTestLoader.loadTestsFromTestCase(SizeTests)
    if not unittest.TextTestRunner(verbosity=2).run(tests).wasSuccessful():
        raise SystemExit(1)
raise SystemExit(check_sizes(args.directories, baseline=load_baseline(args.baseline)) if args.directories else 0)
PY
