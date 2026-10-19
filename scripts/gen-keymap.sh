#!/usr/bin/env bash
# Generate both consumers from the compiled action catalogue. No backend needed.
set -euo pipefail
cd "$(dirname "$0")/.."
binary=${FBT_BINARY:-target/debug/fbittorrent}
exec python3 - "$binary" "${1:-}" <<'PY'
import json
from pathlib import Path
import subprocess
import sys

binary, mode = sys.argv[1:]
if mode not in ('', '--check'):
    raise SystemExit('usage: scripts/gen-keymap.sh [--check]')
keys = json.loads(subprocess.check_output([binary, 'robot-docs', 'keys', '--json', '--surface', 'all']))
markdown = subprocess.check_output([binary, 'robot-docs', 'keys', '--surface', 'all'], text=True)
path = Path('plugin/oliver.torrents/keymap.json')
readme = Path('README.md')
text = readme.read_text()
begin, end = '<!-- keys:begin -->', '<!-- keys:end -->'
if begin not in text:
    text += '\n## Keys\n\n' + begin + '\n' + end + '\n'
before, rest = text.split(begin, 1)
_, after = rest.split(end, 1)
outputs = {path: json.dumps(keys, ensure_ascii=False, indent=2) + '\n',
           readme: before + begin + '\n' + markdown + end + after}
for destination, expected in outputs.items():
    if mode == '--check':
        actual = destination.read_text() if destination.exists() else ''
        if actual != expected:
            import difflib
            sys.stderr.writelines(difflib.unified_diff(actual.splitlines(True), expected.splitlines(True), str(destination), 'generated'))
            raise SystemExit(1)
    else:
        destination.write_text(expected)
    print(('matched ' if mode else 'generated ') + str(destination))
PY
